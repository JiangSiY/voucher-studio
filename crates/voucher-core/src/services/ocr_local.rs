//! 本地 OCR 引擎（G1 Spike B 验证路线：ort + RapidOCR PP-OCRv4）
//!
//! 与 rapidocr_onnxruntime 1.4.4 逐参数对齐（模型文件同源）：
//! - Det  limit_side_len=736(min) / 均值方差 0.5 / thresh=0.3 / box_thresh=0.5 / unclip=1.6 / 2×2 膨胀
//! - Cls  3×48×192 / softmax 后阈值 0.9 判 180° 翻转
//! - Rec  3×48×动态宽（右侧补零到 320 或实际比例宽）/ CTC 解码 / 词表内嵌于 rec 模型元数据
//! - 汇总 rec 置信度 ≥ 0.5（text_score）保留；框按「先上下、同行先左右」排序
//!
//! DB 后处理用「连通域 + 轴对齐外接矩形 + 矩形 unclip 近似」替代
//! cv2.findContours + pyclipper（对水平文本等效，RapidOCR 语义保留）。

use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};
use ort::session::Session;
use ort::value::Tensor;

use crate::{CoreError, Result};

pub const DET_MODEL: &str = "ch_PP-OCRv4_det_infer.onnx";
pub const REC_MODEL: &str = "ch_PP-OCRv4_rec_infer.onnx";
pub const CLS_MODEL: &str = "ch_ppocr_mobile_v2.0_cls_infer.onnx";
pub const MODEL_FILES: [&str; 3] = [DET_MODEL, REC_MODEL, CLS_MODEL];

// ---- RapidOCR config.yaml 对齐参数 ----
const DET_LIMIT_SIDE_LEN: f32 = 736.0;
const DET_THRESH: f32 = 0.3;
const DET_BOX_THRESH: f32 = 0.5;
const DET_UNCLIP_RATIO: f32 = 1.6;
const DET_MIN_SIZE: f32 = 3.0;
const CLS_IMG_H: u32 = 48;
const CLS_IMG_W: u32 = 192;
const CLS_THRESH: f32 = 0.9;
const REC_IMG_H: u32 = 48;
const REC_IMG_W_MAX: u32 = 320;
const TEXT_SCORE: f32 = 0.5;

/// 一行识别结果（v5 起定义于 models 层，供版面分析复用）
pub use crate::models::OcrLine;

pub struct LocalOcr {
    det: Session,
    det_input: String,
    cls: Session,
    cls_input: String,
    rec: Session,
    rec_input: String,
    /// 索引即类别号：0 = blank，1..=n = 字典字符，n+1 = 空格
    chars: Vec<String>,
}

impl LocalOcr {
    /// 模型是否已就位
    pub fn is_installed(models_dir: &Path) -> bool {
        MODEL_FILES.iter().all(|f| models_dir.join(f).is_file())
    }

    /// 从源目录复制缺失模型到运行目录（dev 用项目 assets，发布用 exe 旁/资源目录）
    pub fn ensure_installed(source: &Path, models_dir: &Path) -> Result<bool> {
        if Self::is_installed(models_dir) {
            return Ok(false);
        }
        if !Self::is_installed(source) {
            return Ok(false);
        }
        std::fs::create_dir_all(models_dir)?;
        for f in MODEL_FILES {
            let dst = models_dir.join(f);
            if !dst.is_file() {
                std::fs::copy(source.join(f), &dst)?;
            }
        }
        Ok(true)
    }

    pub fn load(models_dir: &Path) -> Result<Self> {
        for f in MODEL_FILES {
            if !models_dir.join(f).is_file() {
                return Err(CoreError::LocalOcr(format!(
                    "模型缺失: {f}（期望位于 {}）",
                    models_dir.display()
                )));
            }
        }
        let open = |name: &str| -> Result<Session> {
            Session::builder()
                .and_then(|mut b| b.commit_from_file(models_dir.join(name)))
                .map_err(|e| CoreError::LocalOcr(format!("加载 {name} 失败: {e}")))
        };
        let det = open(DET_MODEL)?;
        let cls = open(CLS_MODEL)?;
        let rec = open(REC_MODEL)?;

        let input_name = |s: &Session| -> String {
            s.inputs()
                .first()
                .map(|i| i.name().to_string())
                .unwrap_or_else(|| "x".into())
        };

        // 词表：内嵌于 rec 模型元数据（与 rapidocr get_character_list 同源）
        let character = rec
            .metadata()
            .ok()
            .and_then(|m| m.custom("character"))
            .ok_or_else(|| CoreError::LocalOcr("rec 模型缺少 character 元数据（词表）".into()))?;
        let mut chars: Vec<String> = vec![String::new()]; // 0 = blank
        chars.extend(
            character
                .split('\n')
                .map(|s| s.trim_end_matches('\r').to_string())
                .filter(|s| !s.is_empty()),
        );
        chars.push(" ".into()); // 末位 = 空格（CTCLabelDecode 语义）

        Ok(Self {
            det_input: input_name(&det),
            cls_input: input_name(&cls),
            rec_input: input_name(&rec),
            det,
            cls,
            rec,
            chars,
        })
    }

    pub fn dict_len(&self) -> usize {
        self.chars.len()
    }

    /// 完整识别一张图片：det → (裁剪 → 竖排旋转 → cls) → rec → 按置信度过滤
    pub fn recognize_image(&mut self, img: &DynamicImage) -> Result<Vec<OcrLine>> {
        let rgb = img.to_rgb8();
        let mut boxes = self.detect(&rgb)?;
        sort_reading_order(&mut boxes);

        let mut out = Vec::new();
        for b in boxes {
            let mut crop = crop_xyxy(&rgb, &b);
            // 竖排文本：旋成横向（对齐 get_rotate_crop_image 的 np.rot90）
            if crop.height() as f32 >= 1.5 * crop.width() as f32 {
                crop = image::imageops::rotate270(&crop);
            }
            // 方向分类：180° 翻转纠正
            if self.cls_flip_180(&crop)? {
                crop = image::imageops::rotate180(&crop);
            }
            let (text, score) = self.recognize_line(&crop)?;
            if score >= TEXT_SCORE && !text.trim().is_empty() {
                out.push(OcrLine {
                    text,
                    score,
                    box_xyxy: b,
                });
            }
        }
        Ok(out)
    }

    // ---------- 检测（DBNet） ----------

    fn detect(&mut self, img: &RgbImage) -> Result<Vec<[f32; 4]>> {
        let (ow, oh) = img.dimensions();
        if ow == 0 || oh == 0 {
            return Ok(Vec::new());
        }
        // RapidOCR：limit_type=min, 736（小图放大），边长取整到 32 的倍数
        let min_side = ow.min(oh) as f32;
        let ratio = if min_side < DET_LIMIT_SIDE_LEN {
            DET_LIMIT_SIDE_LEN / min_side
        } else {
            1.0
        };
        let round32 = |v: f32| -> usize { (((v / 32.0).round() as i64).max(1) * 32) as usize };
        let rw = round32(ow as f32 * ratio);
        let rh = round32(oh as f32 * ratio);
        let resized = image::imageops::resize(img, rw as u32, rh as u32, FilterType::Triangle);

        let data = chw_norm_half(&resized);
        let (shape, prob) = run_session(&mut self.det, &self.det_input, [1, 3, rh, rw], data)?;
        let _ = shape; // [1,1,rh,rw]
        Ok(db_postprocess(&prob, rw, rh, ow as usize, oh as usize))
    }

    // ---------- 方向分类 ----------

    fn cls_flip_180(&mut self, crop: &RgbImage) -> Result<bool> {
        let (w, h) = crop.dimensions();
        if w == 0 || h == 0 {
            return Ok(false);
        }
        let plane = (CLS_IMG_H * CLS_IMG_W) as usize;
        let ratio = w as f32 / h as f32;
        let resized_w = (((CLS_IMG_H as f32 * ratio).ceil() as u32).clamp(1, CLS_IMG_W)) as usize;
        let resized =
            image::imageops::resize(crop, resized_w as u32, CLS_IMG_H, FilterType::Triangle);

        let mut data = vec![0f32; 3 * plane];
        for y in 0..CLS_IMG_H as usize {
            for x in 0..resized_w {
                let p = resized.get_pixel(x as u32, y as u32);
                for c in 0..3 {
                    let v = p[c] as f32 / 255.0;
                    data[c * plane + y * CLS_IMG_W as usize + x] = (v - 0.5) / 0.5;
                }
            }
        }
        let (_shape, out) = run_session(
            &mut self.cls,
            &self.cls_input,
            [1, 3, CLS_IMG_H as usize, CLS_IMG_W as usize],
            data,
        )?;
        if out.len() < 2 {
            return Ok(false);
        }
        // softmax 二分类：label 1 = 180°
        let m = out[0].max(out[1]);
        let (e0, e1) = ((out[0] - m).exp(), (out[1] - m).exp());
        Ok(e1 / (e0 + e1) > CLS_THRESH)
    }

    // ---------- 识别（CRNN/SVTR + CTC） ----------

    fn recognize_line(&mut self, crop: &RgbImage) -> Result<(String, f32)> {
        let (w, h) = crop.dimensions();
        if w == 0 || h == 0 {
            return Ok((String::new(), 0.0));
        }
        let ratio = w as f32 / h as f32;
        let max_ratio = (REC_IMG_W_MAX as f32 / REC_IMG_H as f32).max(ratio);
        let img_w = (REC_IMG_H as f32 * max_ratio) as usize;
        let resized_w = ((REC_IMG_H as f32 * ratio).ceil() as usize).clamp(1, img_w);
        let resized =
            image::imageops::resize(crop, resized_w as u32, REC_IMG_H, FilterType::Triangle);

        let plane = REC_IMG_H as usize * img_w;
        let mut data = vec![0f32; 3 * plane];
        for y in 0..REC_IMG_H as usize {
            for x in 0..resized_w {
                let p = resized.get_pixel(x as u32, y as u32);
                for c in 0..3 {
                    let v = p[c] as f32 / 255.0;
                    data[c * plane + y * img_w + x] = (v - 0.5) / 0.5;
                }
            }
        }

        let (shape, out) = run_session(
            &mut self.rec,
            &self.rec_input,
            [1, 3, REC_IMG_H as usize, img_w],
            data,
        )?;
        let t = *shape.get(1).unwrap_or(&0) as usize;
        let c = *shape.get(2).unwrap_or(&0) as usize;
        if t == 0 || c == 0 || out.len() < t * c {
            return Ok((String::new(), 0.0));
        }

        // CTC 贪心解码：去重（连续同类别）→ 去 blank → 映射字符；置信度取入选帧均值
        let mut text = String::new();
        let mut probs: Vec<f32> = Vec::new();
        let mut last = usize::MAX;
        for ti in 0..t {
            let row = &out[ti * c..(ti + 1) * c];
            let (mut best_i, mut best_v) = (0usize, f32::MIN);
            for (i, v) in row.iter().enumerate() {
                if *v > best_v {
                    best_v = *v;
                    best_i = i;
                }
            }
            if best_i != 0 && best_i != last {
                if let Some(ch) = self.chars.get(best_i) {
                    text.push_str(ch);
                    probs.push(best_v);
                }
            }
            last = best_i;
        }
        let score = if probs.is_empty() {
            0.0
        } else {
            probs.iter().sum::<f32>() / probs.len() as f32
        };
        Ok((text, score))
    }
}

// ---------- 共用工具 ----------

/// 运行单输入单输出模型，返回 (shape, 扁平数据)
fn run_session(
    session: &mut Session,
    input_name: &str,
    shape: [usize; 4],
    data: Vec<f32>,
) -> Result<(Vec<i64>, Vec<f32>)> {
    let tensor = Tensor::from_array((shape, data))
        .map_err(|e| CoreError::LocalOcr(format!("构造输入张量失败: {e}")))?;
    let outputs = session
        .run(ort::inputs![input_name => tensor])
        .map_err(|e| CoreError::LocalOcr(format!("推理失败: {e}")))?;
    // 先确认有输出再索引（2026-09-23 审计 B17）：原先直接 `outputs[0]`，
    // 模型异常/无输出时越界 panic，且发生在 spawn_blocking 里只表现为 JoinError。
    if outputs.len() == 0 {
        return Err(CoreError::LocalOcr(
            "模型未返回任何输出（模型文件异常？）".into(),
        ));
    }
    let (shape, data) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| CoreError::LocalOcr(format!("读取输出失败: {e}")))?;
    Ok((shape.to_vec(), data.to_vec()))
}

/// CHW + (x/255 - 0.5)/0.5 归一化
fn chw_norm_half(img: &RgbImage) -> Vec<f32> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let plane = w * h;
    let mut out = vec![0f32; 3 * plane];
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x as u32, y as u32);
            for c in 0..3 {
                out[c * plane + y * w + x] = (p[c] as f32 / 255.0 - 0.5) / 0.5;
            }
        }
    }
    out
}

/// DB 后处理（RapidOCR 语义）：
/// 阈值二值化 → 2×2 膨胀 → 连通域 → 分数/最小尺寸过滤 → unclip 扩边 → 映射回原图
fn db_postprocess(prob: &[f32], w: usize, h: usize, ow: usize, oh: usize) -> Vec<[f32; 4]> {
    // 形状校验（2026-09-23 审计 B17）：模型输出长度与期望的 w×h 不符时（模型被替换、
    // 输入尺寸算错），原先会在下面的 `prob[i]` 直接索引越界 panic——而该 panic 发生在
    // spawn_blocking 线程里，只体现为 JoinError，用户侧看到的是「OCR 一直失败」。
    if prob.len() < w * h {
        log::warn!(
            "检测输出形状异常（prob {} < {w}×{h}），本次按空结果处理",
            prob.len()
        );
        return Vec::new();
    }
    let mut mask = vec![false; w * h];
    for i in 0..w * h {
        mask[i] = prob[i] > DET_THRESH;
    }
    // 2×2 膨胀
    let mut dil = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            dil[i] = mask[i]
                || (x > 0 && mask[i - 1])
                || (y > 0 && mask[i - w])
                || (x > 0 && y > 0 && mask[i - w - 1]);
        }
    }

    let mut visited = vec![false; w * h];
    let mut boxes = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let start = y * w + x;
            if !dil[start] || visited[start] {
                continue;
            }
            let mut stack = vec![start];
            visited[start] = true;
            let (mut minx, mut miny, mut maxx, mut maxy) = (x, y, x, y);
            let (mut score_sum, mut score_n) = (0f32, 0u32);
            while let Some(j) = stack.pop() {
                let (jy, jx) = (j / w, j % w);
                minx = minx.min(jx);
                maxx = maxx.max(jx);
                miny = miny.min(jy);
                maxy = maxy.max(jy);
                if mask[j] {
                    score_sum += prob[j];
                    score_n += 1;
                }
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let (nx, ny) = (jx as i32 + dx, jy as i32 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let k = ny as usize * w + nx as usize;
                        if dil[k] && !visited[k] {
                            visited[k] = true;
                            stack.push(k);
                        }
                    }
                }
            }
            if score_n == 0 {
                continue;
            }
            let (bw, bh) = (maxx - minx + 1, maxy - miny + 1);
            if (bw as f32) < DET_MIN_SIZE || (bh as f32) < DET_MIN_SIZE {
                continue;
            }
            if score_sum / (score_n as f32) < DET_BOX_THRESH {
                continue;
            }
            // unclip：d = 面积×1.6 / 周长（矩形近似 Clipper 偏移）
            let d = DET_UNCLIP_RATIO * (bw * bh) as f32 / (2.0 * (bw + bh) as f32);
            let (ex0, ey0) = (minx as f32 - d, miny as f32 - d);
            let (ex1, ey1) = (maxx as f32 + d, maxy as f32 + d);
            // 概率图坐标 → 原图坐标（四舍五入，对齐 np.round）
            let sx = ow as f32 / w as f32;
            let sy = oh as f32 / h as f32;
            let x0 = ((ex0 * sx).round()).clamp(0.0, ow as f32);
            let y0 = ((ey0 * sy).round()).clamp(0.0, oh as f32);
            let x1 = ((ex1 * sx).round()).clamp(0.0, ow as f32);
            let y1 = ((ey1 * sy).round()).clamp(0.0, oh as f32);
            // unclip 后最小边（min_size + 2）
            if x1 - x0 <= DET_MIN_SIZE + 2.0 || y1 - y0 <= DET_MIN_SIZE + 2.0 {
                continue;
            }
            boxes.push([x0, y0, x1, y1]);
        }
    }
    boxes
}

/// 阅读顺序排序（对齐 RapidOCR sorted_boxes：(y,x) 排序 + 同行 y 差 <10 按左到右）
fn sort_reading_order(boxes: &mut [[f32; 4]]) {
    boxes.sort_by(|a, b| {
        a[1].partial_cmp(&b[1])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal))
    });
    let n = boxes.len();
    for i in 0..n.saturating_sub(1) {
        let mut j = i as i64;
        while j >= 0 {
            let (a, b) = (boxes[j as usize + 1], boxes[j as usize]);
            if (a[1] - b[1]).abs() < 10.0 && a[0] < b[0] {
                boxes.swap(j as usize + 1, j as usize);
                j -= 1;
            } else {
                break;
            }
        }
    }
}

/// 轴对齐裁剪
fn crop_xyxy(img: &RgbImage, b: &[f32; 4]) -> RgbImage {
    let (iw, ih) = img.dimensions();
    let x0 = (b[0].floor().max(0.0) as u32).min(iw.saturating_sub(1));
    let y0 = (b[1].floor().max(0.0) as u32).min(ih.saturating_sub(1));
    let x1 = (b[2].ceil().max(0.0) as u32).min(iw).max(x0 + 1);
    let y1 = (b[3].ceil().max(0.0) as u32).min(ih).max(y0 + 1);
    image::imageops::crop_imm(img, x0, y0, x1 - x0, y1 - y0).to_image()
}

/// 模型来源目录探测（应用启动时自动安装模型用），按优先级：
/// 环境变量 → **打包资源目录**（macOS: `Xxx.app/Contents/Resources`）→
/// 项目 assets（开发期编译期路径）→ exe 旁 `ocr-models`。
///
/// 2026-09-23 审计 A2：原实现只找「exe 旁」，而 macOS 的 .app 把资源放在
/// `Contents/Resources/`（Tauri 的 resources），`exe.parent()` 是 `Contents/MacOS` →
/// 打包版找不到模型、本地 OCR 静默失效。更隐蔽的是编译期仓库路径在**构建机上仍然存在**，
/// 于是「本机冒烟测试全过、发给别人就废」——所以资源目录必须排在开发期路径**之前**。
pub fn default_model_sources(resource_dir: Option<&std::path::Path>) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("VOUCHER_OCR_MODELS") {
        v.push(PathBuf::from(p));
    }
    if let Some(rd) = resource_dir {
        v.push(rd.join("ocr-models"));
    }
    // 开发期：voucher-core 的 ../../assets/ocr-models（即项目根 assets）
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ocr-models");
    if dev.is_dir() {
        v.push(dev);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join("ocr-models"));
        }
    }
    v
}

#[cfg(test)]
mod source_tests {
    use super::*;

    /// 2026-09-23 缺陷锁（审计 A2）：打包资源目录必须**优先于**开发期编译期路径。
    /// 否则在构建机上（仓库还在磁盘里）打包版会一直用仓库里的模型，
    /// 把「发给别人后没有本地 OCR」这个问题一直掩盖到用户手里。
    #[test]
    fn resource_dir_precedes_dev_path() {
        let tmp = std::env::temp_dir().join(format!("vs-ocr-src-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let sources = default_model_sources(Some(&tmp));
        let want_rd = tmp.join("ocr-models");
        let pos_rd = sources
            .iter()
            .position(|p| p == &want_rd)
            .expect("应包含资源目录");
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ocr-models");
        let pos_dev = sources.iter().position(|p| p == &dev);
        if let Some(pd) = pos_dev {
            assert!(
                pos_rd < pd,
                "资源目录（{pos_rd}）必须排在开发期路径（{pd}）之前"
            );
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
