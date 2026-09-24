//! 原生凭据库往返（仅桌面平台）。
//!
//! 为什么单独成文件：`set_default_credential_builder` 是**进程级全局**设置，而
//! `tests/common/mod.rs` 会在 `test_ctx()` 里把它换成共享内存后端（供逻辑类用例
//! 使用，见该文件说明）。若本用例与那些用例同处一个测试二进制，同进程内并发执行时
//! 它究竟跑在原生凭据库还是内存后端上，取决于线程调度——那会让「原生后端可用」
//! 这一结论变得不确定。独立成文件即独立进程，全局设置互不干扰，语义恒定。
//!
//! 覆盖价值：生产代码用的正是 macOS 原生后端（Keychain），本用例是唯一直接验证
//! 该链路的用例（内存后端只能验证 SecretStore 的包装逻辑）。
//!
//! Linux（CI runner）无 secret service 守护进程，故仅在 macOS 编译执行。

#[cfg(target_os = "macos")]
#[test]
fn keyring_secret_roundtrip() -> voucher_core::Result<()> {
    let store = voucher_core::SecretStore::new("VoucherStudioTest");
    store.set("test_account", "s3cret-value")?;
    assert_eq!(store.get("test_account")?.as_deref(), Some("s3cret-value"));
    assert!(store.delete("test_account")?);
    assert_eq!(store.get("test_account")?, None);
    assert!(!store.delete("test_account")?);
    Ok(())
}
