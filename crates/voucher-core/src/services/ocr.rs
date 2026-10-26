//! OCR 服务：**云端优先（DeepSeek 视觉）+ 本地兜底（Apple Vision）**
//!
//! 配置语义（`AppConfig.ocr.engine`）：
//! - `auto`（默认）：先走云端；云端不可用（无 Key / 网络失败 / 返回异常）才用本地。
//! - `cloud`：只走云端，失败即报错（用户显式选择，不做静默回退）。
//! - `local`：只走本地（离线可用）。
//!
//! 为什么改成云端优先：云端模型能一次给出「文本 + 日期/金额/对方/商家」结构化要素，
//! 比本地正则抽取准；本地保留只为「断网/无 Key 也得能用」。本地引擎换成 Apple Vision
//! 后零模型资产（原 PP-OCRv5 需 21MB 模型 + ONNX Runtime），实测快约 3.8×、要素命中持平。
//!
//! 缓存三层（都以图片内容哈希为键）：
//! - `ocr_cache`：识别文本（含产出引擎名，用于「本地缓存 → 云端升级重试」判定）
//! - `ocr_field_cache`：云端结构化要素（重新识别不再付费调 API）
//! - `ocr_layouts`：本地版面坐标框（几何配对），云端产物没有版面故写库时清掉
//!
//! 锁纪律（审计 B8）：推理（本地数百 ms、云端可达 60s）**绝不持 DB 连接锁**；
//! 查缓存 / 写缓存各取一次短锁。

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::Engine as _;

use crate::db::repos::{
    attachment_repo, enhance_hit_repo, ocr_cache_repo, ocr_field_cache_repo, ocr_layout_repo,
};
use crate::models::{Attachment, OcrLine, OcrStatus};
use crate::utils::config::{
    AppConfig, OCR_ENGINE_CLOUD, OCR_ENGINE_LOCAL, OCR_PROVIDER_DEEPSEEK, SECRET_OCR_API_KEY,
};
use crate::{CoreContext, CoreError, Result};

/// DeepSeek 引擎标识（写 ocr_cache.engine 溯源）
const DEFAULT_DEEPSEEK_ENGINE: &str = "deepseek";

/// 云端请求超时（图片上传 + 推理；DeepSeek 视觉首字较慢，留足余量）
const CLOUD_TIMEOUT_SECS: u64 = 90;
/// 上传前把长边压到该上限（官方限制单边 8192、单图 32MiB；2048 足够看清票据文字，
/// 能省流量与推理时间，且不会因过度缩放丢小字）
const CLOUD_MAX_SIDE: u32 = 2048;
/// 上传 JPEG 质量
const CLOUD_JPEG_QUALITY: u8 = 88;

/// 云端要素抽取提示词：要求输出 JSON（JSON mode 要求提示里出现 "json" 与样例）
const DEEPSEEK_PROMPT: &str = r#"你是票据/凭证信息抽取器。请阅读这张图片，完成两件事：
1. 把整张图上所有可见文字按阅读顺序（先上后下、先左后右）用换行拼成一个字符串放到 text。
2. 抽取结构化要素：date（凭证/交易日期，格式 YYYY-MM-DD）、amount（价税合计/实付金额，单位元，只保留数字与小数点，如 1234.56）、counterparty（对方单位：收款方/销售方/商户名，不含地址电话）、merchant（开票方或商家名称，可与 counterparty 相同）。
规则：看不清、不存在或不确定的字段一律填 null，不要猜测、不要编造；金额不要带 ¥ 或"元"。
只输出一个 json 对象，不要输出任何解释文字，字段固定为：
{"text": "整图文字", "date": "2026-08-19", "amount": "1234.56", "counterparty": "杭州某某科技有限公司", "merchant": "杭州某某科技有限公司"}"#;

/// 识别单个附件；命中缓存直接返回。
pub fn run_for_attachment(ctx: &CoreContext, attachment_id: i64) -> Result<Attachment> {
    let attachment = ctx.db.with_conn(|conn| {
        attachment_repo::get(conn, attachment_id)?
            .ok_or_else(|| CoreError::NotFound(format!("附件 {attachment_id} 不存在")))
    })?;

    if !attachment.is_image() {
        return Err(CoreError::Invalid("仅支持识别图片附件".into()));
    }

    let abs = ctx.paths.attachment_abs_checked(&attachment.file_path)?;
    if !abs.exists() {
        return Err(CoreError::NotFound(format!(
            "附件文件缺失: {}",
            abs.display()
        )));
    }

    let cfg = ctx.config();
    if !cfg.ocr.enabled {
        return Err(CoreError::Invalid("OCR 未启用（设置 → OCR）".into()));
    }

    // v0.6 C 步：优先用增强版（5 步算法后的彩色 JPEG）做 OCR；
    // 增强版不存在（未增强 / 失败）时回退原图。
    // v0.6.5 D-2：使用增强版时记一条 enhance_hits 命中（OCR 来源），给设置页命中率统计。
    let ocr_target = if let Some(ep) = &attachment.enhanced_path {
        let enhanced_abs = ctx.paths.attachment_abs_checked(ep)?;
        if enhanced_abs.exists() {
            if let Err(e) = ctx.db.with_conn(|conn| {
                enhance_hit_repo::record(conn, attachment.id, enhance_hit_repo::HitSource::Ocr)
            }) {
                log::warn!("记录 enhance_hits 失败（不影响 OCR）: {e}");
            }
            enhanced_abs
        } else {
            log::warn!("增强版文件缺失，回退原图: {}", enhanced_abs.display());
            abs.clone()
        }
    } else {
        abs.clone()
    };

    let recognised = resolve_recognition(ctx, &cfg, &attachment.file_hash, &ocr_target)?;

    ctx.db.with_conn(|conn| {
        attachment_repo::set_ocr(conn, attachment.id, OcrStatus::Done, Some(&recognised.text))?;
        // 版面：本地引擎产出行坐标 → 落表供几何配对；
        // 云端没有版面 → 清掉旧版面，否则旧本地坐标会在下次填单时压过云端文本。
        match &recognised.lines {
            Some(lines) => {
                if let Err(e) =
                    ocr_layout_repo::set_layout(conn, attachment.id, recognised.engine, lines)
                {
                    log::warn!("写入 OCR 版面失败（不影响识别结果）: {e}");
                }
            }
            None => {
                if let Err(e) = ocr_layout_repo::delete_layout(conn, attachment.id) {
                    log::warn!("清理旧 OCR 版面失败（不影响识别结果）: {e}");
                }
            }
        }
        attachment_repo::get(conn, attachment.id)?
            .ok_or_else(|| CoreError::Invalid("附件更新后读取失败".into()))
    })
}

/// 并发识别路数：云端是网络等待（单张 1–3s），3 路并行；本地 Vision 走系统框架，
/// 框架内部已并行，多个请求互相争抢反而更慢，故保持串行。
///
/// 导入与批量识别共用同一口径（导入 8 张从 40s 降到 ~15s）。
pub fn import_ocr_workers(ctx: &CoreContext) -> usize {
    if wants_cloud(&ctx.config().ocr.engine) {
        3
    } else {
        1
    }
}

/// 批量识别待处理附件，返回成功数（失败的逐个记为 failed，不中断批次）。
/// 识别成功后自动填单（Manual 模式：只填空字段，不覆盖已录入内容）。
/// `on_progress` 每处理完一张回调 (已完成数, 总数)，供上层做进度反馈（传 `&|_, _| {}` 忽略）。
///
/// 并发：云端请求是网络等待（单张 1–3s），故 3 路并发；本地 Vision 走系统框架，
/// 保持串行（框架内部已并行，多个请求互相争抢反而更慢）。
pub fn run_pending(
    ctx: &CoreContext,
    limit: u32,
    on_progress: &dyn Fn(usize, usize),
) -> Result<usize> {
    let pending = ctx
        .db
        .with_conn(|conn| attachment_repo::list_pending_ocr(conn, limit))?;
    let total = pending.len();
    let workers = import_ocr_workers(ctx);

    let mut done = 0usize;
    let mut ok = 0usize;

    if workers == 1 {
        for a in &pending {
            if finish_one(ctx, a, run_for_attachment(ctx, a.id)) {
                ok += 1;
            }
            done += 1;
            on_progress(done, total);
        }
    } else {
        // 分块并发：块内 3 张并行识别（网络等待为主），块间串行。
        // `on_progress` 可能持有 AppHandle（`&dyn Fn` 非 Sync），故只在主线程按块回调。
        for chunk in pending.chunks(workers) {
            let results: Vec<Result<Attachment>> = std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|a| {
                        let id = a.id;
                        scope.spawn(move || run_for_attachment(ctx, id))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().unwrap_or_else(|_| {
                            Err(CoreError::Invalid("OCR 识别线程异常退出".into()))
                        })
                    })
                    .collect()
            });
            for (a, r) in chunk.iter().zip(results) {
                if finish_one(ctx, a, r) {
                    ok += 1;
                }
                done += 1;
                on_progress(done, total);
            }
        }
    }
    Ok(ok)
}

/// 收尾一张：成功就自动填单，失败就记 failed。返回是否成功。
fn finish_one(ctx: &CoreContext, a: &Attachment, r: Result<Attachment>) -> bool {
    match r {
        Ok(att) => {
            if let Err(e) = crate::services::ocr_fields::apply_for_attachment(
                ctx,
                att.voucher_id,
                att.id,
                crate::services::ocr_fields::FillMode::Manual,
            ) {
                log::warn!("OCR 自动填单失败（凭证 {}）: {e}", att.voucher_id);
            }
            true
        }
        Err(e) => {
            log::warn!("OCR 失败（附件 {}）: {e}", a.id);
            let _ = ctx
                .db
                .with_conn(|conn| attachment_repo::set_ocr(conn, a.id, OcrStatus::Failed, None));
            false
        }
    }
}

// ---------- 缓存 + 引擎选择 ----------

/// 一次识别的产物
struct Recognised {
    text: String,
    /// 版面数据：本地引擎有，云端没有
    lines: Option<Vec<OcrLine>>,
    /// 云端结构化要素（原样 JSON 对象；本地引擎为 None）
    fields: Option<serde_json::Value>,
    /// 产出引擎标识，写 `ocr_cache.engine` 溯源
    engine: &'static str,
    /// 模型名（云端才有意义），随要素一起入 `ocr_field_cache`
    model: String,
}

impl Recognised {
    fn from_cache(engine: String, text: String) -> Self {
        Self {
            text,
            lines: None,
            fields: None,
            engine: Box::leak(engine.into_boxed_str()),
            model: String::new(),
        }
    }
}

/// 取识别结果：缓存命中直接用；命中本地缓存而当前策略含云端 → 尝试一次云端升级。
fn resolve_recognition(
    ctx: &CoreContext,
    cfg: &AppConfig,
    file_hash: &str,
    image: &Path,
) -> Result<Recognised> {
    let cached = ctx
        .db
        .with_conn(|conn| ocr_cache_repo::get(conn, file_hash))?;

    if let Some((engine, text)) = cached {
        // 缓存来自本地、而当前策略希望云端 → 升一次级（进程内每张图只试一次，
        // 断网时不会每张都白等 90s 超时）
        if !is_cloud_engine(&engine) && wants_cloud(&cfg.ocr.engine) && upgrade_allowed(file_hash) {
            match cloud_recognise(ctx, cfg, image) {
                Ok(r) => {
                    store_recognition(ctx, file_hash, &r)?;
                    return Ok(r);
                }
                Err(e) => {
                    log::warn!("云端识别失败，沿用本地缓存结果: {e}");
                    ctx.record_ocr_run(&engine, Some(&e.to_string()));
                }
            }
        }
        return Ok(Recognised::from_cache(engine, text));
    }

    let r = infer_ocr(ctx, cfg, image)?;
    store_recognition(ctx, file_hash, &r)?;
    Ok(r)
}

/// 写回三层缓存（短锁；失败只告警，不影响识别结果本身）
fn store_recognition(ctx: &CoreContext, file_hash: &str, r: &Recognised) -> Result<()> {
    ctx.db
        .with_conn(|conn| ocr_cache_repo::upsert(conn, file_hash, r.engine, &r.text))?;
    if let Some(fields) = &r.fields {
        let json = fields.to_string();
        if let Err(e) = ctx.db.with_conn(|conn| {
            ocr_field_cache_repo::upsert(conn, file_hash, r.engine, &r.model, &json)
        }) {
            log::warn!("写入云端要素缓存失败（不影响识别）: {e}");
        }
    }
    Ok(())
}

/// 进程内「已试过云端升级」的图片哈希集合（避免断网时每张图都等满超时）
fn upgrade_allowed(file_hash: &str) -> bool {
    static TRIED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let set = TRIED.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = set.lock().unwrap_or_else(|p| p.into_inner());
    if guard.len() > 4096 {
        guard.clear();
    }
    guard.insert(file_hash.to_string())
}

/// 引擎配置是否希望使用云端
pub(crate) fn wants_cloud(engine: &str) -> bool {
    engine != OCR_ENGINE_LOCAL
}

/// 引擎名是否属于云端家族（本地只有 `local`）
pub(crate) fn is_cloud_engine(engine: &str) -> bool {
    engine != OCR_ENGINE_LOCAL
}

/// **锁外推理**：按配置选择引擎（auto 含云端失败回退本地）
fn infer_ocr(ctx: &CoreContext, cfg: &AppConfig, image: &Path) -> Result<Recognised> {
    match cfg.ocr.engine.as_str() {
        OCR_ENGINE_LOCAL => local_recognise(ctx, image),
        OCR_ENGINE_CLOUD => {
            let r = cloud_recognise(ctx, cfg, image)?;
            Ok(r)
        }
        // auto（含未知值，normalize 已收口到这里）
        _ => match cloud_recognise(ctx, cfg, image) {
            Ok(r) => Ok(r),
            Err(e) => {
                log::warn!("云端 OCR 不可用，回退本地识别: {e}");
                let r = local_recognise(ctx, image);
                if r.is_ok() {
                    ctx.record_ocr_run("local", Some(&e.to_string()));
                }
                r
            }
        },
    }
}

// ---------- 引擎抽象 ----------

/// OCR 引擎统一接口
trait OcrEngine {
    /// 引擎标识（写 ocr_cache.engine 溯源）
    fn name(&self) -> &'static str;
    /// 模型名（云端才有意义）
    fn model(&self) -> String;
    /// 识别：返回文本（本地引擎另带版面数据）
    fn recognize(&self, ctx: &CoreContext, image_path: &Path) -> Result<Recognised>;
}

/// 本地引擎：Apple Vision（macOS 系统框架）
struct VisionEngine;

impl OcrEngine for VisionEngine {
    fn name(&self) -> &'static str {
        "local"
    }

    fn model(&self) -> String {
        String::new()
    }

    fn recognize(&self, _ctx: &CoreContext, image_path: &Path) -> Result<Recognised> {
        let lines = crate::services::ocr_vision::recognize(image_path)?;
        let text = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Recognised {
            text,
            lines: Some(lines),
            fields: None,
            engine: self.name(),
            model: self.model(),
        })
    }
}

/// 云端引擎：DeepSeek 视觉（OpenAI 兼容 chat/completions）
struct DeepseekEngine {
    model: String,
}

impl OcrEngine for DeepseekEngine {
    fn name(&self) -> &'static str {
        "deepseek"
    }

    fn model(&self) -> String {
        self.model.clone()
    }

    fn recognize(&self, ctx: &CoreContext, image_path: &Path) -> Result<Recognised> {
        deepseek_recognise(ctx, image_path, &self.model)
    }
}

/// 云端引擎：自建通用协议（v0.22 起的 `{image_base64, model}` → `{text}`）
struct CustomCloudEngine {
    model: String,
}

impl OcrEngine for CustomCloudEngine {
    fn name(&self) -> &'static str {
        OCR_ENGINE_CLOUD
    }

    fn model(&self) -> String {
        self.model.clone()
    }

    fn recognize(&self, ctx: &CoreContext, image_path: &Path) -> Result<Recognised> {
        let text = custom_cloud_recognise(ctx, image_path)?;
        ctx.record_ocr_run(self.name(), None);
        Ok(Recognised {
            text,
            lines: None,
            fields: None,
            engine: self.name(),
            model: self.model(),
        })
    }
}

/// 云端识别（按 provider 分派）
fn cloud_recognise(ctx: &CoreContext, cfg: &AppConfig, image_path: &Path) -> Result<Recognised> {
    let engine: Box<dyn OcrEngine> = if cfg.ocr.provider == OCR_PROVIDER_DEEPSEEK {
        Box::new(DeepseekEngine {
            model: cfg.ocr.model.clone(),
        })
    } else {
        Box::new(CustomCloudEngine {
            model: cfg.ocr.model.clone(),
        })
    };
    engine.recognize(ctx, image_path)
}

/// 本地识别（Apple Vision）：行文本按阅读顺序拼接为纯文本
fn local_recognise(ctx: &CoreContext, image_path: &Path) -> Result<Recognised> {
    let engine = VisionEngine;
    let r = engine.recognize(ctx, image_path)?;
    ctx.record_ocr_run(engine.name(), None);
    Ok(r)
}

// ---------- DeepSeek 视觉（OpenAI 兼容） ----------

/// 构造 OpenAI 兼容请求体（纯函数，便于单测校验协议字段）
fn deepseek_body(model: &str, data_url: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": DEEPSEEK_PROMPT},
                {"type": "image_url", "image_url": {"url": data_url, "detail": "original"}},
            ],
        }],
        // JSON 模式：要求提示里含 "json" 字样（见 DEEPSEEK_PROMPT），否则官方会拒
        "response_format": {"type": "json_object"},
        // 关闭思考模式：识别是确定性抽取，思考只会拖长耗时、抬高费用
        "thinking": {"type": "disabled"},
        "max_tokens": 2048,
        "stream": false,
    })
}

fn deepseek_recognise(ctx: &CoreContext, image_path: &Path, model: &str) -> Result<Recognised> {
    let cfg = ctx.config();
    let api_url = cfg.ocr.api_url.trim().to_string();
    if api_url.is_empty() {
        return Err(CoreError::Invalid(
            "云端 OCR API 地址未配置（设置 → OCR）".into(),
        ));
    }
    validate_api_url(&api_url)?;
    let (b64, mime) = encode_image_for_cloud(image_path)?;
    let data_url = format!("data:{mime};base64,{b64}");

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(CLOUD_TIMEOUT_SECS))
        .build()?;
    let mut req = client.post(&api_url).json(&deepseek_body(model, &data_url));
    if let Some(key) = ctx.secrets.get(SECRET_OCR_API_KEY)? {
        if !key.is_empty() {
            req = req.bearer_auth(key);
        } else {
            return Err(CoreError::Invalid(
                "云端 OCR API Key 未设置（设置 → 密钥管理）".into(),
            ));
        }
    } else {
        return Err(CoreError::Invalid(
            "云端 OCR API Key 未设置（设置 → 密钥管理）".into(),
        ));
    }

    let resp: serde_json::Value = req
        .send()?
        .error_for_status()
        .map_err(|e| CoreError::Invalid(format!("云端 OCR 调用失败: {e}")))?
        .json()?;

    let content = resp
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let fields = parse_json_object(content).ok_or_else(|| {
        CoreError::Invalid(format!(
            "云端 OCR 返回内容不是合法 json（可能被截断或模型未按格式输出）: {}",
            truncate_for_log(content)
        ))
    })?;
    let text = json_text(&fields);
    if text.trim().is_empty() {
        return Err(CoreError::Invalid(
            "云端 OCR 返回空文本（整图未识别到文字）".into(),
        ));
    }
    ctx.record_ocr_run(DEFAULT_DEEPSEEK_ENGINE, None);
    Ok(Recognised {
        text,
        lines: None,
        fields: Some(fields),
        engine: DEFAULT_DEEPSEEK_ENGINE,
        model: model.to_string(),
    })
}

/// 从模型输出里取出 JSON 对象：容忍 ```json 围栏与前后解释文字
fn parse_json_object(content: &str) -> Option<serde_json::Value> {
    let trimmed = content.trim();
    let candidate = match (trimmed.find('{'), trimmed.rfind('}')) {
        (Some(start), Some(end)) if end > start => &trimmed[start..=end],
        _ => return None,
    };
    serde_json::from_str::<serde_json::Value>(candidate)
        .ok()
        .filter(|v| v.is_object())
}

/// 结构化要素里的整图文本；缺失时退回「要素拼装」保证 ocr_text 可搜索
fn json_text(fields: &serde_json::Value) -> String {
    if let Some(t) = fields.get("text").and_then(|v| v.as_str()) {
        if !t.trim().is_empty() {
            return t.trim().to_string();
        }
    }
    let mut parts = Vec::new();
    for key in ["date", "amount", "counterparty", "merchant"] {
        if let Some(v) = fields.get(key).and_then(|v| v.as_str()) {
            if !v.trim().is_empty() {
                parts.push(v.trim().to_string());
            }
        }
    }
    parts.join("\n")
}

fn truncate_for_log(s: &str) -> String {
    let mut out: String = s.chars().take(120).collect();
    if s.chars().count() > 120 {
        out.push('…');
    }
    out
}

/// 上传前预处理：长边压到 [`CLOUD_MAX_SIDE`]、转 JPEG（体积与推理时间都显著下降）
fn encode_image_for_cloud(image_path: &Path) -> Result<(String, &'static str)> {
    let img = image::open(image_path)
        .map_err(|e| CoreError::Invalid(format!("读取图片失败 {}: {e}", image_path.display())))?;
    let (w, h) = (img.width(), img.height());
    let img = if w > CLOUD_MAX_SIDE || h > CLOUD_MAX_SIDE {
        img.resize(
            CLOUD_MAX_SIDE,
            CLOUD_MAX_SIDE,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageOutputFormat::Jpeg(CLOUD_JPEG_QUALITY))
        .map_err(|e| CoreError::Invalid(format!("图片编码失败: {e}")))?;
    Ok((
        base64::engine::general_purpose::STANDARD.encode(buf.into_inner()),
        "image/jpeg",
    ))
}

// ---------- 自建通用云端协议（v0.22 遗留，给存量用户） ----------

/// 云端识别：POST {image_base64, model} → {text}（兼容 data.text / result 字段）
fn custom_cloud_recognise(ctx: &CoreContext, image_path: &Path) -> Result<String> {
    let cfg = ctx.config();
    let api_url = cfg.ocr.api_url.trim().to_string();
    if api_url.is_empty() {
        return Err(CoreError::Invalid(
            "云端 OCR API 地址未配置（设置 → OCR）".into(),
        ));
    }
    // 体检 Q-3：强制 https，报错发生在携带 Bearer Key / 发出任何请求之前。
    validate_api_url(&api_url)?;

    let bytes = std::fs::read(image_path)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(CLOUD_TIMEOUT_SECS))
        .build()?;

    let mut req = client.post(&api_url).json(&serde_json::json!({
        "image_base64": b64,
        "model": cfg.ocr.model,
    }));
    if let Some(key) = ctx.secrets.get(SECRET_OCR_API_KEY)? {
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
    }

    let resp: serde_json::Value = req.send()?.error_for_status()?.json()?;
    extract_text(&resp)
        .ok_or_else(|| CoreError::Invalid("OCR 服务返回格式无法解析（缺少 text 字段）".into()))
}

/// 校验云端 OCR API 地址（体检 Q-3）：协议必须 `https`。
///
/// `http` 仅在回环地址（localhost / 127.0.0.1 / ::1）放行——用于本地调试服务；
/// 其它主机用 `http` 会以明文传输 API Key（`Authorization: Bearer`），直接拒绝。
/// 其余协议（ftp 等）一律拒绝。
fn validate_api_url(api_url: &str) -> Result<()> {
    let url = reqwest::Url::parse(api_url).map_err(|e| {
        CoreError::Invalid(format!(
            "云端 OCR API 地址无法解析（需形如 https://host/path）: {e}"
        ))
    })?;
    match url.scheme() {
        "https" => Ok(()),
        "http" => {
            // IPv6 的 host_str() 带方括号（如 "[::1]"），比较前统一剥掉
            let host = url
                .host_str()
                .unwrap_or("")
                .trim_matches(|c| c == '[' || c == ']')
                .to_ascii_lowercase();
            let is_loopback = host == "localhost" || host == "127.0.0.1" || host == "::1";
            if is_loopback {
                Ok(())
            } else {
                Err(CoreError::Invalid(format!(
                    "云端 OCR API 地址必须使用 https（当前为 http://{host}）。\
                     为避免 API Key 明文传输，仅 localhost/127.0.0.1/::1 允许 http"
                )))
            }
        }
        other => Err(CoreError::Invalid(format!(
            "云端 OCR API 地址协议不支持: {other}（需 https）"
        ))),
    }
}

/// 常见返回格式逐层取文本（某条路径不存在时继续尝试下一条，而非提前退出）
fn extract_text(v: &serde_json::Value) -> Option<String> {
    for path in ["text", "data.text", "result", "data.result", "result.text"] {
        if let Some(cur) = path.split('.').try_fold(v, |acc, k| acc.get(k)) {
            if let Some(s) = cur.as_str() {
                if !s.trim().is_empty() {
                    return Some(s.trim().to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_supports_common_shapes() {
        let a = serde_json::json!({"text": "发票 100 元"});
        assert_eq!(extract_text(&a).unwrap(), "发票 100 元");

        let b = serde_json::json!({"data": {"text": "收据"}});
        assert_eq!(extract_text(&b).unwrap(), "收据");

        let c = serde_json::json!({"result": "出租车票"});
        assert_eq!(extract_text(&c).unwrap(), "出租车票");

        let d = serde_json::json!({"foo": 1});
        assert!(extract_text(&d).is_none());
    }

    /// 云端 API 地址：https 放行；http 仅回环地址放行，其余拒绝。
    #[test]
    fn cloud_api_url_requires_https_except_loopback() {
        assert!(validate_api_url("https://ocr.example.com/v1/recognize").is_ok());
        assert!(validate_api_url("http://127.0.0.1:8000/v1").is_ok());
        assert!(validate_api_url("http://localhost:8000/v1").is_ok());
        assert!(validate_api_url("http://[::1]:8000/v1").is_ok());

        let err = validate_api_url("http://ocr.example.com/v1").unwrap_err();
        assert!(err.to_string().contains("https"));

        assert!(validate_api_url("ftp://ocr.example.com/v1").is_err());
        assert!(validate_api_url("not a url").is_err());
    }

    /// 请求体必须是 OpenAI 兼容的 vision 形态（图走 image_url data URL）
    #[test]
    fn deepseek_body_is_openai_vision_shaped() {
        let body = deepseek_body("deepseek-flash", "data:image/jpeg;base64,AAA");
        assert_eq!(body["model"], "deepseek-flash");
        let content = &body["messages"][0]["content"];
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/jpeg;base64,AAA");
        // JSON 模式 + 关闭思考（识别任务不需要思考，省时省钱）
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["thinking"]["type"], "disabled");
        // 官方要求 JSON 模式时提示词里必须出现 "json"
        assert!(DEEPSEEK_PROMPT.contains("json"));
    }

    /// 模型输出可能带 ```json 围栏或前后解释，必须能取出 JSON 对象
    #[test]
    fn parse_json_object_tolerates_fences_and_prose() {
        let v =
            parse_json_object("```json\n{\"text\":\"合计 100 元\",\"date\":null}\n```").unwrap();
        assert_eq!(v["text"], "合计 100 元");
        assert!(v["date"].is_null());

        let v = parse_json_object("这是结果：{\"text\":\"abc\"} 请查收").unwrap();
        assert_eq!(v["text"], "abc");

        assert!(parse_json_object("").is_none());
        assert!(parse_json_object("完全不是 json").is_none());
        assert!(parse_json_object("[1,2,3]").is_none(), "数组不算要素对象");
    }

    /// text 缺失时不能用空文本落缓存（否则搜索无内容），退回要素拼装
    #[test]
    fn json_text_falls_back_to_fields() {
        let v = serde_json::json!({"date": "2026-08-19", "amount": "1234.56", "text": ""});
        assert_eq!(json_text(&v), "2026-08-19\n1234.56");
        let v = serde_json::json!({"text": "整图文字"});
        assert_eq!(json_text(&v), "整图文字");
    }

    #[test]
    fn engine_classification_and_cloud_preference() {
        assert!(wants_cloud("auto"));
        assert!(wants_cloud("cloud"));
        assert!(!wants_cloud("local"));

        assert!(!is_cloud_engine("local"));
        assert!(is_cloud_engine("deepseek"));
        assert!(is_cloud_engine("cloud"));
    }
}
