//! 前端用到的每条命令都必须在 `dispatch` 里有分支。
//!
//! 为什么要这个测试：网页端把 Tauri `invoke` 换成了 `POST /api/cmd/{cmd}`，
//! 前端 `src/api/client.ts` 是命令名的真源；一旦新增命令只改了桌面壳
//! （`src-tauri/src/commands/`）而漏了 `voucher-api`，网页端就会在运行时
//! 报「未知命令：xxx」（2026-11-18 `enhance_status_v2` 就是这样漏的）。

const CLIENT: &str = include_str!("../../../src/api/client.ts");
const DISPATCH: &str = include_str!("../src/lib.rs");

/// 从 `invoke<Foo>("name"` / `invoke("name")` 里抠命令名
fn frontend_commands() -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in CLIENT.match_indices("invoke") {
        let rest = &CLIENT[i + "invoke".len()..];
        // 跳过泛型参数 `invoke<Foo>`
        let rest = if let Some(stripped) = rest.strip_prefix('<') {
            match stripped.find('>') {
                Some(j) => &stripped[j + 1..],
                None => continue,
            }
        } else {
            rest
        };
        let rest = rest.trim_start();
        if let Some(rest) = rest.strip_prefix('(') {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('"') {
                if let Some(end) = rest.find('"') {
                    let name = &rest[..end];
                    if !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
                    {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn every_frontend_command_has_a_dispatch_arm() {
    let cmds = frontend_commands();
    assert!(
        cmds.len() > 100,
        "解析 client.ts 失败：只找到 {} 条命令",
        cmds.len()
    );
    let missing: Vec<&str> = cmds
        .iter()
        .filter(|c| !DISPATCH.contains(&format!("\"{c}\"")))
        .map(String::as_str)
        .collect();
    assert!(
        missing.is_empty(),
        "这些命令前端在用、dispatch 里没有：{missing:?}"
    );
}
