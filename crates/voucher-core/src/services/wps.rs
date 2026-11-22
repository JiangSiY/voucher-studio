//! WPS 台账同步的本地骨架（v0.28 W1-T2）
//!
//! 本模块目前做四件事：
//! 1. 同步表运行期自检（缺表/版本不符 → `WPS_E_SCHEMA`）；
//! 2. `wps_state` / 运行记录 / 链接表的最小读写骨架；
//! 3. 传输层 mock 边界与错误分类（400006 / 429 / 超时）；
//! 4. `WpsClient` 的类型化 JSON 包装与 MCP 会话 mock。
//!
//! **尚未实现真实 HTTP/MCP 传输**，也不包含字段映射与合并逻辑——这些在 W1 后续
//! 子任务按方案 §4/§6 落地。类型化方法只封装 W0 已实测的动作与 JSON 入参，
//! 返回 `serde_json::Value`，不臆造台账字段结构；真实返回结构由后续 W1 映射层解析。

use std::collections::HashSet;
#[cfg(test)]
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::db::Database;
use crate::{CoreContext, CoreError, Result};

/// 同步表集合；与 `db::schema::SCHEMA_WPS` 保持一致。
pub const WPS_TABLES: [&str; 13] = [
    "wps_links",
    "wps_tombstones",
    "wps_scans",
    "wps_conflicts",
    "wps_schema",
    "wps_master",
    "wps_sync_runs",
    "wps_run_changes",
    "wps_run_created",
    "wps_idempotency_keys",
    "wps_record_index",
    "wps_write_intents",
    "wps_state",
];

/// 运行记录保留条数（S0-11）；超出后按 id 删除最旧记录。
pub const WPS_SYNC_RUN_KEEP: i64 = 500;

/// `wps_state` 的运行期版本标记；不参与 SQLite `user_version`。
pub const WPS_SCHEMA_VERSION: &str = "wps-1";

/// 已由 W0 实测的 MCP 动作名（不新增未验证动作）。
pub const ACTION_GET_SCHEMA: &str = "dbsheet.get_schema";
pub const ACTION_LIST_RECORDS: &str = "dbsheet.list_records";
pub const ACTION_CREATE_RECORDS: &str = "dbsheet.create_records";
pub const ACTION_UPDATE_RECORDS: &str = "dbsheet.update_records";

/// 只读重试默认参数（v0.28 定稿）：3 次尝试、1s 起、2x、30s 上限、full jitter。
pub const READ_MAX_ATTEMPTS: usize = 3;
pub const READ_INITIAL_BACKOFF_MS: u64 = 1_000;
pub const READ_BACKOFF_MULTIPLIER: u32 = 2;
pub const READ_MAX_BACKOFF_MS: u64 = 30_000;
/// 写入默认 0 次自动重试（总尝试次数 1）；仅受控测试证明幂等后才允许 1 次重试。
pub const WRITE_MAX_ATTEMPTS: usize = 1;

/// 传输模式契约；默认 mock，http 仅在 S0 显式门禁打开后可用。
pub const TRANSPORT_MOCK: &str = "mock";
pub const TRANSPORT_HTTP: &str = "http";

/// 传输模式；`Http` 仅能在 S0 显式门禁通过后由配置启用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WpsTransportMode {
    #[default]
    Mock,
    Http,
}

impl WpsTransportMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mock => TRANSPORT_MOCK,
            Self::Http => TRANSPORT_HTTP,
        }
    }

    /// 不认识的配置值返回 `None`；调用方决定是否拒绝启动。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            TRANSPORT_MOCK => Some(Self::Mock),
            TRANSPORT_HTTP => Some(Self::Http),
            _ => None,
        }
    }
}

/// 同步客户端配置契约；默认 mock、写入不重试。
///
/// 真实 HTTP 传输只能在 S0 安全评审显式批准后开启；仓库当前没有配置载体，
/// 该结构先作为启动校验与测试契约，接线见 `(待补充：配置载体)`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WpsConfig {
    pub transport: WpsTransportMode,
    pub s0_approved: bool,
    pub write_max_attempts: usize,
    pub write_retry_enabled: bool,
}

impl Default for WpsConfig {
    fn default() -> Self {
        Self {
            transport: WpsTransportMode::Mock,
            s0_approved: false,
            write_max_attempts: WRITE_MAX_ATTEMPTS,
            write_retry_enabled: false,
        }
    }
}

impl WpsConfig {
    /// 启动校验：http 必须 S0 批准；写入默认且当前只允许 0 次重试。
    pub fn validate(&self) -> std::result::Result<(), WpsError> {
        if self.transport == WpsTransportMode::Http && !self.s0_approved {
            return Err(WpsError::Protocol(
                "WPS_E_CONFIG: transport=http 需要 S0 显式批准（s0_approved=true）".into(),
            ));
        }
        if self.write_max_attempts == 0 {
            return Err(WpsError::Protocol(
                "WPS_E_CONFIG: write_max_attempts 不能为 0".into(),
            ));
        }
        if self.write_retry_enabled {
            return Err(WpsError::Protocol(
                "WPS_E_CONFIG: 写入重试尚未启用；默认 0 次".into(),
            ));
        }
        if self.write_max_attempts != 1 {
            return Err(WpsError::Protocol(
                "WPS_E_CONFIG: 写入重试关闭时 write_max_attempts 必须为 1".into(),
            ));
        }
        Ok(())
    }
}

/// 创建单条记录时必需的键值对；缺 key 在类型层无法构造。
#[derive(Debug, Clone, Copy)]
pub struct KeyedRecord<'a> {
    pub record: &'a Value,
    pub idempotency_key: &'a str,
}

impl<'a> KeyedRecord<'a> {
    pub fn new(record: &'a Value, idempotency_key: &'a str) -> Self {
        Self {
            record,
            idempotency_key,
        }
    }
}

/// 逐条写入的本地结果；仅本地状态，不代表 WPS 服务端保证。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStatus {
    Succeeded,
    Unknown,
    Rejected,
    NotAttempted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    pub idempotency_key: String,
    pub status: WriteStatus,
    pub remote_record_id: Option<String>,
    pub error: Option<WpsError>,
}

/// 同步层重试策略；读写分离，写入默认不重试。
#[derive(Debug, Clone)]
pub struct WpsRetryPolicy {
    pub read_attempts: usize,
    pub initial_backoff_ms: u64,
    pub multiplier: u32,
    pub max_backoff_ms: u64,
    pub full_jitter: bool,
    pub write_attempts: usize,
}

impl Default for WpsRetryPolicy {
    fn default() -> Self {
        Self {
            read_attempts: READ_MAX_ATTEMPTS,
            initial_backoff_ms: READ_INITIAL_BACKOFF_MS,
            multiplier: READ_BACKOFF_MULTIPLIER,
            max_backoff_ms: READ_MAX_BACKOFF_MS,
            full_jitter: true,
            write_attempts: WRITE_MAX_ATTEMPTS,
        }
    }
}

impl WpsRetryPolicy {
    /// 第 `attempt` 次失败后的等待时间；`attempt` 从 1 开始。
    fn backoff(&self, attempt: usize) -> Duration {
        let exp = self
            .initial_backoff_ms
            .saturating_mul(u64::from(
                self.multiplier
                    .saturating_pow(attempt.saturating_sub(1) as u32),
            ))
            .min(self.max_backoff_ms);
        let ms = if self.full_jitter && exp > 0 {
            pseudo_jitter(exp)
        } else {
            exp
        };
        Duration::from_millis(ms)
    }
}

/// 无第三方依赖的 full jitter：取当前纳秒的低位，落在 0..=max。
fn pseudo_jitter(max_ms: u64) -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    if max_ms == 0 {
        0
    } else {
        nanos % (max_ms + 1)
    }
}

/// 同步层错误。稳定错误码用于日志/界面提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WpsError {
    /// 同步表缺失/版本不符；调用方应提示重启或恢复备份。
    Schema(String),
    /// WPS 400006：授权失效/需要重新授权。
    AuthExpired,
    /// 凭据不可用/缺失/过期；请求前本地拒绝，不回退文件后端。
    Credentials(String),
    /// WPS 429：限流，调用方退避后重试。
    RateLimited,
    /// 请求超时；与 429 一样属于可重试。
    Timeout,
    /// 远端返回的其他业务错误。
    Remote(String),
    /// 分页响应缺少可解析的 records 数组；只读拉取专用错误码。
    PullMalformed(String),
    /// MCP 会话过期（HTTP 404）；只读可重初始化一次，写入不自动重放。
    SessionExpired,
    /// 写入缺少客户端幂等键；本地拒绝，不发起请求。
    IdempotencyMissing,
    /// 记录数与幂等键数不一致；本地拒绝，不发起请求。
    IdempotencyCountMismatch { records: usize, keys: usize },
    /// 本地 mock/协议层错误（例如 mock 队列为空、会话未初始化）。
    Protocol(String),
}

impl WpsError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Schema(_) => "WPS_E_SCHEMA",
            Self::AuthExpired => "WPS_E_AUTH",
            Self::Credentials(_) => "WPS_E_CREDENTIALS",
            Self::RateLimited => "WPS_E_RATE",
            Self::Timeout => "WPS_E_TIMEOUT",
            Self::Remote(_) => "WPS_E_REMOTE",
            Self::PullMalformed(_) => "WPS_E_PULL_MALFORMED",
            Self::SessionExpired => "WPS_E_SESSION",
            Self::IdempotencyMissing => "WPS_E_IDEMPOTENCY_MISSING",
            Self::IdempotencyCountMismatch { .. } => "WPS_E_IDEMPOTENCY_COUNT_MISMATCH",
            Self::Protocol(_) => "WPS_E_PROTOCOL",
        }
    }

    /// 从 MCP/HTTP JSON 错误体识别错误码。
    ///
    /// W0 已确认 400006 与 429 会出现在 WPS 的 JSON 里；这里只认这两个稳定码。
    /// 远端自由文本可能含 token、电话、税号或金额，一律丢弃，不进入日志、
    /// 运行记录、界面或返回错误；未知错误只保留本地固定文案。
    pub fn from_mcp_error(value: &Value) -> Self {
        let code = value
            .get("code")
            .and_then(Value::as_i64)
            .or_else(|| value.pointer("/error/code").and_then(Value::as_i64));
        match code {
            Some(400006) => Self::AuthExpired,
            Some(429) => Self::RateLimited,
            _ => Self::Remote("远端返回未分类错误".into()),
        }
    }

    pub fn from_http_status(status: u16) -> Self {
        match status {
            401 | 403 => Self::AuthExpired,
            404 => Self::SessionExpired,
            429 => Self::RateLimited,
            _ => Self::Remote(format!("HTTP {status}")),
        }
    }

    /// 回环 HTTP 的 I/O 错误只保留安全分类，不拼接 OS/URL 文本。
    pub fn from_io(err: &std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Self::Timeout,
            _ => Self::Remote("HTTP connect failed".into()),
        }
    }

    /// 只读是否可重试：429/超时/5xx/初始连接失败；400006 等鉴权错误不可重试。
    pub fn is_retryable_read(&self) -> bool {
        match self {
            Self::RateLimited | Self::Timeout => true,
            Self::Remote(msg) => msg.starts_with("HTTP 5") || msg.contains("connect"),
            _ => false,
        }
    }
}

impl std::fmt::Display for WpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema(msg) => write!(f, "{}: {msg}", self.code()),
            Self::AuthExpired => write!(f, "{}: WPS 授权已失效，请重新授权", self.code()),
            Self::Credentials(msg) => write!(f, "{}: {msg}", self.code()),
            Self::RateLimited => write!(f, "{}: WPS 限流，请稍后重试", self.code()),
            Self::Timeout => write!(f, "{}: WPS 请求超时", self.code()),
            Self::Remote(msg) => write!(f, "{}: {msg}", self.code()),
            Self::PullMalformed(msg) => write!(f, "{}: {msg}", self.code()),
            Self::SessionExpired => write!(f, "{}: MCP 会话已过期", self.code()),
            Self::IdempotencyMissing => {
                write!(f, "{}: 写入缺少 idempotency_key，已本地拒绝", self.code())
            }
            Self::IdempotencyCountMismatch { records, keys } => write!(
                f,
                "{}: 记录数 {records} 与幂等键数 {keys} 不一致，已本地拒绝",
                self.code()
            ),
            Self::Protocol(msg) => write!(f, "{}: {msg}", self.code()),
        }
    }
}

impl std::error::Error for WpsError {}

/// 凭据提供方：请求前由 `WpsClient` 取 token，token 只进入传输层认证参数，
/// 不进入 `tools/call` 的 arguments、URL、日志或备份。
pub trait WpsCredentialProvider: Send + Sync {
    fn access_token(&self) -> std::result::Result<String, WpsError>;
}

/// 传输层边界：负责 MCP 会话初始化与一次 `tools/call`，不解释台账字段。
///
/// `auth` 是 `Authorization: Bearer` 的 token 值；真实 HTTP 实现用它设置请求头，
/// 本仓库当前只有 Mock/开发伪传输。token 不得拼进 `args` 或 URL。
pub trait WpsTransport: Send + Sync {
    /// 初始化 MCP 会话并返回 session id（对应 `initialize` + 响应头）。
    fn initialize(&self, auth: Option<&str>) -> std::result::Result<String, WpsError>;

    /// 带 session id 执行一次 `tools/call`；`auth` 只用于请求认证。
    fn call(
        &self,
        session_id: &str,
        action: &str,
        args: &Value,
        auth: Option<&str>,
    ) -> std::result::Result<Value, WpsError>;
}

/// 极薄的客户端：持有 file_id 与会话，提供类型化 JSON 包装。
///
/// 返回 `Value` 而不是领域结构：W1 尚未定义台账字段结构，先用 JSON 透传，
/// 避免把未评审的字段映射固化进公共 API。
pub struct WpsClient<T: WpsTransport> {
    transport: T,
    file_id: String,
    session: Mutex<Option<String>>,
    retry: WpsRetryPolicy,
    allowed_files: Option<HashSet<String>>,
    credential: Option<Arc<dyn WpsCredentialProvider>>,
}

impl<T: WpsTransport> WpsClient<T> {
    pub fn new(transport: T, file_id: impl Into<String>) -> Self {
        Self::with_policy(transport, file_id, WpsRetryPolicy::default())
    }

    /// 测试或受控场景可注入更短/更长的退避；不改变公共类型化方法的签名。
    pub fn with_policy(transport: T, file_id: impl Into<String>, retry: WpsRetryPolicy) -> Self {
        Self {
            transport,
            file_id: file_id.into(),
            session: Mutex::new(None),
            retry,
            allowed_files: None,
            credential: None,
        }
    }

    /// 注入凭据提供方；生产 HTTP 应使用 WPS 专用钥匙串提供方。
    pub fn with_credentials(mut self, provider: Arc<dyn WpsCredentialProvider>) -> Self {
        self.credential = Some(provider);
        self
    }

    /// 使用 WPS 专用 keyring-only 入口；本地 Mock/伪服务使用合成 provider，
    /// 不在测试/开发路径读取真实 token。
    pub fn with_wps_credentials(self) -> Self {
        self.with_credentials(Arc::new(
            crate::services::wps_credentials::WpsCredentialStore::new(),
        ))
    }

    fn auth_token(&self) -> std::result::Result<Option<String>, WpsError> {
        let Some(provider) = &self.credential else {
            return Ok(None);
        };
        let token = provider.access_token()?;
        if token.trim().is_empty() {
            return Err(WpsError::Credentials("WPS token 为空，拒绝请求".into()));
        }
        Ok(Some(token))
    }

    /// 绑定目标 file_id 白名单；S0-06 使用，未列入的 file_id 在发请求前拒绝。
    pub fn with_allowed_files(
        mut self,
        files: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.allowed_files = Some(files.into_iter().map(Into::into).collect());
        self
    }

    fn ensure_file_allowed(&self) -> std::result::Result<(), WpsError> {
        if let Some(allowed) = &self.allowed_files {
            if !allowed.contains(&self.file_id) {
                return Err(WpsError::Protocol(format!(
                    "WPS_E_FILE_NOT_ALLOWED: 目标文件 {0} 不在允许清单",
                    self.file_id
                )));
            }
        }
        Ok(())
    }

    /// 按配置构造；默认 mock，http 必须 S0 批准。
    pub fn with_config(
        transport: T,
        file_id: impl Into<String>,
        config: &WpsConfig,
    ) -> std::result::Result<Self, WpsError> {
        config.validate()?;
        let policy = WpsRetryPolicy {
            write_attempts: config.write_max_attempts,
            ..WpsRetryPolicy::default()
        };
        Ok(Self::with_policy(transport, file_id, policy))
    }

    /// 初始化会话；重复调用会覆盖旧 session（真实实现应先释放旧会话）。
    pub fn initialize(&self) -> std::result::Result<String, WpsError> {
        self.ensure_file_allowed()?;
        let auth = self.auth_token()?;
        let session_id = self.transport.initialize(auth.as_deref())?;
        *self
            .session
            .lock()
            .map_err(|_| WpsError::Protocol("WPS 会话锁中毒".into()))? = Some(session_id.clone());
        Ok(session_id)
    }

    pub fn session_id(&self) -> Option<String> {
        self.session.lock().ok().and_then(|guard| guard.clone())
    }

    /// 测试专用：让同 crate 的 B3 门用例检查底层 mock 的调用记录。
    #[cfg(test)]
    pub(crate) fn transport(&self) -> &T {
        &self.transport
    }

    /// 原始 `tools/call`；未初始化时返回 `WPS_E_PROTOCOL`。
    pub fn call(&self, action: &str, args: &Value) -> std::result::Result<Value, WpsError> {
        self.ensure_file_allowed()?;
        let guard = self
            .session
            .lock()
            .map_err(|_| WpsError::Protocol("WPS 会话锁中毒".into()))?;
        let session_id = guard
            .as_deref()
            .ok_or_else(|| WpsError::Protocol("MCP 会话未初始化，请先 initialize()".into()))?;
        let auth = self.auth_token()?;
        self.transport
            .call(session_id, action, args, auth.as_deref())
    }

    /// 只读请求：429/5xx/超时按 `WpsRetryPolicy` 重试；400006 不重试；
    /// 404 会重初始化一次后继续，避免会话过期被当成业务错误。
    fn call_read(&self, action: &str, args: &Value) -> std::result::Result<Value, WpsError> {
        let mut attempts_left = self.retry.read_attempts.max(1);
        let mut attempt = 0usize;
        let mut reinitialized = false;
        let mut last = None;
        while attempts_left > 0 {
            match self.call(action, args) {
                Ok(value) => return Ok(value),
                Err(WpsError::SessionExpired) if !reinitialized => {
                    self.initialize()?;
                    reinitialized = true;
                    continue;
                }
                Err(err) if err.is_retryable_read() && attempts_left > 1 => {
                    attempt += 1;
                    std::thread::sleep(self.retry.backoff(attempt));
                    last = Some(err);
                    attempts_left -= 1;
                }
                Err(err) => return Err(err),
            }
        }
        Err(last.unwrap_or_else(|| WpsError::Protocol("只读重试未执行".into())))
    }

    pub fn get_schema(&self, sheet_id: i64) -> std::result::Result<Value, WpsError> {
        self.call_read(
            ACTION_GET_SCHEMA,
            &json!({ "file_id": self.file_id, "sheet_id": sheet_id }),
        )
    }

    pub fn list_records(
        &self,
        sheet_id: i64,
        view_id: Option<&str>,
        offset: usize,
        page_size: usize,
    ) -> std::result::Result<Value, WpsError> {
        let mut args = json!({
            "file_id": self.file_id,
            "sheet_id": sheet_id,
            "offset": offset,
            "page_size": page_size,
        });
        if let Some(view_id) = view_id {
            args["view_id"] = Value::String(view_id.to_string());
        }
        self.call_read(ACTION_LIST_RECORDS, &args)
    }

    /// 逐条创建：每条记录必须携带独立幂等键；返回逐条结果。
    ///
    /// 本方法不发送 WPS 原生批量接口；每次 `tools/call` 只含一条记录。
    ///
    /// 缺少 key 的调用无法编译：
    ///
    /// ```compile_fail
    /// use voucher_core::services::wps::{WpsClient, WpsError, WpsTransport};
    /// use serde_json::Value;
    /// struct T;
    /// impl WpsTransport for T {
    ///     fn initialize(&self, _: Option<&str>) -> Result<String, WpsError> { Ok("s".into()) }
    ///     fn call(&self, _: &str, _: &str, _: &Value, _: Option<&str>) -> Result<Value, WpsError> {
    ///         Ok(Value::Null)
    ///     }
    /// }
    /// let client = WpsClient::new(T, "file-1");
    /// client.create_records(1, &[serde_json::json!({"fields": {}})]);
    /// ```
    pub fn create_records(
        &self,
        sheet_id: i64,
        records: &[KeyedRecord<'_>],
    ) -> Vec<std::result::Result<Value, WpsError>> {
        records
            .iter()
            .map(|record| self.create_record(sheet_id, record.record, record.idempotency_key))
            .collect()
    }

    /// 创建单条记录；`idempotency_key` 放在 `tools/call` 的 arguments 顶层。
    /// 写入不自动重试；超时/断连原样返回，由上层决定是否人工确认。
    pub fn create_record(
        &self,
        sheet_id: i64,
        record: &Value,
        idempotency_key: &str,
    ) -> std::result::Result<Value, WpsError> {
        let args = build_create_args(
            std::slice::from_ref(record),
            &[idempotency_key],
            &self.file_id,
            sheet_id,
        )?;
        self.call(ACTION_CREATE_RECORDS, &args)
    }

    /// 更新不强制幂等键；调用方可选传入。
    pub fn update_records(
        &self,
        sheet_id: i64,
        records: &[Value],
    ) -> std::result::Result<Value, WpsError> {
        self.update_records_with_key(sheet_id, records, None)
    }

    pub fn update_records_with_key(
        &self,
        sheet_id: i64,
        records: &[Value],
        idempotency_key: Option<&str>,
    ) -> std::result::Result<Value, WpsError> {
        let mut args = json!({
            "file_id": self.file_id,
            "sheet_id": sheet_id,
            "records": records,
        });
        if let Some(key) = idempotency_key.filter(|k| !k.trim().is_empty()) {
            args["idempotency_key"] = Value::String(key.to_string());
        }
        self.call(ACTION_UPDATE_RECORDS, &args)
    }
}

/// 构造单条/多条创建参数；记录数与键数不一致时本地拒绝，0 次请求。
///
/// 公共写方法只接收 `KeyedRecord`，不会产生长度不一致；这个函数保留计数校验，
/// 作为内部防御与测试契约，防止未来有人用平行数组绕过每条记录必须有 key 的约束。
pub(crate) fn build_create_args(
    records: &[Value],
    keys: &[&str],
    file_id: &str,
    sheet_id: i64,
) -> std::result::Result<Value, WpsError> {
    if records.len() != keys.len() {
        return Err(WpsError::IdempotencyCountMismatch {
            records: records.len(),
            keys: keys.len(),
        });
    }
    if records.is_empty() {
        return Err(WpsError::Protocol("创建记录为空".into()));
    }
    if keys.iter().any(|key| key.trim().is_empty()) {
        return Err(WpsError::IdempotencyMissing);
    }
    // 顶层只能承载一个 key；多记录调用由 `create_records` 逐条拆开。
    if records.len() != 1 {
        return Err(WpsError::Protocol(
            "WPS_E_PROTOCOL: 创建路径只允许逐条发送，单次调用必须 1 条记录".into(),
        ));
    }
    Ok(json!({
        "file_id": file_id,
        "sheet_id": sheet_id,
        "records": records,
        "idempotency_key": keys[0],
    }))
}

/// 从 W0 已观测到的响应形状中提取记录 id；未知形状返回 `None`，由上层标记 unknown。
pub(crate) fn extract_remote_record_id(value: &Value) -> Option<String> {
    for pointer in [
        "/records/0/id",
        "/data/records/0/id",
        "/record/id",
        "/data/id",
        "/id",
    ] {
        if let Some(id) = value.pointer(pointer).and_then(Value::as_str) {
            if !id.trim().is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// 解析 MCP 响应：支持纯 JSON 与 SSE `data:` 行（`content[0].text` 由调用方再解析）。
pub fn parse_mcp_payload(text: &str) -> std::result::Result<Value, WpsError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(WpsError::Protocol("MCP 响应为空".into()));
    }
    let payload = if trimmed.starts_with('{') || trimmed.starts_with('[') {
        trimmed.to_string()
    } else {
        let data: Vec<&str> = trimmed
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .collect();
        if data.is_empty() {
            return Err(WpsError::Protocol(format!(
                "MCP 响应既不是 JSON 也没有 data 行: {trimmed}"
            )));
        }
        data.join("\n")
    };
    serde_json::from_str(&payload)
        .map_err(|err| WpsError::Protocol(format!("MCP 响应 JSON 解析失败: {err}")))
}

/// 测试用假传输：按队列返回预设响应/错误，并记录 session/动作。
#[cfg(test)]
pub(crate) struct MockTransport {
    session_id: String,
    responses: Mutex<VecDeque<std::result::Result<Value, WpsError>>>,
    calls: Mutex<Vec<MockCall>>,
    initialize_calls: Mutex<usize>,
    initialize_auths: Mutex<Vec<Option<String>>>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct MockCall {
    pub(crate) session_id: String,
    pub(crate) action: String,
    pub(crate) args: Value,
    pub(crate) auth: Option<String>,
}

#[cfg(test)]
impl MockTransport {
    pub(crate) fn new(
        session_id: impl Into<String>,
        responses: Vec<std::result::Result<Value, WpsError>>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            responses: Mutex::new(responses.into()),
            calls: Mutex::new(Vec::new()),
            initialize_calls: Mutex::new(0),
            initialize_auths: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn initialize_calls(&self) -> usize {
        *self
            .initialize_calls
            .lock()
            .expect("mock transport mutex poisoned")
    }

    pub(crate) fn initialize_auths(&self) -> Vec<Option<String>> {
        self.initialize_auths
            .lock()
            .expect("mock transport mutex poisoned")
            .clone()
    }

    pub(crate) fn calls(&self) -> Vec<MockCall> {
        self.calls
            .lock()
            .expect("mock transport mutex poisoned")
            .clone()
    }
}

#[cfg(test)]
impl WpsTransport for MockTransport {
    fn initialize(&self, auth: Option<&str>) -> std::result::Result<String, WpsError> {
        *self
            .initialize_calls
            .lock()
            .expect("mock transport mutex poisoned") += 1;
        self.initialize_auths
            .lock()
            .expect("mock transport mutex poisoned")
            .push(auth.map(str::to_string));
        Ok(self.session_id.clone())
    }

    fn call(
        &self,
        session_id: &str,
        action: &str,
        args: &Value,
        auth: Option<&str>,
    ) -> std::result::Result<Value, WpsError> {
        self.calls
            .lock()
            .expect("mock transport mutex poisoned")
            .push(MockCall {
                session_id: session_id.to_string(),
                action: action.to_string(),
                args: args.clone(),
                auth: auth.map(str::to_string),
            });
        self.responses
            .lock()
            .expect("mock transport mutex poisoned")
            .pop_front()
            .unwrap_or_else(|| Err(WpsError::Protocol("mock 队列已空".into())))
    }
}

/// 幂等键本地记录（客户端追溯/去重，不代表 WPS 服务端保证）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyKeyRecord {
    pub idempotency_key: String,
    pub run_id: i64,
    pub record_type: String,
    pub local_status: String,
    pub remote_record_id: Option<String>,
    pub created_at: String,
}

/// `wps_*` 表的本地读写骨架。
pub struct WpsStore<'a> {
    db: &'a Database,
}

impl<'a> WpsStore<'a> {
    pub fn new(ctx: &'a CoreContext) -> Self {
        Self { db: &ctx.db }
    }

    pub fn from_db(db: &'a Database) -> Self {
        Self { db }
    }

    /// 供 `wps_index` 等跨模块方法复用底层连接池；字段 `db` 保持私有。
    pub(crate) fn database(&self) -> &Database {
        self.db
    }

    /// 运行期自检：13 张表齐全且 `wps_state.schema_version='wps-1'`。
    pub fn ensure_schema(&self) -> Result<()> {
        self.database().with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
            let existing: HashSet<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<_, _>>()?;
            let missing: Vec<&str> = WPS_TABLES
                .iter()
                .copied()
                .filter(|name| !existing.contains(*name))
                .collect();
            if !missing.is_empty() {
                return Err(CoreError::Invalid(format!(
                    "WPS_E_SCHEMA: 缺少同步表 {}",
                    missing.join(", ")
                )));
            }

            let version: Option<String> = conn
                .query_row(
                    "SELECT value FROM wps_state WHERE key = 'schema_version'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            match version.as_deref() {
                Some(WPS_SCHEMA_VERSION) => Ok(()),
                Some(other) => Err(CoreError::Invalid(format!(
                    "WPS_E_SCHEMA: 同步表版本不符（期望 {WPS_SCHEMA_VERSION}，实际 {other}）"
                ))),
                None => Err(CoreError::Invalid(format!(
                    "WPS_E_SCHEMA: wps_state 缺少 schema_version={WPS_SCHEMA_VERSION}"
                ))),
            }
        })
    }

    pub fn get_state(&self, key: &str) -> Result<Option<String>> {
        self.db.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT value FROM wps_state WHERE key = ?1",
                    params![key],
                    |row| row.get(0),
                )
                .optional()?)
        })
    }

    pub fn set_state(&self, key: &str, value: &str) -> Result<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO wps_state(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }

    /// 开一条运行记录，返回 run_id。
    pub fn start_run(&self, mode: &str, role: &str) -> Result<i64> {
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO wps_sync_runs(started_at, mode, role, status)
                 VALUES(datetime('now'), ?1, ?2, 'running')",
                params![mode, role],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// 收口运行记录；`error` 为空时状态应为 `ok`。
    #[allow(clippy::too_many_arguments)]
    pub fn finish_run(
        &self,
        run_id: i64,
        status: &str,
        scanned: i64,
        created: i64,
        updated: i64,
        skipped: i64,
        conflicts: i64,
        errors: i64,
        duration_ms: i64,
        error: Option<&str>,
    ) -> Result<()> {
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE wps_sync_runs
                 SET finished_at = datetime('now'), status = ?2, scanned = ?3,
                     created = ?4, updated = ?5, skipped = ?6, conflicts = ?7,
                     errors = ?8, duration_ms = ?9, error = ?10
                 WHERE id = ?1",
                params![
                    run_id,
                    status,
                    scanned,
                    created,
                    updated,
                    skipped,
                    conflicts,
                    errors,
                    duration_ms,
                    error
                ],
            )?;
            conn.execute(
                "DELETE FROM wps_sync_runs
                  WHERE id NOT IN (
                      SELECT id FROM wps_sync_runs ORDER BY id DESC LIMIT ?1
                  )",
                params![WPS_SYNC_RUN_KEEP],
            )?;
            Ok(())
        })
    }

    /// 创建记录前登记幂等键；`local_status` 初始为 `pending`。
    pub fn insert_idempotency_key(
        &self,
        idempotency_key: &str,
        run_id: i64,
        record_type: &str,
    ) -> Result<()> {
        if idempotency_key.trim().is_empty() {
            return Err(CoreError::Invalid(
                "WPS_E_IDEMPOTENCY_MISSING: 幂等键为空".into(),
            ));
        }
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO wps_idempotency_keys
                    (idempotency_key, run_id, record_type, local_status, created_at)
                 VALUES(?1, ?2, ?3, 'pending', datetime('now'))",
                params![idempotency_key, run_id, record_type],
            )?;
            Ok(())
        })
    }

    /// 回填幂等键状态；成功时同时写 `remote_record_id`。
    pub fn mark_idempotency_key(
        &self,
        idempotency_key: &str,
        local_status: &str,
        remote_record_id: Option<&str>,
    ) -> Result<()> {
        if !matches!(local_status, "pending" | "succeeded" | "unknown") {
            return Err(CoreError::Invalid(format!(
                "非法幂等状态 {local_status}；只允许 pending/succeeded/unknown"
            )));
        }
        self.db.with_conn(|conn| {
            let changed = conn.execute(
                "UPDATE wps_idempotency_keys
                 SET local_status = ?2, remote_record_id = COALESCE(?3, remote_record_id)
                 WHERE idempotency_key = ?1",
                params![idempotency_key, local_status, remote_record_id],
            )?;
            if changed == 0 {
                return Err(CoreError::Invalid(format!(
                    "幂等键不存在: {idempotency_key}"
                )));
            }
            Ok(())
        })
    }

    /// 读取幂等键状态；不存在返回 `None`。
    pub fn idempotency_key(&self, idempotency_key: &str) -> Result<Option<IdempotencyKeyRecord>> {
        self.db.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT idempotency_key, run_id, record_type, local_status,
                            remote_record_id, created_at
                     FROM wps_idempotency_keys WHERE idempotency_key = ?1",
                    params![idempotency_key],
                    |row| {
                        Ok(IdempotencyKeyRecord {
                            idempotency_key: row.get(0)?,
                            run_id: row.get(1)?,
                            record_type: row.get(2)?,
                            local_status: row.get(3)?,
                            remote_record_id: row.get(4)?,
                            created_at: row.get(5)?,
                        })
                    },
                )
                .optional()?)
        })
    }

    /// 读取一条关联的本地状态（W2/恢复展示用；不修改任何数据）。
    pub fn link_state(&self, link_id: i64) -> Result<Option<String>> {
        self.database().with_conn(|conn| {
            conn.query_row(
                "SELECT state FROM wps_links WHERE id = ?1",
                params![link_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn count_links(&self, state: &str) -> Result<i64> {
        self.db.with_conn(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM wps_links WHERE state = ?1",
                params![state],
                |row| row.get(0),
            )?)
        })
    }
}

/// 逐条写入会话：客户端 + 本地幂等键状态 + run_id。
///
/// 每条记录独立一次 `tools/call`；任一条失败立即停止，后续记录标记
/// `NotAttempted`，已成功记录不回滚。WPS 原生批量创建不用于写入路径。
pub struct WpsWriteSession<'a, T: WpsTransport> {
    client: &'a WpsClient<T>,
    store: &'a WpsStore<'a>,
    run_id: i64,
}

impl<'a, T: WpsTransport> WpsWriteSession<'a, T> {
    pub fn new(client: &'a WpsClient<T>, store: &'a WpsStore<'a>, run_id: i64) -> Self {
        Self {
            client,
            store,
            run_id,
        }
    }

    /// 逐条创建；返回与输入等长的结果，`NotAttempted` 表示失败后未发起。
    pub fn create_records(
        &self,
        sheet_id: i64,
        record_type: &str,
        records: &[KeyedRecord<'_>],
    ) -> Vec<WriteOutcome> {
        // 先做整体校验：任何空 key/重复 key 都在发请求前拒绝。
        let mut seen = HashSet::new();
        let invalid = records.iter().any(|record| {
            let key = record.idempotency_key.trim();
            key.is_empty() || !seen.insert(key.to_string())
        });
        if invalid {
            return records
                .iter()
                .map(|record| WriteOutcome {
                    idempotency_key: record.idempotency_key.to_string(),
                    status: WriteStatus::Rejected,
                    remote_record_id: None,
                    error: Some(if record.idempotency_key.trim().is_empty() {
                        WpsError::IdempotencyMissing
                    } else {
                        WpsError::Protocol("WPS_E_IDEMPOTENCY_DUPLICATE: 同一批次幂等键重复".into())
                    }),
                })
                .collect();
        }

        let mut outcomes = Vec::with_capacity(records.len());
        for (index, record) in records.iter().enumerate() {
            match self.store.idempotency_key(record.idempotency_key) {
                Ok(Some(row)) if row.local_status == "succeeded" => {
                    outcomes.push(WriteOutcome {
                        idempotency_key: record.idempotency_key.to_string(),
                        status: WriteStatus::Succeeded,
                        remote_record_id: row.remote_record_id,
                        error: None,
                    });
                    continue;
                }
                Ok(Some(row)) => {
                    outcomes.push(WriteOutcome {
                        idempotency_key: record.idempotency_key.to_string(),
                        status: WriteStatus::Rejected,
                        remote_record_id: row.remote_record_id,
                        error: Some(WpsError::Protocol(format!(
                            "WPS_E_IDEMPOTENCY_STATE: 幂等键状态为 {}，拒绝自动重试",
                            row.local_status
                        ))),
                    });
                    push_not_attempted(&mut outcomes, &records[index + 1..]);
                    return outcomes;
                }
                Ok(None) => {}
                Err(err) => {
                    outcomes.push(WriteOutcome {
                        idempotency_key: record.idempotency_key.to_string(),
                        status: WriteStatus::Rejected,
                        remote_record_id: None,
                        error: Some(WpsError::Protocol(format!(
                            "WPS_E_IDEMPOTENCY_STATE: 读取本地幂等键失败: {err}"
                        ))),
                    });
                    push_not_attempted(&mut outcomes, &records[index + 1..]);
                    return outcomes;
                }
            }

            if let Err(err) =
                self.store
                    .insert_idempotency_key(record.idempotency_key, self.run_id, record_type)
            {
                outcomes.push(WriteOutcome {
                    idempotency_key: record.idempotency_key.to_string(),
                    status: WriteStatus::Rejected,
                    remote_record_id: None,
                    error: Some(WpsError::Protocol(format!(
                        "WPS_E_IDEMPOTENCY_STATE: 写入本地幂等键失败: {err}"
                    ))),
                });
                push_not_attempted(&mut outcomes, &records[index + 1..]);
                return outcomes;
            }

            match self
                .client
                .create_record(sheet_id, record.record, record.idempotency_key)
            {
                Ok(value) => match extract_remote_record_id(&value) {
                    Some(remote_id) => {
                        let mark = self.store.mark_idempotency_key(
                            record.idempotency_key,
                            "succeeded",
                            Some(&remote_id),
                        );
                        outcomes.push(WriteOutcome {
                            idempotency_key: record.idempotency_key.to_string(),
                            status: if mark.is_ok() {
                                WriteStatus::Succeeded
                            } else {
                                WriteStatus::Unknown
                            },
                            remote_record_id: Some(remote_id),
                            error: mark.err().map(|err| {
                                WpsError::Protocol(format!(
                                    "WPS_E_IDEMPOTENCY_STATE: 回填成功状态失败: {err}"
                                ))
                            }),
                        });
                    }
                    None => {
                        let _ = self.store.mark_idempotency_key(
                            record.idempotency_key,
                            "unknown",
                            None,
                        );
                        outcomes.push(WriteOutcome {
                            idempotency_key: record.idempotency_key.to_string(),
                            status: WriteStatus::Unknown,
                            remote_record_id: None,
                            error: Some(WpsError::Protocol(
                                "WPS_E_REMOTE_ID: 创建响应缺少记录 id，无法确认成功".into(),
                            )),
                        });
                    }
                },
                Err(err) => {
                    let _ =
                        self.store
                            .mark_idempotency_key(record.idempotency_key, "unknown", None);
                    outcomes.push(WriteOutcome {
                        idempotency_key: record.idempotency_key.to_string(),
                        status: WriteStatus::Unknown,
                        remote_record_id: None,
                        error: Some(err),
                    });
                }
            }

            if outcomes
                .last()
                .is_some_and(|outcome| outcome.status != WriteStatus::Succeeded)
            {
                push_not_attempted(&mut outcomes, &records[index + 1..]);
                return outcomes;
            }
        }
        outcomes
    }
}

fn push_not_attempted(outcomes: &mut Vec<WriteOutcome>, records: &[KeyedRecord<'_>]) {
    outcomes.extend(records.iter().map(|record| WriteOutcome {
        idempotency_key: record.idempotency_key.to_string(),
        status: WriteStatus::NotAttempted,
        remote_record_id: None,
        error: None,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use rusqlite::Connection;

    fn db_with_schema() -> Database {
        let conn = Connection::open_in_memory().expect("open memory db");
        schema::init(&conn).expect("init schema");
        Database::new(conn)
    }

    #[test]
    fn ensure_schema_ok_after_init() {
        let db = db_with_schema();
        WpsStore::from_db(&db).ensure_schema().expect("schema ok");
    }

    #[test]
    fn ensure_schema_missing_table_returns_wps_e_schema() {
        let db = db_with_schema();
        db.with_conn(|conn| {
            conn.execute_batch("DROP TABLE wps_links")?;
            Ok(())
        })
        .expect("drop table");
        let err = WpsStore::from_db(&db)
            .ensure_schema()
            .expect_err("missing table must fail");
        assert!(
            err.to_string().contains("WPS_E_SCHEMA"),
            "unexpected error: {err}"
        );
        assert!(err.to_string().contains("wps_links"));
    }

    #[test]
    fn ensure_schema_version_mismatch_returns_wps_e_schema() {
        let db = db_with_schema();
        WpsStore::from_db(&db)
            .set_state("schema_version", "wps-0")
            .expect("set version");
        let err = WpsStore::from_db(&db)
            .ensure_schema()
            .expect_err("version mismatch must fail");
        assert!(err.to_string().contains("WPS_E_SCHEMA"));
    }

    #[test]
    fn state_roundtrip_and_run_lifecycle() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        assert_eq!(store.get_state("writer").expect("get"), None);
        store.set_state("writer", "desktop").expect("set");
        assert_eq!(
            store.get_state("writer").expect("get"),
            Some("desktop".into())
        );

        let run_id = store.start_run("manual", "desktop").expect("start run");
        store
            .finish_run(run_id, "ok", 10, 1, 2, 3, 0, 0, 1234, None)
            .expect("finish run");
    }

    #[test]
    fn uninitialized_call_returns_protocol_error() {
        let mock = MockTransport::new("sess-1", vec![]);
        let client = WpsClient::new(mock, "file-1");
        let err = client
            .call(ACTION_LIST_RECORDS, &json!({}))
            .expect_err("must require session");
        assert_eq!(err.code(), "WPS_E_PROTOCOL");
        assert!(err.to_string().contains("未初始化"));
        assert!(client.transport.calls().is_empty());
    }

    #[test]
    fn mock_transport_maps_400006_429_and_timeout() {
        let mock = MockTransport::new(
            "sess-1",
            vec![
                Err(WpsError::from_mcp_error(&json!({"code": 400006}))),
                Err(WpsError::from_http_status(429)),
                Err(WpsError::Timeout),
            ],
        );
        let client = WpsClient::new(mock, "file-1");
        assert_eq!(client.initialize().expect("initialize"), "sess-1");
        assert_eq!(client.session_id().as_deref(), Some("sess-1"));

        assert_eq!(
            client
                .call(ACTION_LIST_RECORDS, &json!({}))
                .expect_err("400006"),
            WpsError::AuthExpired
        );
        assert_eq!(
            client
                .call(ACTION_CREATE_RECORDS, &json!({}))
                .expect_err("429"),
            WpsError::RateLimited
        );
        assert_eq!(
            client
                .call(ACTION_UPDATE_RECORDS, &json!({}))
                .expect_err("timeout"),
            WpsError::Timeout
        );
        assert_eq!(WpsError::AuthExpired.code(), "WPS_E_AUTH");
        assert_eq!(WpsError::RateLimited.code(), "WPS_E_RATE");
        assert_eq!(WpsError::Timeout.code(), "WPS_E_TIMEOUT");

        let calls = client.transport.calls();
        assert_eq!(calls.len(), 3);
        assert!(calls.iter().all(|call| call.session_id == "sess-1"));
    }

    #[test]
    fn typed_methods_wrap_actions_and_args() {
        let mock = MockTransport::new(
            "sess-2",
            vec![
                Ok(json!({"sheets": []})),
                Ok(json!({"records": []})),
                Ok(json!({"records": [{"id": "r1"}]})),
                Ok(json!({"records": [{"id": "r1"}]})),
            ],
        );
        let client = WpsClient::new(mock, "file-abc");
        client.initialize().expect("initialize");

        client.get_schema(3).expect("get schema");
        client
            .list_records(3, Some("E"), 100, 50)
            .expect("list records");
        let created = json!({"fields": {"a": 1}});
        client
            .create_records(3, &[KeyedRecord::new(&created, "key-1")])
            .into_iter()
            .next()
            .expect("one result")
            .expect("create");
        client
            .update_records(3, &[json!({"id": "r1", "fields": {"a": 2}})])
            .expect("update");

        let calls = client.transport.calls();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].action, ACTION_GET_SCHEMA);
        assert_eq!(calls[0].args["file_id"], "file-abc");
        assert_eq!(calls[0].args["sheet_id"], 3);
        assert_eq!(calls[1].action, ACTION_LIST_RECORDS);
        assert_eq!(calls[1].args["view_id"], "E");
        assert_eq!(calls[1].args["offset"], 100);
        assert_eq!(calls[1].args["page_size"], 50);
        assert_eq!(calls[2].action, ACTION_CREATE_RECORDS);
        assert_eq!(calls[2].args["records"][0]["fields"]["a"], 1);
        assert_eq!(calls[2].args["idempotency_key"], "key-1");
        assert_eq!(calls[3].action, ACTION_UPDATE_RECORDS);
        assert_eq!(calls[3].args["records"][0]["id"], "r1");
        assert!(calls.iter().all(|call| call.session_id == "sess-2"));
    }

    fn fast_policy() -> WpsRetryPolicy {
        WpsRetryPolicy {
            read_attempts: 3,
            initial_backoff_ms: 1,
            multiplier: 2,
            max_backoff_ms: 5,
            full_jitter: false,
            write_attempts: 1,
        }
    }

    fn keyed<'a>(record: &'a Value, key: &'a str) -> KeyedRecord<'a> {
        KeyedRecord::new(record, key)
    }

    #[test]
    fn create_without_key_rejects_locally_without_request() {
        let mock = MockTransport::new("sess-1", vec![Ok(json!({"records": []}))]);
        let client = WpsClient::new(mock, "file-1");
        client.initialize().expect("initialize");
        let record = json!({"fields": {}});
        let err = client
            .create_record(3, &record, "   ")
            .expect_err("missing key");
        assert_eq!(err, WpsError::IdempotencyMissing);
        assert_eq!(err.code(), "WPS_E_IDEMPOTENCY_MISSING");
        assert!(client.transport.calls().is_empty());
    }

    #[test]
    fn count_mismatch_rejects_locally_without_request() {
        let mock = MockTransport::new("sess-1", vec![Ok(json!({"records": []}))]);
        let client = WpsClient::new(mock, "file-1");
        let records = vec![json!({"a": 1}), json!({"b": 2})];
        let err = build_create_args(&records, &["k1"], "file-1", 3).expect_err("count mismatch");
        assert_eq!(
            err,
            WpsError::IdempotencyCountMismatch {
                records: 2,
                keys: 1
            }
        );
        assert_eq!(err.code(), "WPS_E_IDEMPOTENCY_COUNT_MISMATCH");
        assert!(client.transport.calls().is_empty());
    }

    #[test]
    fn write_timeout_is_not_retried() {
        let mock = MockTransport::new("sess-1", vec![Err(WpsError::Timeout)]);
        let client = WpsClient::with_policy(mock, "file-1", fast_policy());
        client.initialize().expect("initialize");
        let record = json!({"fields": {}});
        let err = client
            .create_record(3, &record, "key-1")
            .expect_err("timeout");
        assert_eq!(err, WpsError::Timeout);
        assert_eq!(client.transport.calls().len(), 1);
    }

    #[test]
    fn read_404_reinitializes_session_once() {
        let mock = MockTransport::new(
            "sess-1",
            vec![
                Err(WpsError::from_http_status(404)),
                Ok(json!({"sheets": []})),
            ],
        );
        let client = WpsClient::with_policy(mock, "file-1", fast_policy());
        client.initialize().expect("initialize");
        client.get_schema(3).expect("reinitialize then read");
        assert_eq!(client.transport.calls().len(), 2);
        assert_eq!(client.transport.initialize_calls(), 2);
    }

    #[test]
    fn config_contract_blocks_http_without_s0_and_write_retry() {
        let config = WpsConfig::default();
        assert_eq!(config.transport, WpsTransportMode::Mock);
        assert!(!config.s0_approved);
        assert_eq!(config.write_max_attempts, 1);
        assert!(!config.write_retry_enabled);
        config.validate().expect("default mock config");

        let http = WpsConfig {
            transport: WpsTransportMode::Http,
            ..WpsConfig::default()
        };
        assert!(http.validate().is_err());

        let approved = WpsConfig {
            transport: WpsTransportMode::Http,
            s0_approved: true,
            ..WpsConfig::default()
        };
        approved.validate().expect("approved http config");

        let retry = WpsConfig {
            write_retry_enabled: true,
            write_max_attempts: 2,
            ..WpsConfig::default()
        };
        assert!(retry.validate().is_err());

        let too_many = WpsConfig {
            write_max_attempts: 2,
            ..WpsConfig::default()
        };
        assert!(too_many.validate().is_err());
    }

    #[test]
    fn read_retries_429_and_5xx_three_attempts() {
        let mock = MockTransport::new(
            "sess-1",
            vec![
                Err(WpsError::from_http_status(429)),
                Err(WpsError::from_http_status(503)),
                Ok(json!({"sheets": []})),
            ],
        );
        let client = WpsClient::with_policy(mock, "file-1", fast_policy());
        client.initialize().expect("initialize");
        client.get_schema(3).expect("read retry succeeds");
        assert_eq!(client.transport.calls().len(), 3);
    }

    #[test]
    fn update_without_key_is_allowed() {
        let mock = MockTransport::new("sess-1", vec![Ok(json!({"records": []}))]);
        let client = WpsClient::new(mock, "file-1");
        client.initialize().expect("initialize");
        client
            .update_records(3, &[json!({"id": "r1", "fields": {}})])
            .expect("update without key");
        let calls = client.transport.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].args.get("idempotency_key").is_none());
    }

    #[test]
    fn batch_each_record_uses_distinct_keys() {
        let mock = MockTransport::new(
            "sess-1",
            vec![
                Ok(json!({"records": [{"id": "r1"}]})),
                Err(WpsError::Timeout),
            ],
        );
        let client = WpsClient::new(mock, "file-1");
        client.initialize().expect("initialize");
        let first = json!({"fields": {"a": 1}});
        let second = json!({"fields": {"a": 2}});
        let results = client.create_records(3, &[keyed(&first, "k1"), keyed(&second, "k2")]);
        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
        assert_eq!(
            results[1].as_ref().expect_err("timeout"),
            &WpsError::Timeout
        );
        let calls = client.transport.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].args["idempotency_key"], "k1");
        assert_eq!(calls[1].args["idempotency_key"], "k2");
        assert_eq!(calls[0].args["records"].as_array().map(Vec::len), Some(1));
        assert_eq!(calls[1].args["records"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn writer_creates_each_record_and_persists_remote_ids() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let run_id = store.start_run("manual", "desktop").expect("run");
        let mock = MockTransport::new(
            "sess-1",
            vec![
                Ok(json!({"records": [{"id": "r1"}]})),
                Ok(json!({"records": [{"id": "r2"}]})),
                Ok(json!({"records": [{"id": "r3"}]})),
            ],
        );
        let client = WpsClient::new(mock, "file-1");
        client.initialize().expect("initialize");
        let writer = WpsWriteSession::new(&client, &store, run_id);
        let records = [
            json!({"fields": {"n": 1}}),
            json!({"fields": {"n": 2}}),
            json!({"fields": {"n": 3}}),
        ];
        let outcomes = writer.create_records(
            3,
            "03",
            &[
                keyed(&records[0], "k1"),
                keyed(&records[1], "k2"),
                keyed(&records[2], "k3"),
            ],
        );
        assert_eq!(
            outcomes.iter().map(|o| o.status).collect::<Vec<_>>(),
            vec![WriteStatus::Succeeded; 3]
        );
        assert_eq!(
            outcomes
                .iter()
                .map(|o| o.remote_record_id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("r1"), Some("r2"), Some("r3")]
        );
        assert_eq!(client.transport.calls().len(), 3);
        assert_eq!(
            store
                .idempotency_key("k1")
                .expect("get")
                .expect("present")
                .local_status,
            "succeeded"
        );
        assert_eq!(
            store
                .idempotency_key("k2")
                .expect("get")
                .expect("present")
                .remote_record_id
                .as_deref(),
            Some("r2")
        );
    }

    #[test]
    fn writer_partial_failure_stops_and_marks_unknown() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let run_id = store.start_run("manual", "desktop").expect("run");
        let mock = MockTransport::new(
            "sess-1",
            vec![
                Ok(json!({"records": [{"id": "r1"}]})),
                Err(WpsError::Timeout),
            ],
        );
        let client = WpsClient::new(mock, "file-1");
        client.initialize().expect("initialize");
        let writer = WpsWriteSession::new(&client, &store, run_id);
        let records = [
            json!({"fields": {"n": 1}}),
            json!({"fields": {"n": 2}}),
            json!({"fields": {"n": 3}}),
        ];
        let outcomes = writer.create_records(
            3,
            "03",
            &[
                keyed(&records[0], "k1"),
                keyed(&records[1], "k2"),
                keyed(&records[2], "k3"),
            ],
        );
        assert_eq!(outcomes.len(), 3);
        assert_eq!(outcomes[0].status, WriteStatus::Succeeded);
        assert_eq!(outcomes[1].status, WriteStatus::Unknown);
        assert_eq!(outcomes[2].status, WriteStatus::NotAttempted);
        assert_eq!(client.transport.calls().len(), 2);
        assert_eq!(
            store
                .idempotency_key("k1")
                .expect("get")
                .expect("present")
                .local_status,
            "succeeded"
        );
        assert_eq!(
            store
                .idempotency_key("k2")
                .expect("get")
                .expect("present")
                .local_status,
            "unknown"
        );
        assert!(store.idempotency_key("k3").expect("get").is_none());
    }

    #[test]
    fn writer_rejects_empty_key_without_request() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let run_id = store.start_run("manual", "desktop").expect("run");
        let mock = MockTransport::new("sess-1", vec![Ok(json!({"records": [{"id": "r1"}]}))]);
        let client = WpsClient::new(mock, "file-1");
        let writer = WpsWriteSession::new(&client, &store, run_id);
        let record = json!({"fields": {}});
        let outcomes = writer.create_records(3, "03", &[keyed(&record, "  ")]);
        assert_eq!(outcomes[0].status, WriteStatus::Rejected);
        assert!(client.transport.calls().is_empty());
    }

    #[test]
    fn extract_remote_record_id_accepts_known_shapes() {
        assert_eq!(
            extract_remote_record_id(&json!({"records": [{"id": "r1"}]})).as_deref(),
            Some("r1")
        );
        assert_eq!(
            extract_remote_record_id(&json!({"data": {"records": [{"id": "r2"}]}})).as_deref(),
            Some("r2")
        );
        assert_eq!(extract_remote_record_id(&json!({"ok": true})), None);
    }

    #[test]
    fn idempotency_keys_roundtrip() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        let run_id = store.start_run("manual", "desktop").expect("run");
        store
            .insert_idempotency_key("k1", run_id, "03")
            .expect("insert");
        let row = store.idempotency_key("k1").expect("get").expect("present");
        assert_eq!(row.local_status, "pending");
        store
            .mark_idempotency_key("k1", "succeeded", Some("Dl9"))
            .expect("mark");
        let row = store.idempotency_key("k1").expect("get").expect("present");
        assert_eq!(row.local_status, "succeeded");
        assert_eq!(row.remote_record_id.as_deref(), Some("Dl9"));
    }

    #[test]
    fn transport_mode_contract() {
        assert_eq!(
            WpsTransportMode::parse("mock"),
            Some(WpsTransportMode::Mock)
        );
        assert_eq!(
            WpsTransportMode::parse(" http "),
            Some(WpsTransportMode::Http)
        );
        assert_eq!(WpsTransportMode::parse("ftp"), None);
        assert_eq!(WpsTransportMode::default(), WpsTransportMode::Mock);
    }

    #[test]
    fn single_record_write_latency_baseline() {
        use std::time::Instant;
        let responses = (0..100)
            .map(|i| Ok(json!({"records": [{"id": format!("r{i}")}]})))
            .collect();
        let mock = MockTransport::new("sess-1", responses);
        let client = WpsClient::new(mock, "file-1");
        client.initialize().expect("initialize");
        let start = Instant::now();
        for _ in 0..100 {
            let record = json!({"fields": {"a": 1}});
            client
                .create_record(3, &record, "perf-key")
                .expect("mock create");
        }
        let average = start.elapsed() / 100;
        eprintln!("mock single-record write average: {average:?}");
        assert!(average < std::time::Duration::from_millis(20));
    }

    #[test]
    fn parse_mcp_payload_accepts_plain_json_and_sse() {
        let plain = parse_mcp_payload(r#"{"result":{"ok":true}}"#).expect("plain");
        assert_eq!(plain["result"]["ok"], true);

        let sse =
            parse_mcp_payload("event: message\ndata: {\"result\":{\"ok\":true}}\n\n").expect("sse");
        assert_eq!(sse["result"]["ok"], true);

        let err = parse_mcp_payload("not json").expect_err("invalid");
        assert_eq!(err.code(), "WPS_E_PROTOCOL");
    }

    #[test]
    fn remote_error_text_never_echoes_free_text() {
        let secret = "synthetic-token-13800000000-91310000MA1K-128600";
        let err = WpsError::from_mcp_error(&json!({
            "code": 500,
            "message": format!("{secret} 后台错误"),
            "fields": {"客户": "某客户", "金额": 128600, "电话": "13800000000"}
        }));
        let text = err.to_string();
        assert_eq!(err.code(), "WPS_E_REMOTE");
        assert!(text.contains("WPS_E_REMOTE"));
        assert!(!text.contains(secret));
        assert!(!text.contains("13800000000"));
        assert!(!text.contains("128600"));
        assert!(!text.contains("91310000MA1K"));
        assert!(!text.contains("某客户"));
        assert!(!text.contains("后台错误"));
    }

    struct StaticProvider(&'static str);

    impl WpsCredentialProvider for StaticProvider {
        fn access_token(&self) -> std::result::Result<String, WpsError> {
            Ok(self.0.into())
        }
    }

    struct FailingProvider;

    impl WpsCredentialProvider for FailingProvider {
        fn access_token(&self) -> std::result::Result<String, WpsError> {
            Err(WpsError::AuthExpired)
        }
    }

    #[test]
    fn credential_token_reaches_transport_auth_and_not_arguments() {
        let transport = MockTransport::new("sess-auth", vec![Ok(json!({"ok": true}))]);
        let client = WpsClient::new(transport, "file-1")
            .with_credentials(Arc::new(StaticProvider("synthetic-token")));
        client.initialize().expect("initialize with token");
        client
            .call("dbsheet.list_records", &json!({"sheet_id": 3}))
            .expect("call");
        assert_eq!(
            client.transport().initialize_auths(),
            vec![Some("synthetic-token".to_string())]
        );
        let calls = client.transport().calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].auth.as_deref(), Some("synthetic-token"));
        assert!(!calls[0].args.to_string().contains("synthetic-token"));
    }

    #[test]
    fn credential_failure_stops_before_any_transport_call() {
        let mock = MockTransport::new("sess-auth", vec![]);
        let client = WpsClient::new(mock, "file-1").with_credentials(Arc::new(FailingProvider));
        let err = client.initialize().expect_err("expired token must stop");
        assert_eq!(err.code(), "WPS_E_AUTH");
        assert_eq!(client.transport().initialize_calls(), 0);
        assert!(client.transport().calls().is_empty());
    }

    #[test]
    fn sync_runs_are_trimmed_to_latest_500() {
        let db = db_with_schema();
        let store = WpsStore::from_db(&db);
        for _ in 0..505 {
            let run_id = store.start_run("manual", "test").expect("start");
            store
                .finish_run(run_id, "ok", 0, 0, 0, 0, 0, 0, 0, None)
                .expect("finish");
        }
        let count: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM wps_sync_runs", [], |row| row.get(0))
                    .map_err(Into::into)
            })
            .expect("count");
        assert_eq!(count, WPS_SYNC_RUN_KEEP);
    }

    #[test]
    fn file_whitelist_rejects_before_transport() {
        let mock = MockTransport::new("sess-1", vec![]);
        let client = WpsClient::new(mock, "file-1").with_allowed_files(["file-2"]);
        let err = client.initialize().expect_err("blocked file");
        assert_eq!(err.code(), "WPS_E_PROTOCOL");
        assert!(err.to_string().contains("WPS_E_FILE_NOT_ALLOWED"));
        assert!(client.transport().calls().is_empty());
    }

    #[test]
    fn file_whitelist_allows_target_file() {
        let mock = MockTransport::new("sess-1", vec![]);
        let client = WpsClient::new(mock, "file-1").with_allowed_files(["file-1"]);
        client.initialize().expect("allowed file");
    }
}
