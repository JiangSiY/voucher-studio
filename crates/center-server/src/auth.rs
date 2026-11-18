//! 单用户认证：首次设置密码 → 登录 → 会话 Cookie。
//!
//! 为什么是「单用户 + 密码 + Cookie」而不是 OAuth/JWT：
//! 这是老板一个人的账务系统，部署在 Tailscale 内网，访问者只有他自己与几台设备。
//! 密码登录 + 服务端会话（可一键踢下线）比 JWT 更简单、更好审计；
//! 不引入第三方登录，财务数据不经过任何第三方身份服务。
//!
//! 会话只存 **token 的 sha256**，不存明文：数据库被拷走也换不出可用的 Cookie。
//! 口令哈希用 PBKDF2-HMAC-SHA256（12 万轮）——本机离线可编译的依赖里没有 argon2，
//! 存储格式自描述：`pbkdf2-sha256$120000$<salt>$<hash>`，以后换算法可平滑升级。

use std::sync::Arc;

use hyper::HeaderMap;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use voucher_core::{CoreContext, CoreError, Result as CoreResult};

use crate::state::{now_iso, ApiErr, AppState};

pub const COOKIE_NAME: &str = "center_session";
const SESSION_DAYS: i64 = 30;
const PBKDF2_ROUNDS: u32 = 120_000;

pub struct AuthReply {
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

#[derive(Deserialize)]
pub struct PasswordBody {
    pub password: String,
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    format!("{:x}", h.finalize())
}

fn new_token() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_password(pw: &str) -> String {
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let mut out = [0u8; 32];
    pbkdf2_hmac::<Sha256>(pw.as_bytes(), &salt, PBKDF2_ROUNDS, &mut out);
    format!(
        "pbkdf2-sha256${PBKDF2_ROUNDS}${}${}",
        salt.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        out.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

fn verify_password(pw: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 4 || parts[0] != "pbkdf2-sha256" {
        return false;
    }
    let rounds: u32 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let (salt, want) = match (unhex(parts[2]), unhex(parts[3])) {
        (Some(s), Some(w)) => (s, w),
        _ => return false,
    };
    let mut out = vec![0u8; want.len()];
    pbkdf2_hmac::<Sha256>(pw.as_bytes(), &salt, rounds, &mut out);
    // 定长比较，避免计时侧信道
    let mut diff = 0u8;
    for (a, b) in out.iter().zip(want.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(hyper::header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

pub fn current_user(ctx: &CoreContext, headers: &HeaderMap) -> Option<i64> {
    let token = cookie_value(headers, COOKIE_NAME)?;
    session_user(ctx, &token)
}

fn has_user(ctx: &CoreContext) -> CoreResult<bool> {
    ctx.db.with_conn(|c| {
        let n: i64 = c.query_row("SELECT COUNT(*) FROM auth_users", [], |r| r.get(0))?;
        Ok(n > 0)
    })
}

fn create_user(ctx: &CoreContext, password: &str) -> CoreResult<i64> {
    let hash = hash_password(password);
    let now = now_iso();
    ctx.db.with_conn(|c| {
        c.execute(
            "INSERT INTO auth_users (username, password_hash, created_at, updated_at)
             VALUES ('owner', ?1, ?2, ?2)",
            rusqlite::params![hash, now],
        )?;
        Ok(c.last_insert_rowid())
    })
}

fn create_session(ctx: &CoreContext, user_id: i64, ua: &str) -> CoreResult<String> {
    let token = new_token();
    let hash = sha256_hex(token.as_bytes());
    let now = chrono::Local::now();
    let expires = now + chrono::Duration::days(SESSION_DAYS);
    let now_s = now.format("%Y-%m-%dT%H:%M:%S%:z").to_string();
    let exp_s = expires.format("%Y-%m-%dT%H:%M:%S%:z").to_string();
    ctx.db.with_conn(|c| {
        c.execute(
            "INSERT INTO auth_sessions (token_hash, user_id, created_at, expires_at, last_seen_at, user_agent)
             VALUES (?1, ?2, ?3, ?4, ?3, ?5)",
            rusqlite::params![hash, user_id, now_s, exp_s, ua],
        )?;
        Ok(())
    })?;
    Ok(token)
}

fn session_user(ctx: &CoreContext, token: &str) -> Option<i64> {
    let hash = sha256_hex(token.as_bytes());
    let now = now_iso();
    ctx.db
        .with_conn(|c| {
            // 顺手清掉过期会话，免得起一个定时任务
            c.execute(
                "DELETE FROM auth_sessions WHERE expires_at < ?1",
                rusqlite::params![now],
            )?;
            let uid: Option<i64> = c
                .query_row(
                    "SELECT user_id FROM auth_sessions WHERE token_hash = ?1 AND expires_at >= ?2",
                    rusqlite::params![hash, now],
                    |r| r.get(0),
                )
                .ok();
            if uid.is_some() {
                c.execute(
                    "UPDATE auth_sessions SET last_seen_at = ?1 WHERE token_hash = ?2",
                    rusqlite::params![now, hash],
                )?;
            }
            Ok(uid)
        })
        .ok()
        .flatten()
}

fn cookie_header(token: &str, secure: bool) -> String {
    let max_age = SESSION_DAYS * 24 * 3600;
    let mut s = format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    if secure {
        s.push_str("; Secure");
    }
    s
}

fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(hyper::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

/// 启动时若配置了 `CENTER_PASSWORD` 且库里还没有用户，就直接建号（无头部署用）
pub fn bootstrap_password(ctx: &CoreContext, password: &str) -> CoreResult<()> {
    if has_user(ctx)? {
        return Ok(());
    }
    if password.trim().len() < 8 {
        return Err(CoreError::Invalid("CENTER_PASSWORD 至少 8 位".into()));
    }
    create_user(ctx, password)?;
    log::info!("首次启动：已按 CENTER_PASSWORD 创建访问账号 owner");
    Ok(())
}

// ---------- 路由（同步实现，由 http 层 spawn_blocking 调用） ----------

pub fn status(ctx: &CoreContext, headers: &HeaderMap) -> Value {
    let needs_setup = has_user(ctx).map(|exists| !exists).unwrap_or(true);
    let logged_in = current_user(ctx, headers).is_some();
    json!({ "needsSetup": needs_setup, "loggedIn": logged_in })
}

pub fn setup(
    st: &Arc<AppState>,
    headers: &HeaderMap,
    body: &PasswordBody,
) -> Result<AuthReply, ApiErr> {
    if body.password.trim().len() < 8 {
        return Err(ApiErr::bad_request("invalid", "密码至少 8 位"));
    }
    if has_user(&st.ctx).map_err(|e| ApiErr::internal(e.to_string()))? {
        return Err(ApiErr::bad_request("invalid", "已经设置过密码，请直接登录"));
    }
    let uid = create_user(&st.ctx, &body.password)
        .map_err(|e| ApiErr::bad_request("setup_failed", e.to_string()))?;
    let token = create_session(&st.ctx, uid, &user_agent(headers))
        .map_err(|e| ApiErr::internal(e.to_string()))?;
    Ok(AuthReply {
        headers: vec![("set-cookie".into(), cookie_header(&token, st.cookie_secure))],
        body: json!({ "ok": true }),
    })
}

pub fn login(
    st: &Arc<AppState>,
    headers: &HeaderMap,
    body: &PasswordBody,
) -> Result<AuthReply, ApiErr> {
    let row: Option<(i64, String)> = st
        .ctx
        .db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT id, password_hash FROM auth_users WHERE username = 'owner'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok())
        })
        .map_err(|e| ApiErr::internal(e.to_string()))?;
    let Some((uid, hash)) = row else {
        return Err(ApiErr::unauthorized("还没有设置访问密码"));
    };
    if !verify_password(&body.password, &hash) {
        return Err(ApiErr::unauthorized("密码不正确"));
    }
    let token = create_session(&st.ctx, uid, &user_agent(headers))
        .map_err(|e| ApiErr::internal(e.to_string()))?;
    Ok(AuthReply {
        headers: vec![("set-cookie".into(), cookie_header(&token, st.cookie_secure))],
        body: json!({ "ok": true }),
    })
}

pub fn logout(st: &Arc<AppState>, headers: &HeaderMap) -> AuthReply {
    if let Some(token) = cookie_value(headers, COOKIE_NAME) {
        let hash = sha256_hex(token.as_bytes());
        let _ = st.ctx.db.with_conn(|c| {
            c.execute(
                "DELETE FROM auth_sessions WHERE token_hash = ?1",
                rusqlite::params![hash],
            )?;
            Ok(())
        });
    }
    AuthReply {
        headers: vec![(
            "set-cookie".into(),
            format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
        )],
        body: json!({ "ok": true }),
    }
}
