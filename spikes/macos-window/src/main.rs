//! Spike M：macOS 窗口材质 / 自绘工具栏 / 无障碍回退 / 系统强调色的能力验证。
//!
//! 目的（对应重构计划里"押在不确定点上"的 7 项）：在**目标系统（macOS 27）**上把
//! 下列问题的答案拿成证据，而不是靠推断：
//! - M1 原生 sidebar 材质能否透出（`transparent` + `windowEffects`）
//! - M2 Liquid Glass 路线是否可用（本 spike 只验证 Tauri 内置材质；`window-vibrancy`
//!   的 `apply_liquid_glass` 属同族 API，若内置材质可用则它可用性一致）
//! - M3 自绘工具栏：`titleBarStyle: Overlay` + `hiddenTitle` + `trafficLightPosition`
//!   与 `data-tauri-drag-region`（依赖 `core:window:allow-start-dragging`）
//! - M4 透明窗口下 CSS `backdrop-filter` 的实际表现（已知与透明窗口冲突）
//! - M5 系统「外观」设置与 `prefers-reduced-transparency` / `prefers-contrast` 的联动
//! - M6 系统强调色能否读到（用 `defaults read -g AppleHighlightColor`，
//!   避免为此引入 objc 绑定；若此项不可行，退化为使用固定强调色）
//! - M7 透明窗口的内描边观感（只能目视，报告里标注）
//!
//! 结论写入 `report.json`（前端把实测值回传，见 `web/index.html`）。

use tauri::{Manager, WebviewWindow};

/// 切换窗口材质；返回实际生效的材质名（`Err` 表示该材质被系统拒绝）
#[tauri::command]
fn apply_effect(window: WebviewWindow, kind: String) -> Result<String, String> {
    use tauri::utils::config::WindowEffectsConfig;
    use tauri::window::{Effect, EffectState};

    let effect = match kind.as_str() {
        "sidebar" => Some(Effect::Sidebar),
        "under-window-background" => Some(Effect::UnderWindowBackground),
        "header-view" => Some(Effect::HeaderView),
        "hud-window" => Some(Effect::HudWindow),
        "content-background" => Some(Effect::ContentBackground),
        "none" => None,
        other => return Err(format!("未知材质：{other}")),
    };
    let cfg = effect.map(|e| WindowEffectsConfig {
        effects: vec![e],
        state: Some(EffectState::Active),
        radius: None,
        color: None,
    });
    window
        .set_effects(cfg)
        .map(|_| kind.clone())
        .map_err(|e| format!("set_effects({kind}) 失败：{e}"))
}

/// 读取系统强调色 / 高亮色（M6）。
///
/// 走 `defaults read -g`（只读系统偏好）而非 objc 绑定：spike 阶段不值得为一行颜色
/// 引入 `objc2-app-kit`；若生产要跟随系统强调色，可在此结论上决定是否引依赖。
#[tauri::command]
fn read_accent() -> serde_json::Value {
    let read = |key: &str| -> Option<String> {
        let out = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "-g", key])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    serde_json::json!({
        "appleHighlightColor": read("AppleHighlightColor"),
        "appleAccentColor": read("AppleAccentColor"),
        "notes": "AppleHighlightColor 形如 'r g b Name'（0..1 浮点）；AppleAccentColor 为索引（存在时以它为准）；两者都读不到时系统应为默认蓝"
    })
}

/// 回传前端实测结果并落盘 `report.json`（证据归档）
#[tauri::command]
fn write_report(app: tauri::AppHandle, payload: serde_json::Value) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("取应用数据目录失败：{e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("report.json");
    let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            apply_effect,
            read_accent,
            write_report
        ])
        .run(tauri::generate_context!())
        .expect("Spike M 启动失败");
}
