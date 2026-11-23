//! P2 专用只读入口（v0.28，S0 签审材料 A2 前置；默认关闭）。
//!
//! 边界：
//! - 只有**已签批**的授权（`P2Grant.approved_by`）才可能打开入口；默认 [`P2Gate::Closed`]。
//! - 发任何请求之前先做本地预检：端点、目标 `file_id`、表与字段范围、凭据授权主体、
//!   执行窗口、动作。任何一项缺失、过期或不匹配都在本地拒绝，**零网络请求**。
//! - 业务动作只允许 `dbsheet.list_records`；请求参数由授权派生，调用方无法扩大范围。
//! - 响应里出现未授权字段也按范围越界拒绝（不静默丢弃）。
//! - 审计行只写脱敏必要信息（掩码 file_id、主机名、表号、动作、结果码、尝试次数）。
//! - 真实 HTTPS 线协议（会话初始化报文、请求头、服务端只读权限）尚未取得可核实依据，
//!   因此本模块只交付守卫、策略与离线测试；真实传输与 A2 参数仍待签批方填写。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

/// 唯一允许的读取动作。
pub const P2_ACTION_LIST_RECORDS: &str = "dbsheet.list_records";

/// 单次读取的传输尝试上限（会话重建不计入次数）。
pub const P2_MAX_READ_ATTEMPTS: usize = 3;

/// P2 入口错误；`code()` 是进入日志/证据的稳定码，文案不含远端自由文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P2Error {
    /// 入口默认关闭：没有 A2 签批授权。
    Closed,
    /// 授权缺少签批人/签批号。
    NotApproved,
    /// 端点未获批、非 https、含用户信息/查询串/片段或不是固定路径。
    Endpoint(String),
    /// 目标 `file_id` 与授权不一致。
    Target(String),
    /// 表或字段超出授权范围。
    Scope(String),
    /// 授权主体（账号/租户）待补或与凭据不一致。
    ScopePending(String),
    /// 不在签批执行窗口内。
    Window(String),
    /// 动作不是只读 `dbsheet.list_records`。
    Action(String),
    /// 服务端返回 3xx；不跟随转发。
    Redirect,
    /// 证书不可信（本地拒绝，不重试）。
    Tls(String),
    /// 主机名与证书不匹配（本地拒绝，不重试）。
    Hostname(String),
    /// 缺少可用凭据；请求前拒绝。
    Credentials(String),
    /// 401/403：授权失效。
    AuthExpired,
    /// 404：MCP 会话过期；允许重建一次会话。
    SessionExpired,
    /// 429：限流；可重试。
    RateLimited,
    /// 超时；可重试。
    Timeout,
    /// 5xx；可重试。
    Server(u16),
    /// 响应结构不合法（缺 records 数组等）。
    Malformed(String),
    /// 真实线协议/端点依据不足，传输未接线。
    ProtocolPending(String),
    /// 其他本地协议错误。
    Protocol(String),
}

impl P2Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Closed => "P2_E_CLOSED",
            Self::NotApproved => "P2_E_NOT_APPROVED",
            Self::Endpoint(_) => "P2_E_ENDPOINT",
            Self::Target(_) => "P2_E_TARGET",
            Self::Scope(_) => "P2_E_SCOPE",
            Self::ScopePending(_) => "P2_E_SCOPE_PENDING",
            Self::Window(_) => "P2_E_WINDOW",
            Self::Action(_) => "P2_E_ACTION",
            Self::Redirect => "P2_E_REDIRECT",
            Self::Tls(_) => "P2_E_TLS",
            Self::Hostname(_) => "P2_E_HOST",
            Self::Credentials(_) => "P2_E_CREDENTIALS",
            Self::AuthExpired => "P2_E_AUTH",
            Self::SessionExpired => "P2_E_SESSION",
            Self::RateLimited => "P2_E_RATE",
            Self::Timeout => "P2_E_TIMEOUT",
            Self::Server(_) => "P2_E_SERVER",
            Self::Malformed(_) => "P2_E_MALFORMED",
            Self::ProtocolPending(_) => "P2_E_PROTOCOL_PENDING",
            Self::Protocol(_) => "P2_E_PROTOCOL",
        }
    }

    /// 只读是否可重试；鉴权/TLS/范围类错误一律不重试。
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::RateLimited | Self::Timeout | Self::Server(_))
    }
}

impl std::fmt::Display for P2Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "{}: P2 入口未打开（需要 A2 单独签批）", self.code()),
            Self::NotApproved => write!(f, "{}: 授权缺少签批人", self.code()),
            Self::Endpoint(msg)
            | Self::Target(msg)
            | Self::Scope(msg)
            | Self::ScopePending(msg)
            | Self::Window(msg)
            | Self::Action(msg)
            | Self::Tls(msg)
            | Self::Hostname(msg)
            | Self::Credentials(msg)
            | Self::Malformed(msg)
            | Self::ProtocolPending(msg)
            | Self::Protocol(msg) => write!(f, "{}: {msg}", self.code()),
            Self::Redirect => write!(f, "{}: 服务端返回重定向，未跟随", self.code()),
            Self::AuthExpired => write!(f, "{}: WPS 授权已失效或无权访问", self.code()),
            Self::SessionExpired => write!(f, "{}: MCP 会话已过期", self.code()),
            Self::RateLimited => write!(f, "{}: WPS 限流", self.code()),
            Self::Timeout => write!(f, "{}: 请求超时", self.code()),
            Self::Server(status) => write!(f, "{}: 服务端 HTTP {status}", self.code()),
        }
    }
}

impl std::error::Error for P2Error {}

/// 已批准的 HTTPS 端点；只接受固定 scheme/host/path，不接受查询串与片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2Endpoint {
    host: String,
    port: u16,
    path: String,
}

impl P2Endpoint {
    pub fn parse(raw: &str) -> Result<Self, P2Error> {
        let raw = raw.trim();
        let rest = raw.strip_prefix("https://").ok_or_else(|| {
            P2Error::Endpoint("P2 只允许 https 端点（不接受 http/其它协议）".into())
        })?;
        if rest.is_empty() {
            return Err(P2Error::Endpoint("P2 端点缺少主机".into()));
        }
        if rest.contains('@') {
            return Err(P2Error::Endpoint("P2 端点不接受用户信息".into()));
        }
        if rest.contains('?') || rest.contains('#') {
            return Err(P2Error::Endpoint("P2 端点不接受查询串或片段".into()));
        }
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, "/mcp"),
        };
        if authority.is_empty() {
            return Err(P2Error::Endpoint("P2 端点缺少主机".into()));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
                let port: u16 = port
                    .parse()
                    .map_err(|_| P2Error::Endpoint("P2 端点端口非法".into()))?;
                (host.to_string(), port)
            }
            Some(_) => return Err(P2Error::Endpoint("P2 端点端口非法".into())),
            None => (authority.to_string(), 443u16),
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() || !host.contains('.') || host.parse::<std::net::IpAddr>().is_ok() {
            return Err(P2Error::Endpoint(
                "P2 端点必须是已批准的域名（不接受 IP 或单标签主机）".into(),
            ));
        }
        if !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            return Err(P2Error::Endpoint("P2 端点主机字符非法".into()));
        }
        if path.len() > 200 || !path.starts_with('/') {
            return Err(P2Error::Endpoint("P2 端点路径不合法".into()));
        }
        Ok(Self {
            host,
            port,
            path: path.to_string(),
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn as_url(&self) -> String {
        if self.port == 443 {
            format!("https://{}{}", self.host, self.path)
        } else {
            format!("https://{}:{}{}", self.host, self.port, self.path)
        }
    }
}

/// 授权动作；只有 `ListRecords` 能通过预检。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum P2Action {
    ListRecords,
    Write,
}

impl P2Action {
    pub fn parse(raw: &str) -> Result<Self, P2Error> {
        match raw.trim() {
            P2_ACTION_LIST_RECORDS => Ok(Self::ListRecords),
            "dbsheet.create_records" | "dbsheet.update_records" => Ok(Self::Write),
            other => Err(P2Error::Action(format!(
                "P2 只允许 {P2_ACTION_LIST_RECORDS}，收到 {other}"
            ))),
        }
    }

    pub fn is_read_only(self) -> bool {
        matches!(self, Self::ListRecords)
    }
}

/// A2 签批的授权参数。字段都来自签批单；未知值保持 `None`／空表即视为待补。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2Grant {
    pub approved_by: Option<String>,
    pub approved_endpoint: P2Endpoint,
    pub endpoint: P2Endpoint,
    pub approved_file_id: String,
    pub file_id: String,
    pub sheets: BTreeMap<i64, BTreeSet<String>>,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub approved_subject: Option<String>,
}

impl P2Grant {
    /// 发网前预检：任何一项不满足都返回错误，且不产生传输调用。
    pub fn preflight(
        &self,
        now: DateTime<Utc>,
        action: P2Action,
        credential_subject: Option<&str>,
        sheet_id: i64,
    ) -> Result<(), P2Error> {
        if self.approved_by.as_deref().unwrap_or("").trim().is_empty() {
            return Err(P2Error::NotApproved);
        }
        if !action.is_read_only() {
            return Err(P2Error::Action(format!(
                "P2 只允许 {P2_ACTION_LIST_RECORDS}，写动作一律拒绝"
            )));
        }
        if self.endpoint != self.approved_endpoint {
            return Err(P2Error::Endpoint("请求端点与签批端点不一致，已拒绝".into()));
        }
        if self.file_id != self.approved_file_id {
            return Err(P2Error::Target("目标 file_id 与签批不一致，已拒绝".into()));
        }
        if now < self.window_start || now >= self.window_end {
            return Err(P2Error::Window("当前时间不在签批执行窗口内，已拒绝".into()));
        }
        let approved_subject = self
            .approved_subject
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                P2Error::ScopePending("授权主体（账号/租户）待补，不能视为已授权".into())
            })?;
        match credential_subject {
            Some(subject) if subject == approved_subject => {}
            Some(_) => {
                return Err(P2Error::Scope(
                    "凭据主体与签批授权主体不一致，已拒绝".into(),
                ))
            }
            None => {
                return Err(P2Error::ScopePending(
                    "凭据未提供可核对的主体，已拒绝".into(),
                ))
            }
        }
        if !self.sheets.contains_key(&sheet_id) {
            return Err(P2Error::Scope(format!("表 {sheet_id} 不在签批范围内")));
        }
        Ok(())
    }

    fn fields_for(&self, sheet_id: i64) -> Result<&BTreeSet<String>, P2Error> {
        self.sheets
            .get(&sheet_id)
            .ok_or_else(|| P2Error::Scope(format!("表 {sheet_id} 不在签批范围内")))
    }
}

/// 入口闸门：默认关闭，只有拿到签批授权才可打开。
#[derive(Debug, Clone, Default)]
pub enum P2Gate {
    #[default]
    Closed,
    Open(Box<P2Grant>),
}

impl P2Gate {
    pub fn open(grant: P2Grant) -> Self {
        Self::Open(Box::new(grant))
    }

    pub fn grant(&self) -> Result<&P2Grant, P2Error> {
        match self {
            Self::Closed => Err(P2Error::Closed),
            Self::Open(grant) => Ok(grant),
        }
    }
}

/// 一次分页读取请求；字段范围由授权决定，调用方不能自带字段清单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P2ReadRequest {
    pub sheet_id: i64,
    pub offset: usize,
    pub page_size: usize,
}

/// 读取结果（只保留已授权字段）。
#[derive(Debug, Clone, PartialEq)]
pub struct P2ReadOutcome {
    pub sheet_id: i64,
    pub records: Vec<Value>,
    pub total: Option<u64>,
    pub attempts_used: usize,
}

/// 凭据提供方：token 只进认证参数；`subject` 用于核对签批授权主体。
pub trait P2Credential: Send + Sync {
    fn access_token(&self) -> Result<String, P2Error>;

    /// 可核对的账号/租户标识；无法核对时返回 `None`（预检会拒绝）。
    fn subject(&self) -> Option<String>;
}

/// 时间源，便于离线测试窗口边界。
pub trait P2Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default)]
pub struct SystemP2Clock;

impl P2Clock for SystemP2Clock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// 传输边界：真实实现待 A2 参数到位后接线；离线测试用计数桩。
pub trait P2Transport: Send + Sync {
    fn initialize(&self, endpoint: &P2Endpoint, auth: Option<&str>) -> Result<String, P2Error>;

    fn call(
        &self,
        endpoint: &P2Endpoint,
        session_id: &str,
        action: &str,
        args: &Value,
        auth: Option<&str>,
    ) -> Result<Value, P2Error>;
}

/// 真实 HTTPS 传输的占位实现：会话报文、请求头与服务端只读权限尚无核实依据，
/// 因此任何调用都在本地失败，绝不发网。真实实现由 A2 参数到位后接线。
#[derive(Debug, Default)]
pub struct UnavailableP2Transport;

impl P2Transport for UnavailableP2Transport {
    fn initialize(&self, _endpoint: &P2Endpoint, _auth: Option<&str>) -> Result<String, P2Error> {
        Err(P2Error::ProtocolPending(
            "真实 HTTPS 会话协议尚未取得可核实依据，P2 传输未接线".into(),
        ))
    }

    fn call(
        &self,
        _endpoint: &P2Endpoint,
        _session_id: &str,
        _action: &str,
        _args: &Value,
        _auth: Option<&str>,
    ) -> Result<Value, P2Error> {
        Err(P2Error::ProtocolPending(
            "真实 HTTPS 会话协议尚未取得可核实依据，P2 传输未接线".into(),
        ))
    }
}

/// 审计行：只包含脱敏必要信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2AuditLine {
    pub host: String,
    pub file_id_masked: String,
    pub sheet_id: i64,
    pub action: &'static str,
    pub outcome: String,
    pub attempts: usize,
}

impl P2AuditLine {
    pub fn new(host: &str, file_id: &str, sheet_id: i64, outcome: &str, attempts: usize) -> Self {
        Self {
            host: host.to_string(),
            file_id_masked: mask_id(file_id),
            sheet_id,
            action: P2_ACTION_LIST_RECORDS,
            outcome: outcome.to_string(),
            attempts,
        }
    }
}

impl std::fmt::Display for P2AuditLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "p2 action={} host={} file={} sheet={} outcome={} attempts={}",
            self.action, self.host, self.file_id_masked, self.sheet_id, self.outcome, self.attempts
        )
    }
}

/// 只保留前 4 位与后 2 位，中间用省略号；过短则整体掩码。
fn mask_id(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 6 {
        return "***".into();
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars.iter().skip(chars.len() - 2).collect();
    format!("{head}…{tail}")
}

/// 固定 TLS 策略：证书与主机名都必须校验，https-only，不跟随重定向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P2TlsPolicy {
    pub verify_certificates: bool,
    pub verify_hostname: bool,
    pub https_only: bool,
    pub follow_redirects: bool,
}

impl Default for P2TlsPolicy {
    fn default() -> Self {
        Self {
            verify_certificates: true,
            verify_hostname: true,
            https_only: true,
            follow_redirects: false,
        }
    }
}

impl P2TlsPolicy {
    /// 任何一项被放宽都拒绝构造客户端。
    pub fn validate(&self) -> Result<(), P2Error> {
        if !self.verify_certificates {
            return Err(P2Error::Tls("P2 不允许关闭证书校验".into()));
        }
        if !self.verify_hostname {
            return Err(P2Error::Hostname("P2 不允许关闭主机名校验".into()));
        }
        if !self.https_only {
            return Err(P2Error::Endpoint("P2 只允许 https".into()));
        }
        if self.follow_redirects {
            return Err(P2Error::Redirect);
        }
        Ok(())
    }
}

/// 真实 HTTPS 只读传输：固定 JSON-RPC 报文、Bearer 头与 `Mcp-Session-Id` 会话，
/// 证书与主机名默认严格校验、不跟随重定向。
///
/// 报文形态取自仓库内已验证的回环传输（`services::wps_http`）与 MCP 公开规范；
/// 真实端点、账号主体与服务端只读权限仍由 A2 授权单决定（#70）。
pub struct HttpsP2Transport {
    endpoint: P2Endpoint,
    client: reqwest::blocking::Client,
}

impl std::fmt::Debug for HttpsP2Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpsP2Transport")
            .field("host", &self.endpoint.host())
            .finish()
    }
}

impl HttpsP2Transport {
    pub fn builder(endpoint: P2Endpoint) -> HttpsP2TransportBuilder {
        HttpsP2TransportBuilder::new(endpoint)
    }

    fn post(
        &self,
        endpoint: &P2Endpoint,
        session: Option<&str>,
        auth: Option<&str>,
        body: &Value,
    ) -> Result<reqwest::blocking::Response, P2Error> {
        if endpoint != &self.endpoint {
            return Err(P2Error::Endpoint(
                "传输层端点与授权端点不一致，拒绝请求".into(),
            ));
        }
        let mut request = self
            .client
            .post(endpoint.as_url())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .body(body.to_string());
        if let Some(token) = auth {
            request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        request.send().map_err(map_transport_error)
    }
}

/// HTTPS 传输构造器：允许测试注入私有 CA 与主机映射，生产调用只用默认严格策略。
pub struct HttpsP2TransportBuilder {
    endpoint: P2Endpoint,
    tls: P2TlsPolicy,
    timeout: Duration,
    roots: Vec<Vec<u8>>,
    resolves: Vec<(String, std::net::SocketAddr)>,
}

impl HttpsP2TransportBuilder {
    pub fn new(endpoint: P2Endpoint) -> Self {
        Self {
            endpoint,
            tls: P2TlsPolicy::default(),
            timeout: Duration::from_secs(10),
            roots: Vec::new(),
            resolves: Vec::new(),
        }
    }

    pub fn tls(mut self, tls: P2TlsPolicy) -> Self {
        self.tls = tls;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 追加信任根（DER）。仅用于本机测试 CA；生产端点必须由系统信任链验证。
    pub fn trust_root_der(mut self, der: Vec<u8>) -> Self {
        self.roots.push(der);
        self
    }

    /// 把主机名固定解析到指定地址（DER 测试服务用，避免真实 DNS）。
    pub fn resolve(mut self, host: &str, addr: std::net::SocketAddr) -> Self {
        self.resolves.push((host.to_string(), addr));
        self
    }

    pub fn build(self) -> Result<HttpsP2Transport, P2Error> {
        self.tls.validate()?;
        if !self.tls.https_only {
            return Err(P2Error::Endpoint("P2 传输只允许 https".into()));
        }
        let mut builder = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.timeout)
            .danger_accept_invalid_certs(!self.tls.verify_certificates)
            .danger_accept_invalid_hostnames(!self.tls.verify_hostname);
        for der in self.roots {
            let cert = reqwest::Certificate::from_der(&der)
                .map_err(|_| P2Error::Tls("测试信任根解析失败".into()))?;
            builder = builder.add_root_certificate(cert);
        }
        for (host, addr) in self.resolves {
            builder = builder.resolve(&host, addr);
        }
        let client = builder
            .build()
            .map_err(|_| P2Error::Protocol("HTTPS 客户端初始化失败".into()))?;
        Ok(HttpsP2Transport {
            endpoint: self.endpoint,
            client,
        })
    }
}

impl P2Transport for HttpsP2Transport {
    fn initialize(&self, endpoint: &P2Endpoint, auth: Option<&str>) -> Result<String, P2Error> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "voucher-studio-p2", "version": "0.28"}
            }
        });
        let response = self.post(endpoint, None, auth, &body)?;
        if let Some(err) = classify_http_status(response.status().as_u16()) {
            return Err(err);
        }
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let text = response
            .text()
            .map_err(|_| P2Error::Protocol("初始化响应读取失败".into()))?;
        let session = session.or_else(|| {
            serde_json::from_str::<Value>(&text).ok().and_then(|value| {
                value
                    .pointer("/result/sessionId")
                    .or_else(|| value.pointer("/result/session_id"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        });
        match session {
            Some(session) if !session.is_empty() => Ok(session),
            _ => Err(P2Error::Protocol("初始化响应缺少 MCP session id".into())),
        }
    }

    fn call(
        &self,
        endpoint: &P2Endpoint,
        session_id: &str,
        action: &str,
        args: &Value,
        auth: Option<&str>,
    ) -> Result<Value, P2Error> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": action, "arguments": args}
        });
        let response = self.post(endpoint, Some(session_id), auth, &body)?;
        if let Some(err) = classify_http_status(response.status().as_u16()) {
            return Err(err);
        }
        let text = response
            .text()
            .map_err(|_| P2Error::Protocol("响应读取失败".into()))?;
        let value: Value =
            serde_json::from_str(&text).map_err(|_| P2Error::Protocol("响应不是 JSON".into()))?;
        if let Some(error) = value.get("error") {
            return Err(map_mcp_error(error));
        }
        if value
            .pointer("/result/isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(P2Error::Protocol("远端返回工具错误，已停止".into()));
        }
        let result = value
            .get("result")
            .cloned()
            .ok_or_else(|| P2Error::Protocol("响应缺少 result".into()))?;
        // MCP 2025-06-18 `tools/call` 的结果形状是 `content: [{type:"text", text: ...}]`，
        // 工具负载通常在 text 里以 JSON 文本返回；这里按规范解包，兼容直接返回负载的服务端。
        if let Some(parts) = result.get("content").and_then(Value::as_array) {
            let mut text = String::new();
            for part in parts {
                if part.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(chunk) = part.get("text").and_then(Value::as_str) {
                        text.push_str(chunk);
                    }
                }
            }
            if !text.is_empty() {
                return serde_json::from_str(&text)
                    .map_err(|_| P2Error::Protocol("工具结果不是 JSON".into()));
            }
        }
        Ok(result)
    }
}

/// 只按错误类型归类传输失败，不回显远端或系统原始文本。
fn map_transport_error(err: reqwest::Error) -> P2Error {
    if err.is_timeout() {
        return P2Error::Timeout;
    }
    let mut text = String::new();
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&err);
    while let Some(current) = source {
        text.push_str(&current.to_string());
        text.push(' ');
        source = current.source();
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("hostname") || lower.contains("host name") || lower.contains("dnsname") {
        P2Error::Hostname("证书主机名不匹配，已拒绝".into())
    } else if lower.contains("certificate") || lower.contains("tls") || lower.contains("ssl") {
        P2Error::Tls("证书不可信或 TLS 握手失败，已拒绝".into())
    } else {
        P2Error::Protocol("HTTPS 连接失败（本地拒绝，不重试）".into())
    }
}

/// JSON-RPC 错误码映射；只保留错误码，不回显错误文本。
fn map_mcp_error(error: &Value) -> P2Error {
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    match code {
        400006 => P2Error::AuthExpired,
        404 => P2Error::SessionExpired,
        429 => P2Error::RateLimited,
        other => P2Error::Protocol(format!("远端错误码 {other}")),
    }
}

/// HTTP 状态到 P2 错误的映射；真实传输接线后复用，离线可测。
pub fn classify_http_status(status: u16) -> Option<P2Error> {
    match status {
        200..=299 => None,
        300..=399 => Some(P2Error::Redirect),
        401 | 403 => Some(P2Error::AuthExpired),
        404 => Some(P2Error::SessionExpired),
        429 => Some(P2Error::RateLimited),
        500..=599 => Some(P2Error::Server(status)),
        other => Some(P2Error::Protocol(format!("HTTP {other}"))),
    }
}

/// TLS 连接失败的安全分类：只按提示词归类，不回显原始错误文本。
pub fn classify_connect_hint(hint: &str) -> P2Error {
    let lower = hint.to_ascii_lowercase();
    if lower.contains("hostname")
        || lower.contains("host name")
        || lower.contains("dnsname")
        || lower.contains("subject")
    {
        P2Error::Hostname("证书主机名不匹配，已拒绝".into())
    } else {
        P2Error::Tls("证书不可信或 TLS 握手失败，已拒绝".into())
    }
}

/// P2 只读入口。
pub struct P2ReadOnlyEntry<T: P2Transport> {
    transport: T,
    credential: Option<Arc<dyn P2Credential>>,
    clock: Arc<dyn P2Clock>,
    backoff: Duration,
    max_attempts: usize,
    /// 测试用审计汇聚点：与 `log::info!` 同源同格式，供证据用例断言脱敏内容。
    audit_sink: Option<Arc<std::sync::Mutex<Vec<String>>>>,
}

impl<T: P2Transport> P2ReadOnlyEntry<T> {
    pub fn new(transport: T, credential: Option<Arc<dyn P2Credential>>) -> Self {
        Self {
            transport,
            credential,
            clock: Arc::new(SystemP2Clock),
            backoff: Duration::from_secs(1),
            max_attempts: P2_MAX_READ_ATTEMPTS,
            audit_sink: None,
        }
    }

    /// 测试用：把审计行同时写入给定缓冲，便于证据用例逐字核对。
    pub fn with_audit_sink(mut self, sink: Arc<std::sync::Mutex<Vec<String>>>) -> Self {
        self.audit_sink = Some(sink);
        self
    }

    /// 测试用：固定时钟、零退避、可控尝试上限。
    pub fn with_clock_and_backoff(
        mut self,
        clock: Arc<dyn P2Clock>,
        backoff: Duration,
        max_attempts: usize,
    ) -> Self {
        self.clock = clock;
        self.backoff = backoff;
        self.max_attempts = max_attempts.max(1);
        self
    }

    pub fn read(&self, gate: &P2Gate, request: &P2ReadRequest) -> Result<P2ReadOutcome, P2Error> {
        let grant = gate.grant()?;
        let subject = self.credential.as_ref().and_then(|c| c.subject());
        grant.preflight(
            self.clock.now(),
            P2Action::ListRecords,
            subject.as_deref(),
            request.sheet_id,
        )?;

        let token = match &self.credential {
            Some(credential) => {
                let token = credential.access_token()?;
                if token.trim().is_empty() {
                    return Err(P2Error::Credentials("WPS token 为空，拒绝请求".into()));
                }
                Some(token)
            }
            None => None,
        };

        let fields: Vec<String> = grant
            .fields_for(request.sheet_id)?
            .iter()
            .cloned()
            .collect();
        let args = json!({
            "file_id": grant.file_id,
            "sheet_id": request.sheet_id,
            "offset": request.offset,
            "page_size": request.page_size,
            "fields": fields,
        });

        let mut session = self
            .transport
            .initialize(&grant.endpoint, token.as_deref())?;
        // 读取预算：一次逻辑读取最多真实发出 max_attempts 次 list_records，
        // 首次调用、会话重建后的重发、超时重试都各消耗一次；初始化不占用读取预算。
        let mut sent = 0usize;
        let mut reinitialized = false;
        let mut last_error: Option<P2Error> = None;
        loop {
            if sent >= self.max_attempts {
                let err = last_error.unwrap_or_else(|| {
                    P2Error::Malformed(format!("读取预算耗尽（上限 {sent} 次）"))
                });
                self.audit(grant, request.sheet_id, err.code(), sent);
                return Err(err);
            }
            sent += 1;
            match self.transport.call(
                &grant.endpoint,
                &session,
                P2_ACTION_LIST_RECORDS,
                &args,
                token.as_deref(),
            ) {
                Ok(value) => {
                    let (records, total) =
                        match self.validate_records(grant, request.sheet_id, &value) {
                            Ok(validated) => validated,
                            Err(err) => {
                                // 响应越界也要留审计：只记错误码与脱敏范围，不记字段值。
                                self.audit(grant, request.sheet_id, err.code(), sent);
                                return Err(err);
                            }
                        };
                    let outcome = P2ReadOutcome {
                        sheet_id: request.sheet_id,
                        records,
                        total,
                        attempts_used: sent,
                    };
                    self.audit(grant, request.sheet_id, "ok", sent);
                    return Ok(outcome);
                }
                Err(P2Error::SessionExpired) => {
                    // 会话重建最多一次，且消耗已发出的那一次读取预算。
                    if reinitialized {
                        self.audit(grant, request.sheet_id, "P2_E_SESSION", sent);
                        return Err(P2Error::SessionExpired);
                    }
                    reinitialized = true;
                    session = self
                        .transport
                        .initialize(&grant.endpoint, token.as_deref())?;
                    last_error = Some(P2Error::SessionExpired);
                }
                Err(err) if err.is_retryable() => {
                    last_error = Some(err);
                    if sent < self.max_attempts {
                        std::thread::sleep(self.backoff);
                    }
                }
                Err(err) => {
                    self.audit(grant, request.sheet_id, err.code(), sent);
                    return Err(err);
                }
            }
        }
    }

    fn validate_records(
        &self,
        grant: &P2Grant,
        sheet_id: i64,
        value: &Value,
    ) -> Result<(Vec<Value>, Option<u64>), P2Error> {
        let allowed = grant.fields_for(sheet_id)?;
        let records = value
            .get("records")
            .and_then(Value::as_array)
            .ok_or_else(|| P2Error::Malformed("响应缺少 records 数组".into()))?;
        let mut kept = Vec::with_capacity(records.len());
        for record in records {
            let object = record
                .as_object()
                .ok_or_else(|| P2Error::Malformed("records 元素不是对象".into()))?;
            // 兼容扁平结构：除 id/record_id 外的键都视为字段。
            let fields = object
                .get("fields")
                .and_then(Value::as_object)
                .or(Some(object));
            let Some(fields) = fields else {
                return Err(P2Error::Malformed("记录缺少 fields 对象".into()));
            };
            let mut filtered: Map<String, Value> = Map::new();
            for (key, item) in fields {
                if key == "id" || key == "record_id" {
                    continue;
                }
                if !allowed.contains(key) {
                    return Err(P2Error::Scope(format!(
                        "响应包含未授权字段 {key}（表 {sheet_id}）"
                    )));
                }
                filtered.insert(key.clone(), item.clone());
            }
            kept.push(Value::Object(filtered));
        }
        let total = value.get("total").and_then(Value::as_u64);
        Ok((kept, total))
    }

    fn audit(&self, grant: &P2Grant, sheet_id: i64, outcome: &str, attempts: usize) {
        let line = P2AuditLine::new(
            grant.endpoint.host(),
            &grant.file_id,
            sheet_id,
            outcome,
            attempts,
        );
        if let Some(sink) = &self.audit_sink {
            if let Ok(mut lines) = sink.lock() {
                lines.push(line.to_string());
            }
        }
        log::info!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    fn endpoint(raw: &str) -> P2Endpoint {
        P2Endpoint::parse(raw).expect("endpoint")
    }

    fn grants(sheets: &[(i64, &[&str])]) -> P2Grant {
        let mut map = BTreeMap::new();
        for (sheet, fields) in sheets {
            map.insert(
                *sheet,
                fields.iter().map(|field| (*field).to_string()).collect(),
            );
        }
        P2Grant {
            approved_by: Some("A2-2026-11-22".into()),
            approved_endpoint: endpoint("https://wps.example.test/mcp"),
            endpoint: endpoint("https://wps.example.test/mcp"),
            approved_file_id: "dbsheet_replica_001".into(),
            file_id: "dbsheet_replica_001".into(),
            sheets: map,
            window_start: DateTime::parse_from_rfc3339("2026-11-22T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            window_end: DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            approved_subject: Some("tenant-a/reader".into()),
        }
    }

    struct FixedClock(DateTime<Utc>);

    impl P2Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    struct SyntheticCredential {
        subject: Option<String>,
    }

    impl P2Credential for SyntheticCredential {
        fn access_token(&self) -> Result<String, P2Error> {
            Ok("synthetic-p2-token".into())
        }

        fn subject(&self) -> Option<String> {
            self.subject.clone()
        }
    }

    #[derive(Default)]
    struct CountingTransport {
        init: Mutex<usize>,
        calls: Mutex<Vec<(String, Value)>>,
        script: Mutex<Vec<Result<Value, P2Error>>>,
    }

    impl CountingTransport {
        fn with_script(script: Vec<Result<Value, P2Error>>) -> Self {
            Self {
                script: Mutex::new(script),
                ..Self::default()
            }
        }

        fn requests(&self) -> usize {
            *self.init.lock().unwrap() + self.calls.lock().unwrap().len()
        }
    }

    impl P2Transport for CountingTransport {
        fn initialize(&self, endpoint: &P2Endpoint, auth: Option<&str>) -> Result<String, P2Error> {
            assert_eq!(endpoint.host(), "wps.example.test");
            assert!(auth.is_some(), "P2 需要显式 token");
            *self.init.lock().unwrap() += 1;
            Ok("session-1".into())
        }

        fn call(
            &self,
            _endpoint: &P2Endpoint,
            _session_id: &str,
            action: &str,
            args: &Value,
            _auth: Option<&str>,
        ) -> Result<Value, P2Error> {
            assert_eq!(action, P2_ACTION_LIST_RECORDS);
            self.calls
                .lock()
                .unwrap()
                .push((action.to_string(), args.clone()));
            self.script
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Ok(json!({"records": [], "total": 0})))
        }
    }

    fn entry(
        transport: CountingTransport,
        subject: Option<&str>,
        now: &str,
    ) -> P2ReadOnlyEntry<CountingTransport> {
        let clock = Arc::new(FixedClock(
            DateTime::parse_from_rfc3339(now)
                .unwrap()
                .with_timezone(&Utc),
        ));
        P2ReadOnlyEntry::new(
            transport,
            Some(Arc::new(SyntheticCredential {
                subject: subject.map(str::to_string),
            })),
        )
        .with_clock_and_backoff(clock, Duration::ZERO, P2_MAX_READ_ATTEMPTS)
    }

    #[test]
    fn p2_default_gate_is_closed_and_issues_zero_requests() {
        let transport = CountingTransport::default();
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let err = reader
            .read(
                &P2Gate::Closed,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_CLOSED");
        assert_eq!(reader.transport.requests(), 0);
    }

    #[test]
    fn p2_legal_read_runs_full_chain_with_scope_derived_args() {
        let transport = CountingTransport::with_script(vec![Ok(json!({
            "records": [{"fields": {"A": 1, "B": "x"}}],
            "total": 1
        }))]);
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let gate = P2Gate::open(grants(&[(3, &["A", "B", "C"])]));
        let outcome = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap();
        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.total, Some(1));
        assert_eq!(outcome.attempts_used, 1);
        let calls = reader.transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let args = &calls[0].1;
        assert_eq!(args["file_id"], "dbsheet_replica_001");
        assert_eq!(args["sheet_id"], 3);
        assert_eq!(args["fields"], json!(["A", "B", "C"]));
        // token 不进 arguments。
        assert!(!args.to_string().contains("synthetic-p2-token"));
    }

    #[test]
    fn p2_denies_unapproved_target_before_any_request() {
        let transport = CountingTransport::default();
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let mut grant = grants(&[(3, &["A"])]);
        grant.file_id = "dbsheet_other".into();
        let err = reader
            .read(
                &P2Gate::open(grant),
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_TARGET");
        assert_eq!(reader.transport.requests(), 0);
    }

    #[test]
    fn p2_denies_sheet_outside_grant_and_response_field_outside_grant() {
        let transport = CountingTransport::default();
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let gate = P2Gate::open(grants(&[(3, &["A"])]));
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 9,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SCOPE");
        assert_eq!(reader.transport.requests(), 0);

        let transport = CountingTransport::with_script(vec![Ok(json!({
            "records": [{"fields": {"A": 1, "SECRET": "x"}}]
        }))]);
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SCOPE");
    }

    #[test]
    fn p2_denies_expired_window_and_unapproved_or_mismatched_subject() {
        let transport = CountingTransport::default();
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T13:30:00Z");
        let gate = P2Gate::open(grants(&[(3, &["A"])]));
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_WINDOW");
        assert_eq!(reader.transport.requests(), 0);

        let transport = CountingTransport::default();
        let reader = entry(transport, Some("tenant-b/reader"), "2026-11-22T10:30:00Z");
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SCOPE");
        assert_eq!(reader.transport.requests(), 0);

        let transport = CountingTransport::default();
        let reader = entry(transport, None, "2026-11-22T10:30:00Z");
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SCOPE_PENDING");
        assert_eq!(reader.transport.requests(), 0);
    }

    #[test]
    fn p2_rejects_unapproved_or_write_actions() {
        assert_eq!(
            P2Action::parse("dbsheet.create_records").unwrap(),
            P2Action::Write
        );
        assert!(!P2Action::parse("dbsheet.create_records")
            .unwrap()
            .is_read_only());
        assert_eq!(
            P2Action::parse("dbsheet.delete_records")
                .unwrap_err()
                .code(),
            "P2_E_ACTION"
        );

        let grant = grants(&[(3, &["A"])]);
        let unapproved = {
            let mut copy = grant.clone();
            copy.approved_by = None;
            copy
        };
        let err = unapproved
            .preflight(
                DateTime::parse_from_rfc3339("2026-11-22T10:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                P2Action::ListRecords,
                Some("tenant-a/reader"),
                3,
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_NOT_APPROVED");

        let err = grant
            .preflight(
                DateTime::parse_from_rfc3339("2026-11-22T10:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                P2Action::Write,
                Some("tenant-a/reader"),
                3,
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_ACTION");
    }

    #[test]
    fn p2_tls_and_hostname_failures_stop_without_follow_up_requests() {
        let transport = CountingTransport::with_script(vec![Err(P2Error::Tls("deny".into()))]);
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let gate = P2Gate::open(grants(&[(3, &["A"])]));
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_TLS");
        assert_eq!(reader.transport.calls.lock().unwrap().len(), 1);

        let transport = CountingTransport::with_script(vec![Err(P2Error::Hostname("deny".into()))]);
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_HOST");
        assert_eq!(reader.transport.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn p2_redirect_is_not_followed() {
        let transport = CountingTransport::with_script(vec![Err(P2Error::Redirect)]);
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let gate = P2Gate::open(grants(&[(3, &["A"])]));
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_REDIRECT");
        let calls = reader.transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "只允许一次请求，不跟随目标地址");
        assert!(!calls[0].1.to_string().contains("Location"));
    }

    #[test]
    fn p2_retries_are_capped_and_session_expiry_reinitializes_once() {
        // 脚本按 pop 取（后进先出）：会话过期 → 超时 → 5xx，第 4 个“成功”不得被取用。
        let transport = CountingTransport::with_script(vec![
            Ok(json!({"records": [{"A": "不应被取用"}]})),
            Err(P2Error::Server(500)),
            Err(P2Error::Timeout),
            Err(P2Error::SessionExpired),
        ]);
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let gate = P2Gate::open(grants(&[(3, &["A"])]));
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        // 预算 3 次：会话过期后的重发、超时、5xx 各占一次，耗尽即停止。
        assert_eq!(err.code(), "P2_E_SERVER");
        assert_eq!(reader.transport.calls.lock().unwrap().len(), 3);
        assert_eq!(*reader.transport.init.lock().unwrap(), 2);
        // 第 4 条成功响应仍在脚本里，未被消费。
        assert_eq!(reader.transport.script.lock().unwrap().len(), 1);

        // 预算内重建后读取成功：2 次调用、1 次重建、attempts_used=2。
        let transport = CountingTransport::with_script(vec![
            Ok(json!({"records": [{"A": "一"}]})),
            Err(P2Error::SessionExpired),
        ]);
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let outcome = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .expect("重建后应成功");
        assert_eq!(outcome.attempts_used, 2);
        assert_eq!(outcome.records.len(), 1);
        assert_eq!(reader.transport.calls.lock().unwrap().len(), 2);
        assert_eq!(*reader.transport.init.lock().unwrap(), 2);

        // 全部失败：总请求次数不超过上限。
        let transport = CountingTransport::with_script(vec![
            Err(P2Error::Server(503)),
            Err(P2Error::Server(503)),
            Err(P2Error::Server(503)),
        ]);
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z");
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SERVER");
        assert_eq!(reader.transport.calls.lock().unwrap().len(), 3);
    }

    #[test]
    fn p2_tls_policy_and_status_mapping_are_strict() {
        assert!(P2TlsPolicy::default().validate().is_ok());
        let relaxed = P2TlsPolicy {
            verify_certificates: false,
            ..P2TlsPolicy::default()
        };
        assert_eq!(relaxed.validate().unwrap_err().code(), "P2_E_TLS");
        let no_host = P2TlsPolicy {
            verify_hostname: false,
            ..P2TlsPolicy::default()
        };
        assert_eq!(no_host.validate().unwrap_err().code(), "P2_E_HOST");
        let redirects = P2TlsPolicy {
            follow_redirects: true,
            ..P2TlsPolicy::default()
        };
        assert_eq!(redirects.validate().unwrap_err().code(), "P2_E_REDIRECT");

        assert!(classify_http_status(200).is_none());
        assert_eq!(classify_http_status(302).unwrap().code(), "P2_E_REDIRECT");
        assert_eq!(classify_http_status(401).unwrap().code(), "P2_E_AUTH");
        assert_eq!(classify_http_status(404).unwrap().code(), "P2_E_SESSION");
        assert_eq!(classify_http_status(429).unwrap().code(), "P2_E_RATE");
        assert_eq!(classify_http_status(503).unwrap().code(), "P2_E_SERVER");
        assert_eq!(
            classify_connect_hint("invalid peer certificate").code(),
            "P2_E_TLS"
        );
        assert_eq!(
            classify_connect_hint("certificate subject name mismatch").code(),
            "P2_E_HOST"
        );
    }

    #[test]
    fn p2_endpoint_parsing_rejects_http_credentials_and_query() {
        assert_eq!(
            P2Endpoint::parse("http://wps.example.test/mcp")
                .unwrap_err()
                .code(),
            "P2_E_ENDPOINT"
        );
        assert_eq!(
            P2Endpoint::parse("https://user@wps.example.test/mcp")
                .unwrap_err()
                .code(),
            "P2_E_ENDPOINT"
        );
        assert_eq!(
            P2Endpoint::parse("https://wps.example.test/mcp?x=1")
                .unwrap_err()
                .code(),
            "P2_E_ENDPOINT"
        );
        assert_eq!(
            P2Endpoint::parse("https://127.0.0.1/mcp")
                .unwrap_err()
                .code(),
            "P2_E_ENDPOINT"
        );
        let parsed = P2Endpoint::parse("https://WPS.Example.Test:8443/mcp").unwrap();
        assert_eq!(parsed.host(), "wps.example.test");
        assert_eq!(parsed.port(), 8443);
        assert_eq!(parsed.as_url(), "https://wps.example.test:8443/mcp");
    }

    #[test]
    fn p2_unwired_https_transport_fails_before_sending() {
        let clock = Arc::new(FixedClock(
            DateTime::parse_from_rfc3339("2026-11-22T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        ));
        let reader = P2ReadOnlyEntry::new(
            UnavailableP2Transport,
            Some(Arc::new(SyntheticCredential {
                subject: Some("tenant-a/reader".into()),
            })),
        )
        .with_clock_and_backoff(clock, Duration::ZERO, P2_MAX_READ_ATTEMPTS);
        let err = reader
            .read(
                &P2Gate::open(grants(&[(3, &["A"])])),
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_PROTOCOL_PENDING");
    }

    #[test]
    fn p2_audit_line_masks_file_id_and_never_carries_token() {
        let line = P2AuditLine::new("wps.example.test", "dbsheet_replica_001", 3, "ok", 1);
        let text = line.to_string();
        assert!(text.contains("file=dbsh…01"));
        assert!(!text.contains("dbsheet_replica_001"));
        assert!(!text.contains("synthetic-p2-token"));
        assert!(line.action == P2_ACTION_LIST_RECORDS);
    }

    // ---- 本机 HTTPS 测试服务（自签 CA，静态 DER 夹具）----

    const TLS_CA: &[u8] = include_bytes!("../../tests/fixtures/p2-tls/ca.der");
    const TLS_SRV_CERT: &[u8] = include_bytes!("../../tests/fixtures/p2-tls/srv.der");
    const TLS_SRV_KEY: &[u8] = include_bytes!("../../tests/fixtures/p2-tls/srv.key.der");
    const TLS_OTHER_CERT: &[u8] = include_bytes!("../../tests/fixtures/p2-tls/other.der");
    const TLS_OTHER_KEY: &[u8] = include_bytes!("../../tests/fixtures/p2-tls/other.key.der");

    #[derive(Clone)]
    enum ServerMode {
        /// 正常返回 `{"records":[...]}`；`leak` 时额外返回未授权字段。
        JsonOk { leak: bool },
        /// 返回 302 与 Location，不改写目标。
        Redirect { location: String },
    }

    struct TlsServer {
        port: u16,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
    }

    impl TlsServer {
        fn endpoint(&self) -> P2Endpoint {
            endpoint(&format!("https://p2-local.test:{}/mcp", self.port))
        }

        fn count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
    }

    impl Drop for TlsServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    fn spawn_tls_server(cert: &'static [u8], key: &'static [u8], mode: ServerMode) -> TlsServer {
        use std::sync::Once;
        static PROVIDER: Once = Once::new();
        PROVIDER.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let config = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.to_vec())],
                    rustls::pki_types::PrivateKeyDer::Pkcs8(
                        rustls::pki_types::PrivatePkcs8KeyDer::from(key.to_vec()),
                    ),
                )
                .expect("server config"),
        );
        let requests_for_thread = Arc::clone(&requests);
        let stop_for_thread = Arc::clone(&stop);
        std::thread::spawn(move || loop {
            if stop_for_thread.load(Ordering::SeqCst) {
                break;
            }
            match listener.accept() {
                Ok((tcp, _)) => {
                    let _ = tcp.set_nonblocking(false);
                    let config = Arc::clone(&config);
                    let requests = Arc::clone(&requests_for_thread);
                    let mode = mode.clone();
                    std::thread::spawn(move || {
                        handle_tls_connection(tcp, config, mode, requests);
                    });
                }
                Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        });
        TlsServer {
            port,
            requests,
            stop,
        }
    }

    fn handle_tls_connection(
        tcp: std::net::TcpStream,
        config: Arc<rustls::ServerConfig>,
        mode: ServerMode,
        requests: Arc<Mutex<Vec<String>>>,
    ) {
        use std::io::{Read, Write};
        let Ok(connection) = rustls::ServerConnection::new(config) else {
            return;
        };
        let mut stream = rustls::StreamOwned::new(connection, tcp);
        let mut raw = Vec::new();
        let mut buffer = [0u8; 2048];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    raw.extend_from_slice(&buffer[..read]);
                    if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => return,
            }
        }
        if raw.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(&raw).to_string();
        let first_line = text.lines().next().unwrap_or_default().to_string();
        requests.lock().unwrap().push(first_line.clone());

        let initialize = text.contains("\"method\":\"initialize\"");
        let (status, headers, body) = match &mode {
            ServerMode::Redirect { location } => (
                302u16,
                vec![("Location".to_string(), location.clone())],
                String::new(),
            ),
            ServerMode::JsonOk { leak } => {
                if initialize {
                    (
                        200,
                        vec![("Mcp-Session-Id".to_string(), "sess-1".to_string())],
                        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\"}}"
                            .to_string(),
                    )
                } else {
                    let mut record = String::from("{\"A\":\"一\"");
                    if *leak {
                        record.push_str(",\"SECRET\":\"不可见\"");
                    }
                    record.push('}');
                    let payload = format!("{{\"records\":[{record}],\"total\":1}}");
                    let literal = serde_json::to_string(&payload).expect("payload literal");
                    (
                        200,
                        Vec::new(),
                        format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":{literal}}}],\"isError\":false}}}}"
                        ),
                    )
                }
            }
        };
        let mut response = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        response.push_str("\r\n");
        response.push_str(&body);
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    fn https_entry(
        transport: HttpsP2Transport,
        subject: &str,
    ) -> P2ReadOnlyEntry<HttpsP2Transport> {
        let clock = Arc::new(FixedClock(
            DateTime::parse_from_rfc3339("2026-11-22T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        ));
        P2ReadOnlyEntry::new(
            transport,
            Some(Arc::new(SyntheticCredential {
                subject: Some(subject.to_string()),
            })),
        )
        .with_clock_and_backoff(clock, Duration::ZERO, P2_MAX_READ_ATTEMPTS)
    }

    fn https_grant(server: &TlsServer) -> P2Gate {
        let mut grant = grants(&[(3, &["A"])]);
        grant.endpoint = server.endpoint();
        grant.approved_endpoint = server.endpoint();
        P2Gate::open(grant)
    }

    fn https_reader(server: &TlsServer, trust_ca: bool) -> HttpsP2Transport {
        let mut builder = HttpsP2Transport::builder(server.endpoint()).resolve(
            "p2-local.test",
            format!("127.0.0.1:{}", server.port).parse().unwrap(),
        );
        if trust_ca {
            builder = builder.trust_root_der(TLS_CA.to_vec());
        }
        builder.build().expect("transport")
    }

    #[test]
    fn p2_https_legal_read_runs_full_chain_with_trusted_ca() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);
        let outcome = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .expect("合法读取应成功");
        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.attempts_used, 1);
        assert_eq!(server.count(), 2, "初始化 1 次 + 读取 1 次");
    }

    #[test]
    fn p2_https_untrusted_certificate_stops_before_http_request() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let reader = https_entry(https_reader(&server, false), "tenant-a/reader");
        let gate = https_grant(&server);
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_TLS");
        assert_eq!(server.count(), 0, "TLS 握手失败，不产生 HTTP 请求");
    }

    #[test]
    fn p2_https_hostname_mismatch_stops_before_http_request() {
        // 服务端出示 other.test 证书，客户端按 p2-local.test 连接。
        let server = spawn_tls_server(
            TLS_OTHER_CERT,
            TLS_OTHER_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_HOST");
        assert_eq!(server.count(), 0, "主机名不匹配，不产生 HTTP 请求");
    }

    #[test]
    fn p2_https_redirect_is_not_followed_and_target_receives_zero_requests() {
        let target = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let location = format!("https://p2-local.test:{}/steal", target.port);
        let source = spawn_tls_server(TLS_SRV_CERT, TLS_SRV_KEY, ServerMode::Redirect { location });
        let reader = https_entry(https_reader(&source, true), "tenant-a/reader");
        let gate = https_grant(&source);
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_REDIRECT");
        assert_eq!(source.count(), 1, "只发出 1 次请求，不跟随重定向");
        assert_eq!(target.count(), 0, "重定向目标收到 0 次请求");
    }

    #[test]
    fn p2_https_response_with_unapproved_field_blocks_whole_page() {
        let server = spawn_tls_server(TLS_SRV_CERT, TLS_SRV_KEY, ServerMode::JsonOk { leak: true });
        let sink = Arc::new(Mutex::new(Vec::new()));
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader")
            .with_audit_sink(Arc::clone(&sink));
        let gate = https_grant(&server);
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SCOPE");
        // 结果层文案：错误码 + 越界字段名，调用方无需再自行拼装。
        let shown = err.to_string();
        assert!(shown.contains("P2_E_SCOPE"), "提示缺错误码: {shown}");
        assert!(
            shown.contains("响应包含未授权字段 SECRET"),
            "提示未点明越界字段: {shown}"
        );
        // 脱敏审计：一行、带错误码与表号，不出现完整 file_id、字段值或凭据。
        let lines = sink.lock().unwrap().clone();
        assert_eq!(lines.len(), 1, "一次失败只写一行审计：{lines:?}");
        let line = &lines[0];
        assert!(line.contains("action=dbsheet.list_records"), "{line}");
        assert!(line.contains("outcome=P2_E_SCOPE"), "{line}");
        assert!(line.contains("sheet=3"), "{line}");
        assert!(line.contains("attempts=1"), "读取尝试次数应为 1：{line}");
        assert!(line.contains("dbsh…01"), "file_id 应掩码：{line}");
        assert!(!line.contains("dbsheet_replica_001"), "{line}");
        assert!(
            !line.contains("不可见") && !line.contains("SECRET"),
            "{line}"
        );
        assert!(!line.contains("synthetic"), "{line}");
        assert_eq!(
            server.count(),
            2,
            "越界字段在收到响应后拒绝，无后续分页或重试"
        );
    }

    #[test]
    fn p2_https_wrong_target_and_write_action_issue_zero_connections() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");

        let mut wrong = grants(&[(3, &["A"])]);
        wrong.endpoint = server.endpoint();
        wrong.approved_endpoint = server.endpoint();
        wrong.file_id = "dbsheet_replica_999".into();
        let err = reader
            .read(
                &P2Gate::open(wrong),
                &P2ReadRequest {
                    sheet_id: 3,
                    offset: 0,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_TARGET");

        let mut write = grants(&[(3, &["A"])]);
        write.endpoint = server.endpoint();
        write.approved_endpoint = server.endpoint();
        let write_err = write
            .preflight(Utc::now(), P2Action::Write, Some("tenant-a/reader"), 3)
            .unwrap_err();
        assert_eq!(write_err.code(), "P2_E_ACTION");
        assert_eq!(server.count(), 0, "预检拒绝时不建立任何连接");
    }
}
