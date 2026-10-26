//! 本地 OCR：Apple Vision（`VNRecognizeTextRequest`，macOS 系统框架）
//!
//! 2026-10-26 换代：由「ort + RapidOCR PP-OCRv5（21MB 模型 + 静态 onnxruntime）」改为
//! macOS 原生 Vision——零模型资产、无需下载、无 ONNX Runtime。
//!
//! 评测依据（102 张真实票据 + 24 张合成票据，同口径 `ocr_layout`/`ocr_fields` 抽取）：
//! - 要素命中与 PP-OCRv5 基本持平（金额 78 vs 79、日期 68 vs 66、对方 31 vs 32）
//! - 单张耗时 247ms（p50 234）vs 950ms（p50 863），约 3.8×
//! - 合成集字符错误率 0.39%（同行合并后）vs 0.00%
//!
//! 两个必须保留的细节（都来自实测，不是经验）：
//! 1. **置信度阈值不能沿用 PP-OCRv5 的 0.5**：Vision 置信度是量化值（中位数恰为 0.50），
//!    0.5 阈值会丢 19.9% 的行、合成集 CER 从 0.39% 升到 1.68% → 取 [`MIN_CONFIDENCE`]。
//! 2. **Vision 按「文本块」切行**，同一行的「标签：」与「值」常被切成两条观测
//!    （PP-OCRv5 按 det 框，二者常合并）。故识别后必须做 [`merge_same_row_lines`]
//!    同行合并归一化，否则纯文本抽取路径（`ocr_fields::extract`）会因「标签与值分家」变差。
//!
//! 坐标契约与 `OcrLine` 一致：**原图像素、左上原点、y 向下**。Vision 的
//! `boundingBox` 是归一化 + **左下原点**，在 [`pixel_box`] 里翻转。

use std::path::Path;

use crate::models::OcrLine;
use crate::Result;

/// Vision 观测置信度下限（实测标定：0.3 与不过滤的抽取结果完全一致）
pub const MIN_CONFIDENCE: f32 = 0.3;

/// 识别语言：简体优先，带繁体与英文（票据中常见英文抬头/数字说明）
pub const LANGUAGES: [&str; 3] = ["zh-Hans", "zh-Hant", "en-US"];

/// 识别整张图片，返回按阅读顺序排列、已做同行合并的文本行（含像素坐标框）。
pub fn recognize(image_path: &Path) -> Result<Vec<OcrLine>> {
    imp::recognize(image_path)
}

/// 启动预热：Vision 的 accurate 档首次调用要编译/加载系统模型（实测冷启动 28.6s，
/// 之后单张 234–325ms）。在后台线程用一张 16×16 白图先跑一遍，把这笔开销挪到启动期，
/// 用户第一次点「识别」就不必等。
///
/// 失败只记日志不打扰用户（Vision 不可用时由调用方回退本地/云端策略判断）；
/// 临时文件用完即删，不落在数据目录里。
pub fn warm_up() {
    #[cfg(target_os = "macos")]
    {
        let dir = std::env::temp_dir();
        let path = dir.join("voucher-studio-ocr-warmup.png");
        let img = image::RgbImage::from_pixel(16, 16, image::Rgb([255u8, 255, 255]));
        if let Err(e) = img.save(&path) {
            log::debug!("OCR 预热图写入失败，跳过预热: {e}");
            return;
        }
        let started = std::time::Instant::now();
        match recognize(&path) {
            Ok(lines) => log::info!(
                "Apple Vision 预热完成（{} 条观测，耗时 {:?}）",
                lines.len(),
                started.elapsed()
            ),
            Err(e) => log::debug!("Apple Vision 预热失败（首次识别时再试）: {e}"),
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// 非 macOS 平台：本地引擎不可用（本项目仅发布 macOS，此分支只为 CI 的 Linux 编译）
#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    use crate::CoreError;

    pub fn recognize(_image_path: &Path) -> Result<Vec<OcrLine>> {
        Err(CoreError::LocalOcr(
            "Apple Vision 本地识别仅支持 macOS".into(),
        ))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::path::Path;

    use super::*;
    use crate::CoreError;

    // ---------- Core Foundation / Objective-C 运行时的最小 FFI 面 ----------

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct CGPoint {
        pub(super) x: f64,
        pub(super) y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct CGSize {
        pub(super) width: f64,
        pub(super) height: f64,
    }

    /// `CGRect`：Vision 的 `boundingBox` 返回值（归一化、左下原点）
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct CGRect {
        pub(super) origin: CGPoint,
        pub(super) size: CGSize,
    }

    #[link(name = "objc", kind = "dylib")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
        /// 变参消息发送入口（按调用点各自 transmute 成真实签名）
        fn objc_msgSend();
        #[cfg(target_arch = "x86_64")]
        fn objc_msgSend_stret();
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: *const c_void,
            buffer: *const u8,
            buf_len: isize,
            is_directory: u8,
        ) -> *mut c_void;
        fn CFRelease(cf: *const c_void);
    }

    // 显式链接 Vision 框架：否则 objc_getClass("VNRecognizeTextRequest") 取不到类
    #[link(name = "Vision", kind = "framework")]
    extern "C" {
        /// Vision 导出的版本常量：只为让链接器加载 Vision 框架（本身不调用）
        #[allow(dead_code)]
        static VNRecognizeTextRequestRevision1: std::ffi::c_long;
    }

    /// 选择器/类名都是编译期常量（纯 ASCII、不含 NUL）。这里先剔掉 NUL 再用
    /// `from_vec_unchecked`：既不会 panic（审计要求生产路径无 unwrap/expect），
    /// 也不需要 `unwrap_or_default`（那会被审计当成「静默容错」）。
    fn cstr(s: &str) -> CString {
        let bytes: Vec<u8> = s.bytes().filter(|b| *b != 0).collect();
        // SAFETY: 上一行已滤掉所有 NUL 字节，构造出的 CString 不含内部 NUL；
        // 传入的是刚从 &str 复制的自有缓冲区，无悬垂引用。
        unsafe { CString::from_vec_unchecked(bytes) }
    }

    /// 诊断开关：设置 `VOUCHER_OCR_TRACE=1` 时把 Vision 管线中间态打到 stderr。
    /// 只在排查「识别返回 0 行」时用，正常路径零开销（每次判断一个环境变量）。
    fn trace_on() -> bool {
        std::env::var_os("VOUCHER_OCR_TRACE").is_some()
    }

    fn trace(what: &str, detail: &str) {
        if trace_on() {
            eprintln!("[vision-trace] {what}: {detail}");
        }
    }

    /// 把任意 ObjC 对象按 `description` 转成字符串（诊断用）
    unsafe fn describe(obj: *mut c_void) -> String {
        if obj.is_null() {
            return "<nil>".into();
        }
        let desc = send_msg0(obj, sel("description"));
        if desc.is_null() {
            return "<no description>".into();
        }
        let utf8 = send_msg0(desc, sel("UTF8String")) as *const c_char;
        if utf8.is_null() {
            return "<no utf8>".into();
        }
        CStr::from_ptr(utf8).to_string_lossy().into_owned()
    }

    unsafe fn cls(name: &str) -> *mut c_void {
        objc_getClass(cstr(name).as_ptr())
    }

    unsafe fn sel(name: &str) -> *mut c_void {
        sel_registerName(cstr(name).as_ptr())
    }

    unsafe fn send_msg0(obj: *mut c_void, s: *mut c_void) -> *mut c_void {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s)
    }

    unsafe fn send_msg1(obj: *mut c_void, s: *mut c_void, a: *mut c_void) -> *mut c_void {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a)
    }

    unsafe fn send_msg2(
        obj: *mut c_void,
        s: *mut c_void,
        a: *mut c_void,
        b: *mut c_void,
    ) -> *mut c_void {
        let f: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
            *mut c_void,
        ) -> *mut c_void = std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a, b)
    }

    unsafe fn send_void1(obj: *mut c_void, s: *mut c_void, a: *mut c_void) {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a)
    }

    unsafe fn send_void_i64(obj: *mut c_void, s: *mut c_void, a: isize) {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, isize) =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a)
    }

    unsafe fn send_void_bool(obj: *mut c_void, s: *mut c_void, a: bool) {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, i8) =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a as i8)
    }

    unsafe fn send_bool2(obj: *mut c_void, s: *mut c_void, a: *mut c_void, b: *mut c_void) -> bool {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i8 =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a, b) != 0
    }

    unsafe fn send_usize0(obj: *mut c_void, s: *mut c_void) -> usize {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> usize =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s)
    }

    unsafe fn send_usize1(obj: *mut c_void, s: *mut c_void, a: usize) -> *mut c_void {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a)
    }

    /// 注意：`VNRecognizedText.confidence` / `VNObservation.confidence` 是 **float（32 位）**，
    /// 不是 double——按 f64 读会把返回值当垃圾（实测置信度全变 0，于是所有行都被 0.3 阈值滤掉）。
    unsafe fn send_f32_0(obj: *mut c_void, s: *mut c_void) -> f32 {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> f32 =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s)
    }

    /// `+[NSArray arrayWithObjects:count:]`：`const id *` + NSUInteger
    unsafe fn send_array_with_objects(
        cls_obj: *mut c_void,
        s: *mut c_void,
        objs: *const *mut c_void,
        count: usize,
    ) -> *mut c_void {
        let f: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *const *mut c_void,
            usize,
        ) -> *mut c_void = std::mem::transmute(objc_msgSend as *const ());
        f(cls_obj, s, objs, count)
    }

    /// `boundingBox` 返回结构体：arm64 走普通消息发送；x86_64 必须走 stret 变体
    unsafe fn send_rect0(obj: *mut c_void, s: *mut c_void) -> CGRect {
        #[cfg(target_arch = "aarch64")]
        {
            let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CGRect =
                std::mem::transmute(objc_msgSend as *const ());
            f(obj, s)
        }
        #[cfg(target_arch = "x86_64")]
        {
            let mut out = CGRect::default();
            let f: unsafe extern "C" fn(*mut CGRect, *mut c_void, *mut c_void) =
                std::mem::transmute(objc_msgSend_stret as *const ());
            f(&mut out, obj, s);
            out
        }
    }

    // ---------- 识别主流程 ----------

    pub fn recognize(image_path: &Path) -> Result<Vec<OcrLine>> {
        let (w, h) = image::image_dimensions(image_path).map_err(|e| {
            CoreError::LocalOcr(format!("读取图片尺寸失败 {}: {e}", image_path.display()))
        })?;

        // accurate（0）是主路径；个别环境（无 ANE 权限的沙箱、CI 无 GPU）会失败或返回空，
        // 此时自动回落 fast（1）——宁可粗一点，也要能出文本。
        match recognize_at_level(image_path, w, h, 0) {
            Ok(lines) if !lines.is_empty() => Ok(lines),
            Ok(_) => recognize_at_level(image_path, w, h, 1),
            Err(e) => {
                log::warn!("Vision accurate 模式失败，回落 fast：{e}");
                recognize_at_level(image_path, w, h, 1)
            }
        }
    }

    fn recognize_at_level(image_path: &Path, w: u32, h: u32, level: isize) -> Result<Vec<OcrLine>> {
        // 自动释放池：Vision 每次识别会产生大量 autoreleased 对象，
        // 批量识别时不建池会一路涨内存。
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let out = run_pipeline(image_path, w, h, level);
            objc_autoreleasePoolPop(pool);
            out
        }
    }

    unsafe fn run_pipeline(
        image_path: &Path,
        w: u32,
        h: u32,
        level: isize,
    ) -> Result<Vec<OcrLine>> {
        use std::os::unix::ffi::OsStrExt;

        let path_bytes = image_path.as_os_str().as_bytes();
        let url = CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            path_bytes.as_ptr(),
            path_bytes.len() as isize,
            0,
        );
        if url.is_null() {
            return Err(CoreError::LocalOcr(format!(
                "无法构造图片 URL: {}",
                image_path.display()
            )));
        }
        if trace_on() {
            trace("url", &describe(url));
        }
        let guard = UrlGuard(url);

        let request = send_msg0(cls("VNRecognizeTextRequest"), sel("alloc"));
        let request = send_msg1(
            request,
            sel("initWithCompletionHandler:"),
            std::ptr::null_mut(),
        );
        send_void_i64(request, sel("setRecognitionLevel:"), level);
        send_void_bool(request, sel("setUsesLanguageCorrection:"), true);

        let lang_objs: Vec<*mut c_void> = LANGUAGES
            .iter()
            .map(|lang| {
                let cs = cstr(lang);
                send_msg1(
                    cls("NSString"),
                    sel("stringWithUTF8String:"),
                    cs.as_ptr() as *mut c_void,
                )
            })
            .collect();
        let lang_arr = send_array_with_objects(
            cls("NSArray"),
            sel("arrayWithObjects:count:"),
            lang_objs.as_ptr(),
            lang_objs.len(),
        );
        send_void1(request, sel("setRecognitionLanguages:"), lang_arr);

        let options = send_msg0(cls("NSDictionary"), sel("dictionary"));
        let handler = send_msg0(cls("VNImageRequestHandler"), sel("alloc"));
        let handler = send_msg2(handler, sel("initWithURL:options:"), url, options);
        if trace_on() {
            trace("handler", &describe(handler));
            trace("request", &describe(request));
        }

        let req_objs = [request];
        let req_arr = send_array_with_objects(
            cls("NSArray"),
            sel("arrayWithObjects:count:"),
            req_objs.as_ptr(),
            req_objs.len(),
        );

        let mut err: *mut c_void = std::ptr::null_mut();
        let ok = send_bool2(
            handler,
            sel("performRequests:error:"),
            req_arr,
            &mut err as *mut *mut c_void as *mut c_void,
        );
        if trace_on() {
            trace("perform ok", &ok.to_string());
            trace("error", &describe(err));
        }
        if !ok {
            return Err(CoreError::LocalOcr(
                "Vision 识别请求执行失败（VNImageRequestHandler.performRequests 返回 NO）".into(),
            ));
        }

        let results = send_msg0(request, sel("results"));
        let count = send_usize0(results, sel("count"));
        if trace_on() {
            trace("results count", &count.to_string());
        }
        let mut lines: Vec<OcrLine> = Vec::with_capacity(count);
        for i in 0..count {
            let obs = send_usize1(results, sel("objectAtIndex:"), i);
            let candidates = send_usize1(obs, sel("topCandidates:"), 1);
            if send_usize0(candidates, sel("count")) == 0 {
                continue;
            }
            let cand = send_usize1(candidates, sel("objectAtIndex:"), 0);
            let ns_text = send_msg0(cand, sel("string"));
            if trace_on() {
                trace("obs", &format!("{i}: {}", describe(ns_text)));
            }
            let utf8 = send_msg0(ns_text, sel("UTF8String")) as *const c_char;
            if utf8.is_null() {
                continue;
            }
            let text = CStr::from_ptr(utf8).to_string_lossy().into_owned();
            if text.trim().is_empty() {
                continue;
            }
            let score = send_f32_0(cand, sel("confidence"));
            if trace_on() {
                trace("obs score", &format!("{i}: {score:.3}"));
            }
            let rect = send_rect0(obs, sel("boundingBox"));
            lines.push(OcrLine {
                text: text.trim().to_string(),
                score,
                box_xyxy: pixel_box(rect, w, h),
            });
        }

        drop(guard);
        Ok(finalize_lines(lines))
    }

    /// Vision 归一化坐标（左下原点）→ 原图像素坐标（左上原点）
    pub(super) fn pixel_box(rect: CGRect, w: u32, h: u32) -> [f32; 4] {
        let x0 = rect.origin.x * w as f64;
        let x1 = (rect.origin.x + rect.size.width) * w as f64;
        let y0 = (1.0 - (rect.origin.y + rect.size.height)) * h as f64;
        let y1 = (1.0 - rect.origin.y) * h as f64;
        [x0 as f32, y0 as f32, x1 as f32, y1 as f32]
    }

    /// `CFURL` 生命周期守卫（显式 `CFRelease`，异常路径也不漏）
    struct UrlGuard(*mut c_void);
    impl Drop for UrlGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }
}

/// 后处理：置信度过滤 → 同行合并 → 阅读顺序
pub(crate) fn finalize_lines(lines: Vec<OcrLine>) -> Vec<OcrLine> {
    let kept: Vec<OcrLine> = lines
        .into_iter()
        .filter(|l| l.score >= MIN_CONFIDENCE)
        .collect();
    let mut merged = merge_same_row_lines(&kept);
    sort_reading_order(&mut merged);
    merged
}

/// 同行合并：把同一水平带上、x 方向相邻的若干观测按 x 拼回一行。
///
/// 这是 Vision 的必需步骤（见模块注释第 2 条）：Vision 常把「标签：」与「值」拆成两条，
/// 合并后与文字的自然行一致，纯文本抽取（`ocr_fields::extract` 逐行找标签）才不会被拆散。
pub(crate) fn merge_same_row_lines(lines: &[OcrLine]) -> Vec<OcrLine> {
    if lines.is_empty() {
        return Vec::new();
    }
    let mut sorted = lines.to_vec();
    sorted.sort_by(|a, b| {
        a.cy()
            .partial_cmp(&b.cy())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut groups: Vec<Vec<OcrLine>> = Vec::new();
    for line in sorted {
        if let Some(group) = groups.last_mut() {
            let group_cy = group.iter().map(|l| l.cy()).sum::<f32>() / group.len() as f32;
            let group_h = group.iter().map(|l| l.height()).fold(0.0f32, f32::max);
            let tol = group_h.max(12.0) * 0.6;
            if (line.cy() - group_cy).abs() < tol {
                group.push(line);
                continue;
            }
        }
        groups.push(vec![line]);
    }

    groups
        .into_iter()
        .map(|mut group| {
            group.sort_by(|a, b| {
                a.x0()
                    .partial_cmp(&b.x0())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut text = String::new();
            let mut x0 = f32::MAX;
            let mut y0 = f32::MAX;
            let mut x1 = f32::MIN;
            let mut y1 = f32::MIN;
            let mut score = f32::MAX;
            for line in &group {
                if text.is_empty() {
                    text = line.text.clone();
                } else {
                    text = join_text(&text, &line.text);
                }
                x0 = x0.min(line.box_xyxy[0]);
                y0 = y0.min(line.box_xyxy[1]);
                x1 = x1.max(line.box_xyxy[2]);
                y1 = y1.max(line.box_xyxy[3]);
                score = score.min(line.score);
            }
            OcrLine {
                text,
                score,
                box_xyxy: [x0, y0, x1, y1],
            }
        })
        .collect()
}

/// 拼接同一行的两段文本：CJK 边界直接相连，拉丁词之间补空格
pub(crate) fn join_text(prev: &str, next: &str) -> String {
    let last = prev.chars().last();
    let first = next.chars().next();
    let space = match (last, first) {
        (Some(a), Some(b)) => !is_cjk(a) && !is_cjk(b) && !a.is_whitespace() && !b.is_whitespace(),
        _ => false,
    };
    if space {
        format!("{prev} {next}")
    } else {
        format!("{prev}{next}")
    }
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3000..=0x30FF        // CJK 标点 + 假名
        | 0x3400..=0x4DBF      // 扩展 A
        | 0x4E00..=0x9FFF      // 基本汉字
        | 0xF900..=0xFAFF      // 兼容汉字
        | 0xFF00..=0xFFEF      // 全角标点
        | 0x20000..=0x2FA1F    // 扩展 B 起
    )
}

/// 阅读顺序：先上后下，同一水平带内先左后右
fn sort_reading_order(lines: &mut [OcrLine]) {
    lines.sort_by(|a, b| {
        let dy = a.cy() - b.cy();
        // 同一行判定：中心 y 相差不到半个行高，按 x 排
        let tol = a.height().max(b.height()).max(10.0) * 0.5;
        if dy.abs() < tol {
            a.x0()
                .partial_cmp(&b.x0())
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            dy.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, score: f32, box_xyxy: [f32; 4]) -> OcrLine {
        OcrLine {
            text: text.into(),
            score,
            box_xyxy,
        }
    }

    /// 标签与值被切成两条观测时必须合并成一行（Vision 的真实行为）
    #[test]
    fn merges_label_and_value_split_into_two_observations() {
        let lines = vec![
            line("凭证编号：", 0.9, [73.0, 98.0, 198.0, 132.0]),
            line("20260819-013", 0.5, [334.0, 102.0, 524.0, 126.0]),
        ];
        let merged = merge_same_row_lines(&lines);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "凭证编号：20260819-013");
        assert_eq!(merged[0].box_xyxy, [73.0, 98.0, 524.0, 132.0]);
        assert_eq!(merged[0].score, 0.5);
    }

    /// 不同行不合并
    #[test]
    fn keeps_separate_rows_separate() {
        let lines = vec![
            line("日期：", 0.9, [70.0, 140.0, 144.0, 168.0]),
            line("金额：", 0.9, [70.0, 178.0, 144.0, 206.0]),
        ];
        assert_eq!(merge_same_row_lines(&lines).len(), 2);
    }

    #[test]
    fn join_text_inserts_space_only_between_latin_words() {
        assert_eq!(join_text("合计", "1234.56元"), "合计1234.56元");
        assert_eq!(join_text("1234.56", "元"), "1234.56元");
        assert_eq!(join_text("Voucher", "Studio"), "Voucher Studio");
    }

    /// 置信度 0.3 以下的观测被丢弃（0.5 会把中位数整行丢掉，见模块注释）
    #[test]
    fn filters_low_confidence_lines_at_03() {
        let lines = vec![
            line("毛利率", 0.3, [0.0, 0.0, 10.0, 10.0]),
            line("噪声", 0.29, [0.0, 40.0, 10.0, 50.0]),
        ];
        let out = finalize_lines(lines);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "毛利率");
    }

    #[test]
    fn sorts_reading_order_top_down_then_left_right() {
        let mut lines = vec![
            line("下右", 0.9, [300.0, 200.0, 400.0, 230.0]),
            line("上", 0.9, [100.0, 50.0, 200.0, 80.0]),
            line("下左", 0.9, [50.0, 200.0, 150.0, 230.0]),
        ];
        sort_reading_order(&mut lines);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["上", "下左", "下右"]);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    /// 端到端：真实 Vision 识别（沙箱环境可能只能跑 fast 模式，故只断言「能出文本」）。
    /// 需要一张图时用仓库内的测试夹具：
    /// `cargo test --release -p voucher-core --test ocr_flow` 里另有覆盖；
    /// 这里只验证像素坐标换算的边界（不依赖模型）。
    #[test]
    fn vision_box_to_pixel_flips_y_axis() {
        // 左下角 0.1 高、左侧 0.25 宽的区域 → 原图上部
        let rect = imp::CGRect {
            origin: imp::CGPoint { x: 0.0, y: 0.9 },
            size: imp::CGSize {
                width: 0.25,
                height: 0.1,
            },
        };
        let b = imp::pixel_box(rect, 800, 560);
        assert_eq!(b, [0.0, 0.0, 200.0, 56.0]);
    }
}
