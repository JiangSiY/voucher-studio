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
use sha2::{Digest, Sha256};

/// 唯一允许的读取动作。
pub const P2_ACTION_LIST_RECORDS: &str = "dbsheet.list_records";

/// 单次读取的传输尝试上限（会话重建不计入次数）。
pub const P2_MAX_READ_ATTEMPTS: usize = 3;

/// 单次授权的执行窗口上限：15 分钟（A2 授权单条款）。
pub const P2_MAX_WINDOW: Duration = Duration::from_secs(15 * 60);

/// 单次授权的整场 HTTP 请求上限：8 次（初始化 + 通知 + 读取 + 终止之和）。
pub const P2_MAX_TOTAL_REQUESTS: u32 = 8;

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
    /// 首屏 `offset` 必须为 0。
    Offset(String),
    /// 请求 `page_size` 与签批值不一致。
    PageSize(String),
    /// 同一份单次授权不能再次启动（已完成或已停止）。
    SingleUse(String),
    /// 执行状态库不可用或登记失败；本地停止，不发网。
    State(String),
    /// 该授权已在持久状态库登记消费，不能再次启动（跨重启/并发有效）。
    AlreadyUsed(String),
    /// 整场请求预算耗尽。
    ExecLimit(String),
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
    /// 扫描运行缺页：任一遍未覆盖授权页集。
    Incomplete(String),
    /// 扫描运行出现授权页集之外的新页（行数漂移）。
    Drift(String),
    /// 两遍扫描指纹不一致（同一次运行内不自洽）。
    Inconsistent(String),
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
            Self::Offset(_) => "P2_E_OFFSET",
            Self::PageSize(_) => "P2_E_PAGE_SIZE",
            Self::SingleUse(_) => "P2_E_SINGLE_USE",
            Self::State(_) => "P2_E_STATE",
            Self::AlreadyUsed(_) => "P2_E_ALREADY_USED",
            Self::ExecLimit(_) => "P2_E_EXEC_LIMIT",
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
            Self::Incomplete(_) => "P2_E_INCOMPLETE",
            Self::Drift(_) => "P2_E_DRIFT",
            Self::Inconsistent(_) => "P2_E_INCONSISTENT",
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
            | Self::Offset(msg)
            | Self::PageSize(msg)
            | Self::SingleUse(msg)
            | Self::State(msg)
            | Self::AlreadyUsed(msg)
            | Self::ExecLimit(msg)
            | Self::ScopePending(msg)
            | Self::Window(msg)
            | Self::Action(msg)
            | Self::Tls(msg)
            | Self::Hostname(msg)
            | Self::Credentials(msg)
            | Self::Malformed(msg)
            | Self::ProtocolPending(msg)
            | Self::Protocol(msg) => write!(f, "{}: {msg}", self.code()),
            Self::Incomplete(msg) => write!(f, "{}: {msg}", self.code()),
            Self::Drift(msg) => write!(f, "{}: {msg}", self.code()),
            Self::Inconsistent(msg) => write!(f, "{}: {msg}", self.code()),
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
    /// 签批授权单的唯一编号；作为持久状态库的主键，跨重启/并发防重。
    pub grant_id: String,
    pub approved_by: Option<String>,
    pub approved_endpoint: P2Endpoint,
    pub endpoint: P2Endpoint,
    pub approved_file_id: String,
    pub file_id: String,
    pub sheets: BTreeMap<i64, BTreeSet<String>>,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub approved_subject: Option<String>,
    /// 签批的首屏 `page_size`；请求值必须一致。
    pub page_size: u32,
    /// 整场执行的 HTTP 请求总上限（初始化 + 读取 + 通知 + 终止）。
    pub max_total_requests: u32,
    /// 是否允许发送会话终止（`DELETE`）。依据不足时由签批人关闭。
    pub terminate_allowed: bool,
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
        if self.grant_id.trim().is_empty() {
            return Err(P2Error::NotApproved);
        }
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
        if self.sheets.is_empty() {
            return Err(P2Error::NotApproved);
        }
        if self.sheets.len() > 1 {
            return Err(P2Error::Scope(
                "单次授权只允许一张表，授权中出现多张表，已拒绝".into(),
            ));
        }
        if !self.sheets.contains_key(&sheet_id) {
            return Err(P2Error::Scope(format!("表 {sheet_id} 不在签批范围内")));
        }
        if self.max_total_requests == 0 {
            return Err(P2Error::ExecLimit(
                "整场请求上限为 0，签批单未绑定预算，已拒绝".into(),
            ));
        }
        if self.max_total_requests > P2_MAX_TOTAL_REQUESTS {
            return Err(P2Error::ExecLimit(format!(
                "整场请求上限 {} 超过 {} 次上限，已拒绝",
                self.max_total_requests, P2_MAX_TOTAL_REQUESTS
            )));
        }
        if self.window_end <= self.window_start {
            return Err(P2Error::Window("执行窗口起止顺序不合法，已拒绝".into()));
        }
        let window = (self.window_end - self.window_start)
            .to_std()
            .map_err(|_| P2Error::Window("执行窗口长度不可用，已拒绝".into()))?;
        if window > P2_MAX_WINDOW {
            return Err(P2Error::Window(format!(
                "执行窗口 {:?} 超过 {:?} 上限，已拒绝",
                window, P2_MAX_WINDOW
            )));
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2ReadRequest {
    pub sheet_id: i64,
    /// 不透明分页游标：首屏传 `None`，下一页传上一页返回的 `cursor`。
    /// 游标由服务端生成、客户端不得自行构造（`2026-11-23` 实测：数值 `0` 与空串语义不同）。
    pub cursor: Option<String>,
    pub page_size: usize,
}

/// 校验后的一页：记录、可选总数、可选下一页游标。
type ValidatedPage = (Vec<Value>, Option<u64>, Option<String>);

/// 读取结果（只保留已授权字段）。
#[derive(Debug, Clone, PartialEq)]
pub struct P2ReadOutcome {
    pub sheet_id: i64,
    pub records: Vec<Value>,
    pub total: Option<u64>,
    /// 服务端游标（`data.detail.offset`）；为空表示没有下一页。
    pub cursor: Option<String>,
    pub attempts_used: usize,
    /// 本轮实际发出的动作计数（初始化/读取/通知/终止/合计）。
    pub actions: P2ActionCounts,
    /// 会话终止的实际状态；失败或未发送都如实记录，不追加请求。
    pub termination: P2Termination,
}

/// 会话终止状态：签审人需要看到「未确认」与「未发送原因」，而不是静默成功。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P2Termination {
    /// 已授权并成功发送 `DELETE`。
    Completed,
    /// 已发送 `DELETE` 但远端未确认（如 5xx/网络失败）；本地已关闭。
    Unconfirmed,
    /// 因窗口/预算/签批开关未发送，附固定原因。
    Skipped(String),
}

impl Default for P2Termination {
    fn default() -> Self {
        Self::Skipped("未执行终止".into())
    }
}

impl P2Termination {
    pub fn message(&self) -> String {
        match self {
            Self::Completed => "远端终止已确认".into(),
            Self::Unconfirmed => "远端终止未确认，本地已关闭".into(),
            Self::Skipped(reason) => format!("未发送终止（{reason}）"),
        }
    }
}

impl std::fmt::Display for P2Termination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

/// 单次 P2 执行的动作计数；用于签审时逐项核对发网次数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct P2ActionCounts {
    pub initialize: u32,
    pub read: u32,
    pub notify: u32,
    pub terminate: u32,
    pub total: u32,
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
    /// `deadline` 由入口按「窗口剩余时间」给出，传输层必须用它作为本次请求的超时上限。
    fn initialize(
        &self,
        endpoint: &P2Endpoint,
        auth: Option<&str>,
        deadline: Duration,
    ) -> Result<String, P2Error>;

    /// 协议必需的 `notifications/initialized`。默认实现不发网（桩传输用）。
    fn notify(
        &self,
        _endpoint: &P2Endpoint,
        _session_id: &str,
        _auth: Option<&str>,
        _deadline: Duration,
    ) -> Result<(), P2Error> {
        Ok(())
    }

    /// 会话终止（`DELETE`）。默认实现不发网；真实传输按授权单开关执行。
    fn terminate(
        &self,
        _endpoint: &P2Endpoint,
        _session_id: &str,
        _auth: Option<&str>,
        _deadline: Duration,
    ) -> Result<(), P2Error> {
        Ok(())
    }

    fn call(
        &self,
        endpoint: &P2Endpoint,
        session_id: &str,
        action: &str,
        args: &Value,
        auth: Option<&str>,
        deadline: Duration,
    ) -> Result<Value, P2Error>;
}

/// 真实 HTTPS 传输的占位实现：会话报文、请求头与服务端只读权限尚无核实依据，
/// 因此任何调用都在本地失败，绝不发网。真实实现由 A2 参数到位后接线。
#[derive(Debug, Default)]
pub struct UnavailableP2Transport;

impl P2Transport for UnavailableP2Transport {
    fn initialize(
        &self,
        _endpoint: &P2Endpoint,
        _auth: Option<&str>,
        _deadline: Duration,
    ) -> Result<String, P2Error> {
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
        _deadline: Duration,
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
pub fn mask_id(value: &str) -> String {
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
    timeout: Duration,
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
        deadline: Duration,
    ) -> Result<reqwest::blocking::Response, P2Error> {
        if endpoint != &self.endpoint {
            return Err(P2Error::Endpoint(
                "传输层端点与授权端点不一致，拒绝请求".into(),
            ));
        }
        let mut request = self
            .client
            .post(endpoint.as_url())
            .timeout(deadline.min(self.timeout))
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

    fn delete(
        &self,
        endpoint: &P2Endpoint,
        session: &str,
        auth: Option<&str>,
        deadline: Duration,
    ) -> Result<reqwest::blocking::Response, P2Error> {
        if endpoint != &self.endpoint {
            return Err(P2Error::Endpoint(
                "传输层端点与授权端点不一致，拒绝请求".into(),
            ));
        }
        let mut request = self
            .client
            .delete(endpoint.as_url())
            .timeout(deadline.min(self.timeout))
            .header("Mcp-Session-Id", session);
        if let Some(token) = auth {
            request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"));
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
        let timeout = self.timeout;
        let client = builder
            .build()
            .map_err(|_| P2Error::Protocol("HTTPS 客户端初始化失败".into()))?;
        Ok(HttpsP2Transport {
            endpoint: self.endpoint,
            client,
            timeout,
        })
    }
}

impl P2Transport for HttpsP2Transport {
    fn initialize(
        &self,
        endpoint: &P2Endpoint,
        auth: Option<&str>,
        deadline: Duration,
    ) -> Result<String, P2Error> {
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
        let response = self.post(endpoint, None, auth, &body, deadline)?;
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
        deadline: Duration,
    ) -> Result<Value, P2Error> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": action, "arguments": args}
        });
        let response = self.post(endpoint, Some(session_id), auth, &body, deadline)?;
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

    fn notify(
        &self,
        endpoint: &P2Endpoint,
        session_id: &str,
        auth: Option<&str>,
        deadline: Duration,
    ) -> Result<(), P2Error> {
        let body = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let response = self.post(endpoint, Some(session_id), auth, &body, deadline)?;
        match classify_http_status(response.status().as_u16()) {
            None => Ok(()),
            Some(err) => Err(err),
        }
    }

    fn terminate(
        &self,
        endpoint: &P2Endpoint,
        session_id: &str,
        auth: Option<&str>,
        deadline: Duration,
    ) -> Result<(), P2Error> {
        let response = self.delete(endpoint, session_id, auth, deadline)?;
        match classify_http_status(response.status().as_u16()) {
            None => Ok(()),
            Some(err) => Err(err),
        }
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

/// 单次授权的消费登记：首次发网前原子登记，跨重启与并发进程有效。
///
/// 实现必须保证同一 `grant_id` 至多成功一次；失败时本地停止，绝不发网。
pub trait P2Consumption: Send + Sync {
    fn consume(
        &self,
        grant_id: &str,
        file_id_masked: &str,
        window_end: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), P2Error>;
}

/// 隔离的 SQLite 执行状态库：只保存防重所需的最小元数据（授权号、登记时间、掩码副本号、窗口终点）。
pub struct SqliteP2State {
    conn: std::sync::Mutex<rusqlite::Connection>,
}

impl SqliteP2State {
    /// 打开（或创建）状态库；失败一律视为「不可用」，入口此后不发网。
    pub fn open(path: &std::path::Path) -> Result<Self, P2Error> {
        let conn = rusqlite::Connection::open(path)
            .map_err(|_| P2Error::State("执行状态库不可用".into()))?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(|_| P2Error::State("执行状态库不可用".into()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS p2_consumed(\
                 grant_id TEXT PRIMARY KEY,\
                 claimed_at TEXT NOT NULL,\
                 file_id_masked TEXT NOT NULL,\
                 window_end TEXT NOT NULL\
             )",
        )
        .map_err(|_| P2Error::State("执行状态库初始化失败".into()))?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }
}

impl P2Consumption for SqliteP2State {
    fn consume(
        &self,
        grant_id: &str,
        file_id_masked: &str,
        window_end: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), P2Error> {
        if grant_id.trim().is_empty() {
            return Err(P2Error::NotApproved);
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| P2Error::State("执行状态库不可用".into()))?;
        match conn.execute(
            "INSERT INTO p2_consumed (grant_id, claimed_at, file_id_masked, window_end) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                grant_id,
                now.to_rfc3339(),
                file_id_masked,
                window_end.to_rfc3339()
            ],
        ) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(P2Error::AlreadyUsed(
                    "该授权已在执行状态库登记消费，不能再次启动".into(),
                ))
            }
            Err(_) => Err(P2Error::State("登记执行状态失败".into())),
        }
    }
}

/// 进程内消费登记：仅用于没有持久库的单元测试；跨重启场景必须用 [`SqliteP2State`]。
#[derive(Default)]
pub struct MemoryConsumption {
    seen: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl P2Consumption for MemoryConsumption {
    fn consume(
        &self,
        grant_id: &str,
        _file_id_masked: &str,
        _window_end: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> Result<(), P2Error> {
        if grant_id.trim().is_empty() {
            return Err(P2Error::NotApproved);
        }
        let mut seen = self
            .seen
            .lock()
            .map_err(|_| P2Error::State("执行状态库不可用".into()))?;
        if !seen.insert(grant_id.to_string()) {
            return Err(P2Error::AlreadyUsed(
                "该授权已在执行状态库登记消费，不能再次启动".into(),
            ));
        }
        Ok(())
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
    /// 已占用的单次授权 key；成功或停止后不可再次启动。
    claimed: std::sync::Mutex<std::collections::HashSet<String>>,
    /// 持久消费登记：配置后必须在发网前登记成功，跨重启/并发有效。
    state: Option<Arc<dyn P2Consumption>>,
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
            claimed: std::sync::Mutex::new(std::collections::HashSet::new()),
            state: None,
        }
    }

    /// 接入隔离执行状态库；未接入时只做进程内防重（重启后可再次启动）。
    pub fn with_state(mut self, state: Arc<dyn P2Consumption>) -> Self {
        self.state = Some(state);
        self
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

    /// 单次执行入口：绑定一份授权、一张表、一页（首屏或指定游标）、签批 `page_size` 与整场请求上限。
    /// 预检通过即占用该授权；成功结束或触发停止条件后不能再次启动。
    pub fn read(&self, gate: &P2Gate, request: &P2ReadRequest) -> Result<P2ReadOutcome, P2Error> {
        let grant = gate.grant()?;
        if request.page_size == 0 || request.page_size != grant.page_size as usize {
            return Err(P2Error::PageSize(format!(
                "请求 page_size={} 与签批值 {} 不一致，已拒绝",
                request.page_size, grant.page_size
            )));
        }
        let subject = self.credential.as_ref().and_then(|c| c.subject());
        grant.preflight(
            self.clock.now(),
            P2Action::ListRecords,
            subject.as_deref(),
            request.sheet_id,
        )?;
        self.claim(grant)?;
        // 持久登记必须发生在任何发网动作之前：登记失败即本地停止，零请求。
        if let Some(state) = &self.state {
            state.consume(
                &grant.grant_id,
                &mask_id(&grant.file_id),
                grant.window_end,
                self.clock.now(),
            )?;
        }

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
        // SkillHub 的 `offset` 是**不透明字符串游标**（首屏传空串，下一页传响应里的
        // `data.detail.offset`）。此前的数值 `0` 会被服务端当成另一种语义，实测只回了
        // 首页一部分记录（2026-11-23 第二发 27 条 vs 插件同参数 50 条）。
        let args = json!({
            "file_id": grant.file_id,
            "sheet_id": request.sheet_id,
            "offset": String::from(request.cursor.as_deref().unwrap_or("")),
            "page_size": request.page_size,
            "fields": fields,
        });

        let mut counts = P2ActionCounts::default();
        let mut session = self.start_session(grant, token.as_deref(), &mut counts)?;
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
            // 每次发网前核对窗口剩余时间与整场请求上限；超时上限即剩余窗口。
            let deadline = self.reserve(grant, &mut counts)?;
            sent += 1;
            counts.read += 1;
            match self.transport.call(
                &grant.endpoint,
                &session,
                P2_ACTION_LIST_RECORDS,
                &args,
                token.as_deref(),
                deadline,
            ) {
                Ok(value) => {
                    let (records, total, cursor) =
                        match self.validate_records(grant, request.sheet_id, &value) {
                            Ok(validated) => validated,
                            Err(err) => {
                                // 响应越界也要留审计：只记错误码与脱敏范围，不记字段值。
                                self.audit(grant, request.sheet_id, err.code(), sent);
                                return Err(err);
                            }
                        };
                    self.audit(grant, request.sheet_id, "ok", sent);
                    let termination = if grant.terminate_allowed {
                        // 结束后终止会话；失败只做本地关闭，不再发网、不追加请求。
                        match self.reserve(grant, &mut counts) {
                            Ok(deadline) => {
                                counts.terminate += 1;
                                match self.transport.terminate(
                                    &grant.endpoint,
                                    &session,
                                    token.as_deref(),
                                    deadline,
                                ) {
                                    Ok(()) => P2Termination::Completed,
                                    Err(err) => {
                                        self.audit(
                                            grant,
                                            request.sheet_id,
                                            "P2_E_TERMINATE",
                                            counts.terminate as usize,
                                        );
                                        let _ = err;
                                        P2Termination::Unconfirmed
                                    }
                                }
                            }
                            Err(err) => {
                                self.audit(
                                    grant,
                                    request.sheet_id,
                                    err.code(),
                                    counts.terminate as usize,
                                );
                                P2Termination::Skipped(err.to_string())
                            }
                        }
                    } else {
                        P2Termination::Skipped("授权单未允许终止动作".into())
                    };
                    return Ok(P2ReadOutcome {
                        sheet_id: request.sheet_id,
                        records,
                        total,
                        cursor,
                        attempts_used: sent,
                        actions: counts,
                        termination,
                    });
                }
                Err(P2Error::SessionExpired) => {
                    // 会话重建最多一次；读取额度已用尽时不再重建。
                    if reinitialized || sent >= self.max_attempts {
                        self.audit(grant, request.sheet_id, "P2_E_SESSION", sent);
                        return Err(P2Error::SessionExpired);
                    }
                    reinitialized = true;
                    session = self.start_session(grant, token.as_deref(), &mut counts)?;
                    last_error = Some(P2Error::SessionExpired);
                }
                Err(err) if err.is_retryable() => {
                    last_error = Some(err);
                    if sent < self.max_attempts {
                        // 退避不得超过窗口剩余时间；剩余不足则直接停止，不再发网。
                        let remain = self.remaining(grant)?;
                        if remain <= self.backoff {
                            self.audit(grant, request.sheet_id, "P2_E_WINDOW", sent);
                            return Err(P2Error::Window("执行窗口在退避期间到期，已停止".into()));
                        }
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

    /// 占用单次授权：同一 key 只能成功一次，成功或停止后不可重启。
    fn claim(&self, grant: &P2Grant) -> Result<(), P2Error> {
        let approver = grant.approved_by.as_deref().ok_or(P2Error::NotApproved)?;
        let key = format!(
            "{approver}|{}|{}",
            grant.file_id,
            grant.window_start.to_rfc3339()
        );
        let mut claimed = self
            .claimed
            .lock()
            .map_err(|_| P2Error::SingleUse("授权占用状态不可用，拒绝启动".into()))?;
        if !claimed.insert(key) {
            return Err(P2Error::SingleUse(
                "该授权已完成或已停止，不能再次启动".into(),
            ));
        }
        Ok(())
    }

    /// 窗口剩余时间；到期即拒绝继续发网。
    fn remaining(&self, grant: &P2Grant) -> Result<Duration, P2Error> {
        let now = self.clock.now();
        if now >= grant.window_end {
            return Err(P2Error::Window("执行窗口已到期，拒绝继续发网".into()));
        }
        (grant.window_end - now)
            .to_std()
            .map_err(|_| P2Error::Window("执行窗口剩余时间不可用".into()))
    }

    /// 消耗一次整场请求预算，返回本次请求允许的最长耗时（不超过窗口剩余时间）。
    fn reserve(&self, grant: &P2Grant, counts: &mut P2ActionCounts) -> Result<Duration, P2Error> {
        let remain = self.remaining(grant)?;
        if counts.total >= grant.max_total_requests {
            return Err(P2Error::ExecLimit(format!(
                "整场请求已达上限 {} 次，已停止",
                grant.max_total_requests
            )));
        }
        counts.total += 1;
        Ok(remain)
    }

    /// 建立（或重建）会话并发一次协议通知；初始化上限 2 次、通知上限 2 次。
    fn start_session(
        &self,
        grant: &P2Grant,
        token: Option<&str>,
        counts: &mut P2ActionCounts,
    ) -> Result<String, P2Error> {
        if counts.initialize >= 2 {
            return Err(P2Error::ExecLimit(
                "会话初始化已达上限（初次 + 最多一次重建），已停止".into(),
            ));
        }
        let deadline = self.reserve(grant, counts)?;
        counts.initialize += 1;
        let session = self
            .transport
            .initialize(&grant.endpoint, token, deadline)?;
        if counts.notify < 2 {
            let deadline = self.reserve(grant, counts)?;
            counts.notify += 1;
            self.transport
                .notify(&grant.endpoint, &session, token, deadline)?;
        }
        Ok(session)
    }

    fn validate_records(
        &self,
        grant: &P2Grant,
        sheet_id: i64,
        value: &Value,
    ) -> Result<ValidatedPage, P2Error> {
        let allowed = grant.fields_for(sheet_id)?;
        // 真实端点的 `tools/call` 文本可能是 WPS 信封（`{code,message,data:{detail:{records}}}`），
        // 也可能是 `data.records` 或裸 `records`；三种都接受，其余一律按形状不符拒绝。
        let payload = Self::records_payload(value)
            .ok_or_else(|| P2Error::Malformed("响应缺少 records 数组".into()))?;
        let records = payload
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
                    // 只给固定提示与表号：不回显远端字段名/字段值。
                    return Err(P2Error::Scope(format!(
                        "响应包含未授权字段，整页已拒绝（表 {sheet_id}）"
                    )));
                }
                filtered.insert(key.clone(), item.clone());
            }
            kept.push(Value::Object(filtered));
        }
        let total = payload
            .get("total")
            .or_else(|| value.get("total"))
            .and_then(Value::as_u64);
        let cursor = payload
            .get("offset")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty())
            .map(str::to_string);
        Ok((kept, total, cursor))
    }

    /// 找到承载 `records` 的那一层：裸响应、`data.records`、或 WPS 信封的 `data.detail.records`。
    fn records_payload(value: &Value) -> Option<&Value> {
        if value.get("records").is_some_and(Value::is_array) {
            return Some(value);
        }
        let data = value.get("data")?;
        if let Some(detail) = data.get("detail") {
            if detail.get("records").is_some_and(Value::is_array) {
                return Some(detail);
            }
        }
        if data.get("records").is_some_and(Value::is_array) {
            return Some(data);
        }
        None
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

// ===== 扫描运行（scan-run，2026-11-24 拟议 v0.3 的实现） =====
//
// 设计：`docs/验收基线/证据/2026-11-19-S0/A2扫描级授权-设计.md` §2–§8。
// 与单页单的差别：一次授权覆盖多表、多页、两遍扫描；`grant_id` 只在运行开始登记一次；
// 不提供外部游标；页数锁死实测值，出现新页即 `P2_E_DRIFT`。

/// 扫描运行的全局请求硬上限（会话 + 读取）：设计 §3 的 80。
pub const P2_SCAN_MAX_TOTAL_REQUESTS: u32 = 80;
/// 扫描运行的读取硬上限：两遍必达 44 + 最多 30 次重试。
pub const P2_SCAN_MAX_READS: u32 = 74;
/// 会话动作硬上限（初始化/重建 + 通知 + 终止）。
pub const P2_SCAN_MAX_SESSION_ACTIONS: u32 = 6;

/// 一次扫描运行的授权规格；构造后须先 `validate()`。
#[derive(Debug, Clone)]
pub struct P2ScanSpec {
    pub endpoint: P2Endpoint,
    pub file_id: String,
    /// 签批目标：请求的 `file_id` 必须与它**完全一致**（Q2 约束①，零请求拒绝）。
    /// 与 `P2Grant::approved_file_id` 同义；扫描单不得只写一个 `file_id` 自证。
    pub approved_file_id: String,
    pub grant_id: String,
    pub approved_by: String,
    /// 签批授权主体（账号/租户）。缺失即 `P2_E_SCOPE_PENDING`，不一致即 `P2_E_SCOPE`；
    /// 与单页单 `P2Grant::preflight` 同规则，且在运行级消费**之前**核对（Q2 约束⑤）。
    pub approved_subject: Option<String>,
    pub sheets: BTreeMap<i64, BTreeSet<String>>,
    /// 每表锁死的页数（签批前 24h 内实测）。
    pub pages: BTreeMap<i64, u32>,
    pub page_size: u32,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub max_reads: u32,
    pub max_total: u32,
    pub terminate_allowed: bool,
}

impl P2ScanSpec {
    /// 返回两遍扫描的必达读取数：`2 × Σ pages`。
    pub fn required_reads(&self) -> u32 {
        self.pages.values().copied().sum::<u32>().saturating_mul(2)
    }

    pub fn validate(&self) -> Result<(), P2Error> {
        if self.grant_id.trim().is_empty() || self.approved_by.trim().is_empty() {
            return Err(P2Error::NotApproved);
        }
        if self.file_id != self.approved_file_id {
            return Err(P2Error::Target(
                "扫描目标 file_id 与签批不一致，已拒绝".into(),
            ));
        }
        if self.sheets.is_empty() {
            return Err(P2Error::Scope("扫描运行未列出任何表".into()));
        }
        if self.page_size == 0 {
            return Err(P2Error::PageSize("扫描运行 page_size 不能为 0".into()));
        }
        let sheet_keys: BTreeSet<i64> = self.sheets.keys().copied().collect();
        let page_keys: BTreeSet<i64> = self.pages.keys().copied().collect();
        if sheet_keys != page_keys {
            return Err(P2Error::Scope(
                "页数清单与表清单不一致：每张表都必须锁死页数".into(),
            ));
        }
        if self.sheets.values().any(BTreeSet::is_empty) {
            return Err(P2Error::Scope("每张表都必须逐列批准字段".into()));
        }
        if self.pages.values().any(|pages| *pages == 0) {
            return Err(P2Error::ExecLimit("页数不能为 0".into()));
        }
        if self.window_end <= self.window_start {
            return Err(P2Error::Window("执行窗口终止时刻必须晚于起点".into()));
        }
        let window = (self.window_end - self.window_start)
            .to_std()
            .map_err(|_| P2Error::Window("执行窗口长度不合法".into()))?;
        if window > P2_MAX_WINDOW {
            return Err(P2Error::Window(format!(
                "执行窗口超过 {:?} 上限，已拒绝",
                P2_MAX_WINDOW
            )));
        }
        let required = self.required_reads();
        if self.max_reads < required {
            return Err(P2Error::ExecLimit(format!(
                "读取上限 {} 低于两遍必达页数 {}",
                self.max_reads, required
            )));
        }
        if self.max_reads > P2_SCAN_MAX_READS {
            return Err(P2Error::ExecLimit(format!(
                "读取上限 {} 超过 {} 上限",
                self.max_reads, P2_SCAN_MAX_READS
            )));
        }
        let session_budget = self.max_total.saturating_sub(self.max_reads);
        if session_budget < 3 || self.max_total > P2_SCAN_MAX_TOTAL_REQUESTS {
            return Err(P2Error::ExecLimit(format!(
                "整场上限 {} 不成立：需 ≤{} 且为会话留 ≥3 次",
                self.max_total, P2_SCAN_MAX_TOTAL_REQUESTS
            )));
        }
        Ok(())
    }

    /// Q2 主体约束：签批主体必须已声明，且凭据主体必须与之一致。
    ///
    /// 与 `P2Grant::preflight` 的判定同源，差别只在于扫描运行没有单表概念。
    /// **必须在 `P2Consumption::consume` 之前调用**：本地可判的拒绝不得烧掉一次性授权。
    pub fn check_subject(&self, credential_subject: Option<&str>) -> Result<(), P2Error> {
        let approved_subject = self
            .approved_subject
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                P2Error::ScopePending("扫描授权主体（账号/租户）待补，不能视为已授权".into())
            })?;
        match credential_subject.map(str::trim) {
            Some(subject) if subject == approved_subject => Ok(()),
            Some(_) => Err(P2Error::Scope(
                "凭据主体与扫描签批主体不一致，已拒绝".into(),
            )),
            None => Err(P2Error::ScopePending(
                "凭据未提供可核对的主体，已拒绝".into(),
            )),
        }
    }
}

/// 一张表一遍扫描的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2SheetScan {
    pub sheet_id: i64,
    pub pages_read: u32,
    pub records: u64,
    /// 逐表指纹：`sha256("id|规范化字段 JSON" 行)` 的前 16 位十六进制。
    pub digest: String,
}

/// 扫描运行结果。
#[derive(Debug, Clone, PartialEq)]
pub struct P2ScanOutcome {
    pub pass1: Vec<P2SheetScan>,
    pub pass2: Vec<P2SheetScan>,
    pub stable: bool,
    pub reads: u32,
    pub actions: P2ActionCounts,
    pub termination: P2Termination,
}

/// 扫描运行入口：复用传输、凭据、状态库与审计，但**不复用**单页单的一次性 claim。
/// 扫描审计的入参集合（避免超长参数列表）。
#[derive(Debug, Clone, Copy)]
struct ScanAuditEntry<'a, 'b> {
    pass: u32,
    sheet_id: i64,
    page: u32,
    records: u64,
    cursor: Option<&'a str>,
    outcome: &'b str,
    counts: P2ActionCounts,
}

pub struct P2ScanRun<T: P2Transport> {
    transport: T,
    credential: Option<Arc<dyn P2Credential>>,
    clock: Arc<dyn P2Clock>,
    state: Option<Arc<dyn P2Consumption>>,
    audit_sink: Option<Arc<std::sync::Mutex<Vec<String>>>>,
}

impl<T: P2Transport> P2ScanRun<T> {
    pub fn new(transport: T, credential: Option<Arc<dyn P2Credential>>) -> Self {
        Self {
            transport,
            credential,
            clock: Arc::new(SystemP2Clock),
            state: None,
            audit_sink: None,
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn P2Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_state(mut self, state: Arc<dyn P2Consumption>) -> Self {
        self.state = Some(state);
        self
    }

    pub fn with_audit_sink(mut self, sink: Arc<std::sync::Mutex<Vec<String>>>) -> Self {
        self.audit_sink = Some(sink);
        self
    }

    pub fn run(&self, spec: &P2ScanSpec) -> Result<P2ScanOutcome, P2Error> {
        spec.validate()?;
        let now = self.clock.now();
        if now < spec.window_start || now >= spec.window_end {
            return Err(P2Error::Window("不在签批执行窗口内".into()));
        }
        // Q2 约束①⑤：目标绑定与主体核对全部在本地做完，**再**登记一次性消费。
        // 顺序即证据：被拒的运行（目标/主体/窗口不成立）不产生消费记录，也不发网。
        let credential_subject = self
            .credential
            .as_ref()
            .and_then(|credential| credential.subject());
        spec.check_subject(credential_subject.as_deref())?;
        self.audit_bind(spec, credential_subject.as_deref());
        // 运行级消费：只在整场开始登记一次，运行内的 44+ 次读取不再登记。
        if let Some(state) = &self.state {
            state.consume(
                &spec.grant_id,
                &mask_id(&spec.file_id),
                spec.window_end,
                now,
            )?;
        }
        let token = match &self.credential {
            Some(credential) => {
                let token = credential.access_token()?;
                if token.trim().is_empty() {
                    return Err(P2Error::Credentials("凭据为空".into()));
                }
                Some(token)
            }
            None => None,
        };

        let mut counts = P2ActionCounts::default();
        let mut reads = 0u32;
        let deadline = self.remaining(spec)?;
        counts.initialize += 1;
        counts.total += 1;
        let mut session = self
            .transport
            .initialize(&spec.endpoint, token.as_deref(), deadline)?;
        counts.notify += 1;
        counts.total += 1;
        let deadline = self.remaining(spec)?;
        self.transport
            .notify(&spec.endpoint, &session, token.as_deref(), deadline)?;

        let mut pass1 = Vec::new();
        let mut pass2 = Vec::new();
        for pass in 1..=2u32 {
            let mut results = Vec::new();
            for (sheet_id, fields) in &spec.sheets {
                let pages = *spec
                    .pages
                    .get(sheet_id)
                    .ok_or_else(|| P2Error::Scope(format!("表 {sheet_id} 缺少页数")))?;
                let mut cursor: Option<String> = None;
                let mut hasher = Sha256::new();
                let mut records = 0u64;
                for page in 1..=pages {
                    let deadline = self.remaining(spec)?;
                    self.check_budget(spec, reads, counts.total)?;
                    let args = json!({
                        "file_id": spec.file_id,
                        "sheet_id": sheet_id,
                        "offset": String::from(cursor.as_deref().unwrap_or("")),
                        "page_size": spec.page_size,
                        "fields": fields.iter().cloned().collect::<Vec<_>>(),
                    });
                    counts.read += 1;
                    counts.total += 1;
                    reads += 1;
                    let value = self.transport.call(
                        &spec.endpoint,
                        &session,
                        P2_ACTION_LIST_RECORDS,
                        &args,
                        token.as_deref(),
                        deadline,
                    )?;
                    let (kept, next) = validate_scan_page(*sheet_id, fields, &value)?;
                    for record in &kept {
                        let id = record
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("row-{records}"));
                        let fields_value = record
                            .get("fields")
                            .cloned()
                            .unwrap_or_else(|| record.clone());
                        hasher.update(id.as_bytes());
                        hasher.update(b"|");
                        hasher.update(canonical_json(&fields_value).as_bytes());
                        hasher.update(b"\n");
                        records += 1;
                    }
                    if page == pages {
                        if let Some(next) = next {
                            return Err(P2Error::Drift(format!(
                                "表 {sheet_id} 出现授权页集之外的新页（游标 {next}）"
                            )));
                        }
                    } else if next.is_none() {
                        return Err(P2Error::Incomplete(format!(
                            "表 {sheet_id} 第 {page} 页之后无游标，未覆盖授权页集（{pages} 页）"
                        )));
                    } else {
                        cursor = next;
                    }
                    self.audit_line(ScanAuditEntry {
                        pass,
                        sheet_id: *sheet_id,
                        page,
                        records,
                        cursor: cursor.as_deref(),
                        outcome: "ok",
                        counts,
                    });
                }
                results.push(P2SheetScan {
                    sheet_id: *sheet_id,
                    pages_read: pages,
                    records,
                    digest: hex16(hasher.finalize()),
                });
            }
            if pass == 1 {
                pass1 = results;
            } else {
                pass2 = results;
            }
        }

        let stable = pass1 == pass2;
        if !stable {
            let changed: Vec<i64> = pass1
                .iter()
                .zip(pass2.iter())
                .filter(|(a, b)| a != b)
                .map(|(a, _)| a.sheet_id)
                .collect();
            return Err(P2Error::Inconsistent(format!(
                "两遍扫描指纹不一致（表 {:?}），运行标 inconsistent",
                changed
            )));
        }

        let mut termination = P2Termination::Skipped("授权单未允许终止动作".into());
        if spec.terminate_allowed {
            let deadline = self.remaining(spec)?;
            if counts.total < spec.max_total {
                counts.terminate += 1;
                counts.total += 1;
                termination = match self.transport.terminate(
                    &spec.endpoint,
                    &session,
                    token.as_deref(),
                    deadline,
                ) {
                    Ok(()) => P2Termination::Completed,
                    Err(_) => P2Termination::Unconfirmed,
                };
            } else {
                termination = P2Termination::Skipped(
                    P2Error::ExecLimit("整场预算不足，未发送终止".into()).to_string(),
                );
            }
        }
        session.clear();

        Ok(P2ScanOutcome {
            pass1,
            pass2,
            stable,
            reads,
            actions: counts,
            termination,
        })
    }

    fn check_budget(&self, spec: &P2ScanSpec, reads: u32, total: u32) -> Result<(), P2Error> {
        if reads >= spec.max_reads {
            return Err(P2Error::ExecLimit(format!(
                "读取已达上限 {} 次，已停止",
                spec.max_reads
            )));
        }
        if total >= spec.max_total {
            return Err(P2Error::ExecLimit(format!(
                "整场请求已达上限 {} 次，已停止",
                spec.max_total
            )));
        }
        Ok(())
    }

    fn remaining(&self, spec: &P2ScanSpec) -> Result<Duration, P2Error> {
        let now = self.clock.now();
        if now >= spec.window_end {
            return Err(P2Error::Window("执行窗口已到期，拒绝继续发网".into()));
        }
        (spec.window_end - now)
            .to_std()
            .map_err(|_| P2Error::Window("执行窗口剩余时间不合法".into()))
    }

    /// 绑定审计：目标与主体在发网前定稿，供卷宗复核「这一次运行绑定到了什么」。
    fn audit_bind(&self, spec: &P2ScanSpec, credential_subject: Option<&str>) {
        let line = format!(
            "p2-scan-bind grant={} target={} subject={} subject_state={}",
            spec.grant_id,
            mask_id(&spec.file_id),
            credential_subject
                .map(mask_id)
                .unwrap_or_else(|| "-".to_string()),
            if credential_subject.is_some() {
                "matched"
            } else {
                "missing"
            },
        );
        if let Some(sink) = &self.audit_sink {
            if let Ok(mut lines) = sink.lock() {
                lines.push(line.clone());
            }
        }
        log::info!("{line}");
    }

    fn audit_line(&self, entry: ScanAuditEntry<'_, '_>) {
        let ScanAuditEntry {
            pass,
            sheet_id,
            page,
            records,
            cursor,
            outcome,
            counts,
        } = entry;
        let cursor_hash = cursor
            .map(|cursor| hex16(Sha256::digest(cursor.as_bytes())))
            .unwrap_or_else(|| "-".into());
        let line = format!(
            "p2-scan pass={pass} sheet={sheet_id} page={page} rows={records} cursor={cursor_hash} outcome={outcome} init={} notify={} read={} term={} total={}",
            counts.initialize, counts.notify, counts.read, counts.terminate, counts.total
        );
        if let Some(sink) = &self.audit_sink {
            if let Ok(mut lines) = sink.lock() {
                lines.push(line.clone());
            }
        }
        log::info!("{line}");
    }
}

/// 扫描页校验：返回（仅授权字段的记录、下一页游标）。记录保留 `id` 供指纹使用。
fn validate_scan_page(
    sheet_id: i64,
    fields: &BTreeSet<String>,
    value: &Value,
) -> Result<(Vec<Value>, Option<String>), P2Error> {
    let payload = P2ReadOnlyEntry::<UnavailableP2Transport>::records_payload(value)
        .ok_or_else(|| P2Error::Malformed("响应缺少 records 数组".into()))?;
    let records = payload
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| P2Error::Malformed("响应缺少 records 数组".into()))?;
    let mut kept = Vec::with_capacity(records.len());
    for record in records {
        let object = record
            .as_object()
            .ok_or_else(|| P2Error::Malformed("records 元素不是对象".into()))?;
        let fields_object = object
            .get("fields")
            .and_then(Value::as_object)
            .or(Some(object))
            .ok_or_else(|| P2Error::Malformed("记录缺少 fields 对象".into()))?;
        let mut filtered = Map::new();
        for (key, item) in fields_object {
            if key == "id" || key == "record_id" {
                continue;
            }
            if !fields.contains(key) {
                return Err(P2Error::Scope(format!(
                    "响应包含未授权字段，整页已拒绝（表 {sheet_id}）"
                )));
            }
            filtered.insert(key.clone(), item.clone());
        }
        if let Some(id) = object.get("id").cloned() {
            filtered.insert("id".into(), id);
        }
        kept.push(Value::Object(filtered));
    }
    let cursor = payload
        .get("offset")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_string);
    Ok((kept, cursor))
}

fn canonical_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| String::from("<invalid>"))
}

fn hex16(bytes: impl AsRef<[u8]>) -> String {
    let digest = bytes.as_ref();
    let mut out = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
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
            grant_id: "A2-2026-11-23-01".into(),
            approved_by: Some("A2-2026-11-22".into()),
            approved_endpoint: endpoint("https://wps.example.test/mcp"),
            endpoint: endpoint("https://wps.example.test/mcp"),
            approved_file_id: "dbsheet_replica_001".into(),
            file_id: "dbsheet_replica_001".into(),
            page_size: 50,
            max_total_requests: 8,
            terminate_allowed: true,
            sheets: map,
            window_start: DateTime::parse_from_rfc3339("2026-11-22T10:25:00Z")
                .unwrap()
                .with_timezone(&Utc),
            window_end: DateTime::parse_from_rfc3339("2026-11-22T10:40:00Z")
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
        fn initialize(
            &self,
            endpoint: &P2Endpoint,
            auth: Option<&str>,
            _deadline: Duration,
        ) -> Result<String, P2Error> {
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
            _deadline: Duration,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SERVER");
        assert_eq!(reader.transport.calls.lock().unwrap().len(), 3);
    }

    #[test]
    fn p2_https_wps_envelope_cursor_is_reported() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::WpsEnvelope { leak: false },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);
        let outcome = reader.read(&gate, &read_req()).expect("读取应成功");
        assert_eq!(outcome.cursor.as_deref(), Some("VW"));
        // 服务端收到的读取请求里，`offset` 必须是空串而不是数字 0。
        let bodies = server.bodies();
        let read_body = bodies
            .iter()
            .find(|body| body.contains("tools/call"))
            .expect("应记录到 list_records 请求体");
        assert!(read_body.contains("\"offset\":\"\""), "{read_body}");
    }

    #[test]
    fn p2_records_payload_accepts_bare_data_records() {
        // 真实端点还有 `data.records` 这一层；三种形状都要能定位 records。
        let value = serde_json::json!({"data":{"records":[{"fields":{"A":"一"}}]}});
        let payload =
            P2ReadOnlyEntry::<UnavailableP2Transport>::records_payload(&value).expect("payload");
        assert_eq!(
            payload["records"][0]["fields"]["A"],
            serde_json::json!("一")
        );
    }

    #[test]
    fn p2_https_wps_envelope_records_are_unwrapped() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::WpsEnvelope { leak: false },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);
        let outcome = reader
            .read(&gate, &read_req())
            .expect("WPS 信封形状应可读取");
        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.records[0]["A"], serde_json::json!("一"));
        assert_eq!(outcome.attempts_used, 1);
    }

    #[test]
    fn p2_https_wps_envelope_scope_violation_blocks_whole_page() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::WpsEnvelope { leak: true },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);
        let err = reader.read(&gate, &read_req()).expect_err("越界应整页拒绝");
        assert_eq!(err.code(), "P2_E_SCOPE");
        let text = err.to_string();
        assert!(text.contains("表 3"), "{text}");
        assert!(!text.contains("SECRET"), "{text}");
        assert!(!text.contains("不可见"), "{text}");
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
                    cursor: None,
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
        /// 读取请求按脚本依次返回状态码（200 时走正常负载）；脚本为空后按 200 返回。
        Script {
            statuses: Arc<Mutex<Vec<u16>>>,
            leak: bool,
        },
        /// `DELETE`（会话终止）返回给定状态码；其余请求照常。
        DeleteFail { status: u16 },
        /// 返回真实端点的 WPS 信封形状：`{code,message,data:{detail:{records,offset}}}`
        WpsEnvelope { leak: bool },
    }

    struct TlsServer {
        port: u16,
        requests: Arc<Mutex<Vec<String>>>,
        bodies: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
    }

    impl TlsServer {
        fn bodies(&self) -> Vec<String> {
            self.bodies.lock().unwrap().clone()
        }
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
        let bodies = Arc::new(Mutex::new(Vec::new()));
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
        let bodies_for_thread = Arc::clone(&bodies);
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
                    let bodies = Arc::clone(&bodies_for_thread);
                    let mode = mode.clone();
                    std::thread::spawn(move || {
                        handle_tls_connection(tcp, config, mode, requests, bodies);
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
            bodies,
            stop,
        }
    }

    fn handle_tls_connection(
        tcp: std::net::TcpStream,
        config: Arc<rustls::ServerConfig>,
        mode: ServerMode,
        requests: Arc<Mutex<Vec<String>>>,
        bodies: Arc<Mutex<Vec<String>>>,
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
        bodies.lock().unwrap().push(text.clone());

        let initialize = text.contains("\"method\":\"initialize\"");
        let is_delete = first_line.starts_with("DELETE ");
        let (status, headers, body) = match &mode {
            ServerMode::DeleteFail { status } if is_delete => (*status, Vec::new(), String::new()),
            ServerMode::DeleteFail { .. } => (
                200,
                if initialize {
                    vec![("Mcp-Session-Id".to_string(), "sess-1".to_string())]
                } else {
                    Vec::new()
                },
                if initialize {
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\"}}"
                        .to_string()
                } else {
                    let payload = "{\"records\":[{\"A\":\"一\"}],\"total\":1}";
                    let literal = serde_json::to_string(payload).expect("payload literal");
                    format!(
                        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":{literal}}}],\"isError\":false}}}}"
                    )
                },
            ),
            ServerMode::Script { statuses, leak } if text.contains("\"method\":\"tools/call\"") => {
                let next = statuses.lock().unwrap().pop().unwrap_or(200);
                if next == 200 {
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
                } else {
                    (next, Vec::new(), String::new())
                }
            }
            ServerMode::Script { .. } => (
                200,
                vec![("Mcp-Session-Id".to_string(), "sess-1".to_string())],
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\"}}"
                    .to_string(),
            ),
            ServerMode::Redirect { location } => (
                302u16,
                vec![("Location".to_string(), location.clone())],
                String::new(),
            ),
            ServerMode::WpsEnvelope { leak } => {
                if initialize {
                    (
                        200,
                        vec![("Mcp-Session-Id".to_string(), "sess-1".to_string())],
                        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\"}}"
                            .to_string(),
                    )
                } else {
                    let mut fields = String::from("{\"A\":\"一\"");
                    if *leak {
                        fields.push_str(",\"SECRET\":\"不可见\"");
                    }
                    fields.push('}');
                    let payload = format!(
                        "{{\"code\":0,\"message\":\"成功\",\"data\":{{\"detail\":{{\"offset\":\"VW\",\"records\":[{{\"fields\":{fields},\"id\":\"REC1\"}}]}},\"result\":\"ok\"}}}}"
                    );
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
                    cursor: None,
                    page_size: 50,
                },
            )
            .expect("合法读取应成功");
        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.attempts_used, 1);
        // 单次执行的动作计数：初始化 1 + 通知 1 + 读取 1 + 终止 1。
        assert_eq!(
            outcome.actions,
            P2ActionCounts {
                initialize: 1,
                read: 1,
                notify: 1,
                terminate: 1,
                total: 4
            }
        );
        assert_eq!(server.count(), 4, "初始化 + 通知 + 读取 + 终止");
        assert_eq!(
            outcome.termination,
            P2Termination::Completed,
            "成功发送 DELETE 应记为已确认"
        );
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
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
                    cursor: None,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SCOPE");
        // 结果层文案：只给固定提示、错误码与表号；不回显远端字段名。
        let shown = err.to_string();
        assert!(shown.contains("P2_E_SCOPE"), "提示缺错误码: {shown}");
        assert!(
            shown.contains("响应包含未授权字段"),
            "提示缺固定文案: {shown}"
        );
        assert!(shown.contains("表 3"), "提示缺表号: {shown}");
        assert!(
            !shown.contains("SECRET") && !shown.contains("不可见"),
            "结果层不得回显远端字段名/值: {shown}"
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
            3,
            "初始化 + 通知 + 读取；越界后拒绝且无后续分页、重试或终止"
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
                    cursor: None,
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

    fn fixed_clock(at: &str) -> Arc<FixedClock> {
        Arc::new(FixedClock(
            DateTime::parse_from_rfc3339(at)
                .unwrap()
                .with_timezone(&Utc),
        ))
    }

    fn entry_with_backoff(
        transport: HttpsP2Transport,
        subject: &str,
        backoff: Duration,
    ) -> P2ReadOnlyEntry<HttpsP2Transport> {
        P2ReadOnlyEntry::new(
            transport,
            Some(Arc::new(SyntheticCredential {
                subject: Some(subject.to_string()),
            })),
        )
        .with_clock_and_backoff(
            fixed_clock("2026-11-22T10:30:00Z"),
            backoff,
            P2_MAX_READ_ATTEMPTS,
        )
    }

    #[test]
    fn p2_https_unapproved_page_size_makes_zero_requests_and_cursor_is_forwarded() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);

        let size_err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    cursor: None,
                    page_size: 20,
                },
            )
            .unwrap_err();
        assert_eq!(size_err.code(), "P2_E_PAGE_SIZE");
        assert_eq!(server.count(), 0, "page_size 与签批值不符时零请求");

        let ok = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    cursor: Some("Cj".to_string()),
                    page_size: 50,
                },
            )
            .expect("带游标的第二页应被允许");
        assert!(!ok.records.is_empty(), "第二页应返回记录");
        let call = server
            .bodies()
            .into_iter()
            .find(|body| body.contains("tools/call"))
            .expect("至少一次 list_records 调用");
        assert!(
            call.contains("\"offset\":\"Cj\""),
            "游标必须原样进入请求体，实际: {call}"
        );
    }

    #[test]
    fn p2_https_window_expiry_during_backoff_stops_without_next_request() {
        let statuses = Arc::new(Mutex::new(vec![500u16]));
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::Script {
                statuses: Arc::clone(&statuses),
                leak: false,
            },
        );
        let reader = entry_with_backoff(
            https_reader(&server, true),
            "tenant-a/reader",
            Duration::from_secs(5),
        );
        let mut grant = grants(&[(3, &["A"])]);
        grant.endpoint = server.endpoint();
        grant.approved_endpoint = server.endpoint();
        grant.window_end = DateTime::parse_from_rfc3339("2026-11-22T10:30:00.100Z")
            .unwrap()
            .with_timezone(&Utc);

        let err = reader
            .read(
                &P2Gate::open(grant),
                &P2ReadRequest {
                    sheet_id: 3,
                    cursor: None,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_WINDOW", "退避期间窗口到期必须停止");
        assert_eq!(server.count(), 3, "初始化 + 通知 + 首次读取，无第二次读取");
    }

    #[test]
    fn p2_https_total_request_cap_stops_before_next_request() {
        let statuses = Arc::new(Mutex::new(vec![500u16]));
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::Script {
                statuses,
                leak: false,
            },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let mut grant = grants(&[(3, &["A"])]);
        grant.endpoint = server.endpoint();
        grant.approved_endpoint = server.endpoint();
        grant.max_total_requests = 3;

        let err = reader
            .read(
                &P2Gate::open(grant),
                &P2ReadRequest {
                    sheet_id: 3,
                    cursor: None,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_EXEC_LIMIT", "整场预算耗尽后停止");
        assert_eq!(server.count(), 3, "初始化 + 通知 + 一次读取后不再发网");
    }

    #[test]
    fn p2_https_completed_authorization_cannot_restart() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);
        reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    cursor: None,
                    page_size: 50,
                },
            )
            .expect("首次读取应成功");
        assert_eq!(server.count(), 4);

        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    cursor: None,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SINGLE_USE");
        assert_eq!(server.count(), 4, "完成后重复启动不得发网");
    }

    #[test]
    fn p2_https_out_of_scope_stops_and_authorization_cannot_restart() {
        let server = spawn_tls_server(TLS_SRV_CERT, TLS_SRV_KEY, ServerMode::JsonOk { leak: true });
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    cursor: None,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SCOPE");
        assert_eq!(server.count(), 3, "越界后无终止、无后续分页");

        let restart = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    cursor: None,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(restart.code(), "P2_E_SINGLE_USE");
        assert_eq!(server.count(), 3, "越界停止后不得再次启动");
    }

    #[test]
    fn p2_https_last_read_attempt_session_expiry_skips_reinitialization() {
        let statuses = Arc::new(Mutex::new(vec![404u16, 500u16, 500u16]));
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::Script {
                statuses,
                leak: false,
            },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);
        let err = reader
            .read(
                &gate,
                &P2ReadRequest {
                    sheet_id: 3,
                    cursor: None,
                    page_size: 50,
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_SESSION");
        assert_eq!(
            server.count(),
            5,
            "初始化 1 + 通知 1 + 读取 3；最后一次额度用完不得再重建会话"
        );
    }

    fn read_req() -> P2ReadRequest {
        P2ReadRequest {
            sheet_id: 3,
            cursor: None,
            page_size: 50,
        }
    }

    /// 初始化失败但不会发网：用于「登记成功后、发网前中断」。
    struct FailingInit;

    impl P2Transport for FailingInit {
        fn initialize(
            &self,
            _endpoint: &P2Endpoint,
            _auth: Option<&str>,
            _deadline: Duration,
        ) -> Result<String, P2Error> {
            Err(P2Error::Protocol("synthetic init failure".into()))
        }

        fn call(
            &self,
            _endpoint: &P2Endpoint,
            _session_id: &str,
            _action: &str,
            _args: &Value,
            _auth: Option<&str>,
            _deadline: Duration,
        ) -> Result<Value, P2Error> {
            Err(P2Error::Protocol("synthetic call failure".into()))
        }
    }

    #[test]
    fn p2_consumed_grant_cannot_restart_after_entry_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p2-state.db");
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let first_store = Arc::new(SqliteP2State::open(&path).expect("state"));
        let first =
            https_entry(https_reader(&server, true), "tenant-a/reader").with_state(first_store);
        let gate = https_grant(&server);
        first.read(&gate, &read_req()).expect("首次执行应成功");
        let after_first = server.count();
        assert_eq!(after_first, 4);

        // 模拟重启：新入口、新连接、同一状态库、同一授权。
        let second_store = Arc::new(SqliteP2State::open(&path).expect("state"));
        let second =
            https_entry(https_reader(&server, true), "tenant-a/reader").with_state(second_store);
        let err = second.read(&gate, &read_req()).unwrap_err();
        assert_eq!(err.code(), "P2_E_ALREADY_USED");
        assert_eq!(server.count(), after_first, "重建入口后同一授权零新增请求");
    }

    #[test]
    fn p2_concurrent_entries_only_one_claims_sqlite_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p2-state.db");
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let store = Arc::new(SqliteP2State::open(&path).expect("state"));
                let reader = entry(
                    CountingTransport::default(),
                    Some("tenant-a/reader"),
                    "2026-11-22T10:30:00Z",
                )
                .with_state(store);
                let gate = P2Gate::open(grants(&[(3, &["A"])]));
                barrier.wait();
                reader
                    .read(&gate, &read_req())
                    .map(|_| ())
                    .map_err(|err| err.code().to_string())
            }));
        }
        let results: Vec<Result<(), String>> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread"))
            .collect();
        let ok = results.iter().filter(|result| result.is_ok()).count();
        let used = results
            .iter()
            .filter(|result| matches!(result, Err(code) if code == "P2_E_ALREADY_USED"))
            .count();
        assert_eq!(ok, 1, "两个进程并发只能一个取得资格: {results:?}");
        assert_eq!(used, 1, "另一个必须是已登记消费: {results:?}");
    }

    #[test]
    fn p2_claim_survives_crash_before_send() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p2-state.db");
        let store = Arc::new(SqliteP2State::open(&path).expect("state"));
        let reader = P2ReadOnlyEntry::new(
            FailingInit,
            Some(Arc::new(SyntheticCredential {
                subject: Some("tenant-a/reader".into()),
            })),
        )
        .with_clock_and_backoff(
            Arc::new(FixedClock(
                DateTime::parse_from_rfc3339("2026-11-22T10:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            )),
            Duration::ZERO,
            P2_MAX_READ_ATTEMPTS,
        )
        .with_state(store);
        let gate = P2Gate::open(grants(&[(3, &["A"])]));
        // 初始化即失败：登记已写，但没有任何 list_records 成功。
        let err = reader.read(&gate, &read_req()).unwrap_err();
        assert_eq!(err.code(), "P2_E_PROTOCOL");

        // 登记后中断：重启也不能恢复同一授权。
        let restarted = P2ReadOnlyEntry::new(
            CountingTransport::default(),
            Some(Arc::new(SyntheticCredential {
                subject: Some("tenant-a/reader".into()),
            })),
        )
        .with_clock_and_backoff(
            Arc::new(FixedClock(
                DateTime::parse_from_rfc3339("2026-11-22T10:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            )),
            Duration::ZERO,
            P2_MAX_READ_ATTEMPTS,
        )
        .with_state(Arc::new(SqliteP2State::open(&path).expect("state")));
        let err = restarted.read(&gate, &read_req()).unwrap_err();
        assert_eq!(err.code(), "P2_E_ALREADY_USED");
    }

    /// 状态库不可用/登记失败：入口必须在发网前停止。
    struct BrokenConsumption;

    impl P2Consumption for BrokenConsumption {
        fn consume(
            &self,
            _grant_id: &str,
            _file_id_masked: &str,
            _window_end: DateTime<Utc>,
            _now: DateTime<Utc>,
        ) -> Result<(), P2Error> {
            Err(P2Error::State("执行状态库不可用".into()))
        }
    }

    #[test]
    fn p2_unusable_state_store_stops_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing-dir").join("p2-state.db");
        let err = SqliteP2State::open(&missing).err().expect("应失败");
        assert_eq!(err.code(), "P2_E_STATE");

        let transport = CountingTransport::default();
        let reader = entry(transport, Some("tenant-a/reader"), "2026-11-22T10:30:00Z")
            .with_state(Arc::new(BrokenConsumption));
        let gate = P2Gate::open(grants(&[(3, &["A"])]));
        let err = reader.read(&gate, &read_req()).unwrap_err();
        assert_eq!(err.code(), "P2_E_STATE");
    }

    #[test]
    fn p2_window_longer_than_limit_and_total_over_cap_are_rejected() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");

        let mut long_window = grants(&[(3, &["A"])]);
        long_window.endpoint = server.endpoint();
        long_window.approved_endpoint = server.endpoint();
        long_window.window_end = long_window.window_start + chrono::Duration::minutes(20);
        let err = reader
            .read(&P2Gate::open(long_window), &read_req())
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_WINDOW");
        assert_eq!(server.count(), 0, "超长窗口零请求");

        let mut over_cap = grants(&[(3, &["A"])]);
        over_cap.endpoint = server.endpoint();
        over_cap.approved_endpoint = server.endpoint();
        over_cap.max_total_requests = P2_MAX_TOTAL_REQUESTS + 1;
        let err = reader
            .read(&P2Gate::open(over_cap), &read_req())
            .unwrap_err();
        assert_eq!(err.code(), "P2_E_EXEC_LIMIT");
        assert_eq!(server.count(), 0, "超上限授权零请求");
    }

    #[test]
    fn p2_https_terminate_failure_reports_unconfirmed_without_extra_requests() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::DeleteFail { status: 500 },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let gate = https_grant(&server);
        let outcome = reader.read(&gate, &read_req()).expect("读取本身应成功");
        assert_eq!(outcome.termination, P2Termination::Unconfirmed);
        assert_eq!(outcome.termination.message(), "远端终止未确认，本地已关闭");
        assert_eq!(server.count(), 4, "终止失败不追加请求");
    }

    #[test]
    fn p2_https_terminate_skipped_when_budget_exhausted() {
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let reader = https_entry(https_reader(&server, true), "tenant-a/reader");
        let mut grant = grants(&[(3, &["A"])]);
        grant.endpoint = server.endpoint();
        grant.approved_endpoint = server.endpoint();
        grant.max_total_requests = 3;
        let outcome = reader
            .read(&P2Gate::open(grant), &read_req())
            .expect("读取应成功");
        let P2Termination::Skipped(reason) = &outcome.termination else {
            assert_eq!(
                outcome.termination,
                P2Termination::Completed,
                "应记录未发送原因"
            );
            return;
        };
        assert!(reason.contains("P2_E_EXEC_LIMIT"), "原因: {reason}");
        assert_eq!(server.count(), 3, "预算耗尽后不发送 DELETE");
    }

    // ---- 跨进程验收：子进程复用本测试二进制（仅在设置 `P2_CP_ROLE` 时执行）----

    const CHILD_TEST: &str = "services::wps_p2::tests::p2_child_worker";

    fn wait_for_file(path: &std::path::Path, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        path.exists()
    }

    fn spawn_p2_child(
        sync: &std::path::Path,
        role: &str,
        db: &std::path::Path,
        port: u16,
        grant_id: &str,
    ) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().expect("current exe"))
            .arg("--exact")
            .arg(CHILD_TEST)
            .arg("--nocapture")
            .env("P2_CP_ROLE", role)
            .env("P2_CP_DB", db)
            .env("P2_CP_PORT", port.to_string())
            .env("P2_CP_GRANT_ID", grant_id)
            .env("P2_CP_SYNC", sync)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn p2 child")
    }

    fn child_output(child: std::process::Child) -> String {
        let output = child.wait_with_output().expect("child output");
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        text
    }

    /// 测试用消费包装：先写状态库，再留一个同步点，用于验证「登记已提交、尚未发网」的时点。
    struct PausingConsumption {
        inner: Arc<SqliteP2State>,
        sync: std::path::PathBuf,
    }

    impl P2Consumption for PausingConsumption {
        fn consume(
            &self,
            grant_id: &str,
            file_id_masked: &str,
            window_end: DateTime<Utc>,
            now: DateTime<Utc>,
        ) -> Result<(), P2Error> {
            self.inner
                .consume(grant_id, file_id_masked, window_end, now)?;
            std::fs::write(self.sync.join("consumed"), b"1").expect("consumed marker");
            let _ = wait_for_file(&self.sync.join("resume"), Duration::from_secs(120));
            Ok(())
        }
    }

    #[test]
    fn p2_child_worker() {
        let Ok(role) = std::env::var("P2_CP_ROLE") else {
            return;
        };
        let db = std::env::var("P2_CP_DB").expect("P2_CP_DB");
        let port: u16 = std::env::var("P2_CP_PORT")
            .expect("P2_CP_PORT")
            .parse()
            .expect("port");
        let grant_id = std::env::var("P2_CP_GRANT_ID").expect("P2_CP_GRANT_ID");
        let sync = std::path::PathBuf::from(std::env::var("P2_CP_SYNC").expect("P2_CP_SYNC"));
        let store = Arc::new(SqliteP2State::open(std::path::Path::new(&db)).expect("state"));
        let consumption: Arc<dyn P2Consumption> = if role == "paused" {
            Arc::new(PausingConsumption {
                inner: store,
                sync: sync.clone(),
            })
        } else {
            store
        };
        let endpoint = endpoint(&format!("https://p2-local.test:{port}/mcp"));
        let transport = HttpsP2Transport::builder(endpoint.clone())
            .resolve(
                "p2-local.test",
                format!("127.0.0.1:{port}").parse().expect("addr"),
            )
            .trust_root_der(TLS_CA.to_vec())
            .build()
            .expect("transport");
        let reader = https_entry(transport, "tenant-a/reader").with_state(consumption);
        let mut grant = grants(&[(3, &["A"])]);
        grant.grant_id = grant_id;
        grant.endpoint = endpoint.clone();
        grant.approved_endpoint = endpoint;
        if role.starts_with("race") {
            std::fs::write(sync.join(format!("ready-{role}")), b"1").expect("ready marker");
            if !wait_for_file(&sync.join("go"), Duration::from_secs(20)) {
                println!("CHILD_ERR role={role} code=P2_E_SYNC_TIMEOUT");
                std::process::exit(3);
            }
        }
        let pid = std::process::id();
        match reader.read(&P2Gate::open(grant), &read_req()) {
            Ok(outcome) => {
                println!(
                    "CHILD_OK role={role} pid={pid} termination={} actions=init{} read{} notify{} term{} total{}",
                    outcome.termination.message(),
                    outcome.actions.initialize,
                    outcome.actions.read,
                    outcome.actions.notify,
                    outcome.actions.terminate,
                    outcome.actions.total
                );
            }
            Err(err) => println!("CHILD_ERR role={role} pid={pid} code={}", err.code()),
        }
        println!("CHILD_EXIT role={role} pid={pid} code=0");
        std::process::exit(0);
    }

    #[test]
    fn p2_cross_process_restart_after_normal_exit_rejects_replay() {
        let dir = tempfile::tempdir().expect("temp dir");
        let sync = dir.path().join("sync");
        std::fs::create_dir_all(&sync).expect("sync dir");
        let db = dir.path().join("state.db");
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let grant_id = "A2-2026-11-23-CP-01";
        let first = spawn_p2_child(&sync, "first", &db, server.port, grant_id);
        let pid_a = first.id();
        let text_a = child_output(first);
        assert!(text_a.contains("CHILD_OK"), "A 应成功：{text_a}");
        assert_eq!(
            server.count(),
            4,
            "A 应完成一轮读取（初始化+通知+读取+终止）"
        );
        let after_a = server.count();
        let replay = spawn_p2_child(&sync, "replay", &db, server.port, grant_id);
        let pid_b = replay.id();
        let text_b = child_output(replay);
        assert!(text_b.contains("P2_E_ALREADY_USED"), "B 应被拒绝：{text_b}");
        assert_eq!(server.count(), after_a, "B 不得新增任何请求");
        println!(
            "CP1 pid_a={pid_a} pid_b={pid_b} after_a={after_a} after_b={}",
            server.count()
        );
    }

    #[test]
    fn p2_cross_process_concurrent_children_claim_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let sync = dir.path().join("sync");
        std::fs::create_dir_all(&sync).expect("sync dir");
        let db = dir.path().join("state.db");
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let grant_id = "A2-2026-11-23-CP-02";
        let a = spawn_p2_child(&sync, "race-a", &db, server.port, grant_id);
        let pid_a = a.id();
        let b = spawn_p2_child(&sync, "race-b", &db, server.port, grant_id);
        let pid_b = b.id();
        assert!(
            wait_for_file(&sync.join("ready-race-a"), Duration::from_secs(20)),
            "A 未就绪"
        );
        assert!(
            wait_for_file(&sync.join("ready-race-b"), Duration::from_secs(20)),
            "B 未就绪"
        );
        std::fs::write(sync.join("go"), b"1").expect("go marker");
        let text_a = child_output(a);
        let text_b = child_output(b);
        let ok = [&text_a, &text_b]
            .iter()
            .filter(|text| text.contains("CHILD_OK"))
            .count();
        let used = [&text_a, &text_b]
            .iter()
            .filter(|text| text.contains("P2_E_ALREADY_USED"))
            .count();
        assert_eq!(ok, 1, "应只有一个进程取得资格：A={text_a} B={text_b}");
        assert_eq!(used, 1, "另一个应被状态库拒绝：A={text_a} B={text_b}");
        assert_eq!(server.count(), 4, "服务端应只发生一轮读取");
        println!(
            "CP2 pid_a={pid_a} pid_b={pid_b} ok={ok} used={used} server_total={}",
            server.count()
        );
    }

    #[test]
    fn p2_cross_process_crash_after_consume_before_send_blocks_restart() {
        let dir = tempfile::tempdir().expect("temp dir");
        let sync = dir.path().join("sync");
        std::fs::create_dir_all(&sync).expect("sync dir");
        let db = dir.path().join("state.db");
        let server = spawn_tls_server(
            TLS_SRV_CERT,
            TLS_SRV_KEY,
            ServerMode::JsonOk { leak: false },
        );
        let grant_id = "A2-2026-11-23-CP-03";
        let mut paused = spawn_p2_child(&sync, "paused", &db, server.port, grant_id);
        let pid_a = paused.id();
        assert!(
            wait_for_file(&sync.join("consumed"), Duration::from_secs(25)),
            "登记未在限时内提交"
        );
        assert_eq!(server.count(), 0, "登记提交后、发网前应为零请求");
        paused.kill().expect("kill paused child");
        let _ = paused.wait();
        let after_kill = server.count();
        let restart = spawn_p2_child(&sync, "after-crash", &db, server.port, grant_id);
        let pid_b = restart.id();
        let text_b = child_output(restart);
        assert!(
            text_b.contains("P2_E_ALREADY_USED"),
            "重启后应被拒绝：{text_b}"
        );
        assert_eq!(server.count(), after_kill, "重启不得新增请求");
        println!(
            "CP3 pid_a={pid_a} pid_b={pid_b} after_kill={after_kill} after_restart={}",
            server.count()
        );
    }
}
