//! 备份链路集成测试：本地增量（快照+附件包）/ GFS 保留 / 异盘双写 / 恢复 / 演练 / 崩溃恢复。

mod common;

use std::io::Read;
use std::path::{Path, PathBuf};

use common::*;
use voucher_core::models::{NewVoucher, VoucherUpdate};
use voucher_core::services::{backup, ingestion, recovery, voucher_mgr};
use voucher_core::{CoreContext, Result};
#[test]
fn backup_and_restore_roundtrip() -> Result<()> {
    let (dir, ctx) = test_ctx();
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "备份前数据".into(),
            ..Default::default()
        },
    )?;

    let info = backup::run_backup(&ctx)?;
    assert!(PathBuf::from(&info.path).exists());

    let backups = backup::list_backups(&ctx)?;
    assert_eq!(backups.len(), 1);

    // 校验非法备份被拒
    let fake = dir.path().join("fake.zip");
    std::fs::write(&fake, b"not a zip")?;
    assert!(backup::inspect_backup(&ctx, &fake).is_err());

    // 恢复到新目录并验证数据在
    let target = dir.path().join("restored");
    let summary = backup::restore_backup(&ctx, PathBuf::from(&info.path).as_path(), &target)?;
    assert!(!summary.restored_files.is_empty());
    assert!(target.join("data").join("voucher.db").exists());

    let conn = voucher_core::db::connection::open(&target.join("data").join("voucher.db"))?;
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM vouchers", [], |r| r.get(0))?;
    assert_eq!(n, 1);
    Ok(())
}

/// v0.14-A：备份加密全链路——开启加密后快照/附件包以 age passphrase 信封
///（格式 v4，AEAD 整文件密封）落盘、恢复（materialize 解密 + 合成）数据完整；
/// 未设密码时备份拒绝执行（不静默降级明文）。
#[test]
fn encrypted_backup_roundtrip() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img = dir.path().join("enc.png");
    make_png_variant(&img, 77);
    ingestion::import_images(&ctx, &[img], None, false)?;
    // 幂等：清掉上次运行（可能中途失败）留下的密码
    let _ = ctx
        .secrets
        .delete(voucher_core::utils::config::SECRET_BACKUP_PASSWORD);

    // 1. 开启加密但未设密码 → 备份拒绝（不产生明文备份）
    {
        let mut cfg = ctx.config();
        cfg.backup.encrypted = true;
        ctx.set_config(cfg)?;
    }
    let err = backup::run_backup(&ctx);
    assert!(
        matches!(err, Err(voucher_core::CoreError::Invalid(ref e)) if e.contains("未设置密码")),
        "开启加密无密码应拒绝备份，实际：{err:?}"
    );

    // 2. 设置密码（凭据管理器；测试后清理）
    ctx.secrets.set(
        voucher_core::utils::config::SECRET_BACKUP_PASSWORD,
        "test-enc-pwd",
    )?;

    let info = backup::run_backup(&ctx)?;
    assert!(PathBuf::from(&info.path).exists());
    assert!(info.attachment_count >= 1, "附件进加密包");

    // 3. 落盘的是 age 信封（格式 v4 整文件密封），zip 打开直接失败（需先解密）
    let snapshot = PathBuf::from(&info.path);
    {
        let head = std::fs::read(&snapshot)?;
        assert!(
            head.starts_with(b"age-encryption.org/v1"),
            "v4 加密快照应为 age 信封头"
        );
        assert!(
            zip::ZipArchive::new(std::io::Cursor::new(head.as_slice())).is_err(),
            "age 信封不是 zip：需解密后才能读取"
        );
    }
    // 附件包同检（备份目录下按内容寻址命名）
    let bundle = find_bundle(&snapshot)?;
    {
        let head = std::fs::read(&bundle)?;
        assert!(
            head.starts_with(b"age-encryption.org/v1"),
            "v4 附件包应为 age 信封头"
        );
    }

    // 4. 恢复链路（read_manifest / compose 走凭据管理器解密）：数据完整
    let target = dir.path().join("restored-enc");
    let summary = backup::restore_backup(&ctx, &snapshot, &target)?;
    assert!(summary.checksums_verified, "解密后 sha256 校验通过");
    assert!(summary.attachment_count >= 1);
    assert!(target.join("data").join("voucher.db").exists());
    let conn = voucher_core::db::connection::open(&target.join("data").join("voucher.db"))?;
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM vouchers", [], |r| r.get(0))?;
    assert_eq!(n, 1, "恢复出加密备份中的凭证");
    // 附件树非空（attachments/<年>/<月>/ 深层结构，顶层是目录）
    let mut atts = 0;
    for e in std::fs::read_dir(target.join("attachments"))? {
        atts += e?.path().is_dir() as i32;
    }
    assert!(atts >= 1, "附件从加密包中释放");

    // 5. 演练同样通过
    let report = backup::drill_backup(&ctx, None)?;
    assert!(report.db_ok);

    // 清理凭据（不污染其他测试 / 用户凭据库）
    let _ = ctx
        .secrets
        .delete(voucher_core::utils::config::SECRET_BACKUP_PASSWORD);
    Ok(())
}

/// 快照同目录下找附件包（备份目录按内容寻址前缀枚举）
fn find_bundle(snapshot: &Path) -> Result<PathBuf> {
    let dir = snapshot.parent().unwrap_or(Path::new("."));
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
        if name.starts_with("voucher-attachments-") {
            return Ok(p);
        }
    }
    Err(voucher_core::CoreError::NotFound("附件包未找到".into()))
}

#[test]
fn backup_includes_attachments_and_drill_passes() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let img = _dir.path().join("a.png");
    make_png(&img);
    ingestion::import_images(&ctx, &[img], None, false)?;

    let info = backup::run_backup(&ctx)?;
    assert!(info.attachment_count >= 1, "附件应计入备份");

    // 附件在恢复产物中（3-2-1：附件原图不可再生，必须在包内）
    let target = _dir.path().join("restored");
    let summary = backup::restore_backup(&ctx, PathBuf::from(&info.path).as_path(), &target)?;
    assert!(summary.checksums_verified, "v2 备份应完成 sha256 校验");
    assert!(summary.attachment_count >= 1);
    assert!(
        target.join("attachments").is_dir(),
        "恢复产物应含 attachments 目录"
    );
    let mut has_file = false;
    for entry in std::fs::read_dir(target.join("attachments"))? {
        let path = entry?.path();
        has_file |= path.is_file() || path.is_dir();
    }
    assert!(has_file, "附件目录不应为空");

    // 恢复演练：校验 + 解包 + 打开数据库，全部通过
    let report = backup::drill_backup(&ctx, None)?;
    assert!(report.db_ok);
    assert!(report.checksums_verified);
    assert!(report.attachment_count >= 1);
    Ok(())
}

#[test]
fn tampered_backup_rejected_by_checksum() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    voucher_mgr::create_voucher(
        &ctx,
        NewVoucher {
            description: "校验和测试".into(),
            ..Default::default()
        },
    )?;
    let info = backup::run_backup(&ctx)?;

    // 构造篡改包：同 entry 名，但 voucher.db 中部翻转一个字节，manifest 保持原样
    let src = PathBuf::from(&info.path);
    let tampered = _dir.path().join("tampered.zip");
    {
        let file = std::fs::File::open(&src)?;
        let mut zip = zip::ZipArchive::new(file)?;
        let out = std::fs::File::create(&tampered)?;
        let mut w = zip::ZipWriter::new(out);
        let opts = zip::write::SimpleFileOptions::default();
        for i in 0..zip.len() {
            let mut bytes = Vec::new();
            let name = {
                let mut e = zip.by_index(i)?;
                e.read_to_end(&mut bytes)?;
                e.name().to_string()
            };
            if name == backup::DB_ENTRY {
                let mid = bytes.len() / 2;
                bytes[mid] ^= 0xFF;
            }
            w.start_file(name.as_str(), opts)?;
            std::io::Write::write_all(&mut w, &bytes)?;
        }
        w.finish()?;
    }

    let err = backup::inspect_backup(&ctx, &tampered);
    assert!(err.is_err(), "被篡改的备份必须被 sha256 校验拒绝");
    Ok(())
}

#[test]
fn secondary_dir_gets_copy_of_backup() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    let secondary = _dir.path().join("other-disk");
    let mut cfg = ctx.config();
    cfg.backup.secondary_dir = secondary.to_string_lossy().to_string();
    ctx.set_config(cfg)?;

    let info = backup::run_backup(&ctx)?;
    assert!(
        secondary.join(&info.file_name).is_file(),
        "异盘目录应有备份副本"
    );
    Ok(())
}

// ---------- v0.9 本地增量备份（快照 + 内容寻址附件包） ----------

/// backups/ 目录内附件包（voucher-attachments-*.zip）数量
fn count_bundles(ctx: &CoreContext) -> Result<usize> {
    let mut n = 0;
    for e in std::fs::read_dir(&ctx.paths.backups)? {
        let e = e?;
        if e.file_name()
            .to_string_lossy()
            .starts_with("voucher-attachments-")
        {
            n += 1;
        }
    }
    Ok(n)
}

/// 附件未变 → 二次备份复用附件包（快照每次新增、bundle 不重建），快照可透明恢复
#[test]
fn local_incremental_reuses_bundle_when_attachments_unchanged() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img = dir.path().join("a.png");
    make_png(&img);
    let s = ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    assert_eq!(s.vouchers.len(), 1);

    let info1 = backup::run_backup(&ctx)?;
    assert_eq!(info1.kind, "snapshot", "新备份产物是快照");
    assert!(info1.file_name.starts_with("voucher-snapshot-"));
    let bundles1 = count_bundles(&ctx)?;
    assert_eq!(bundles1, 1, "首备份产生一个附件包");

    // 附件未变（只改非附件数据：备注）→ 再备份
    voucher_mgr::update_voucher(
        &ctx,
        s.vouchers[0].voucher_id,
        VoucherUpdate {
            note: Some("改备注再备份".into()),
            ..Default::default()
        },
    )?;
    let info2 = backup::run_backup(&ctx)?;
    assert_ne!(info1.file_name, info2.file_name, "快照每次新增");
    assert_eq!(
        count_bundles(&ctx)?,
        bundles1,
        "附件未变：附件包不重建（本次 0 附件 I/O）"
    );

    // 快照透明恢复：合成全量 → 数据完整
    let target = dir.path().join("restored");
    let summary = backup::restore_backup(&ctx, PathBuf::from(&info2.path).as_path(), &target)?;
    assert!(summary.attachment_count >= 1);
    assert!(summary.checksums_verified);
    let conn = voucher_core::db::connection::open(&target.join("data").join("voucher.db"))?;
    let note: String = conn.query_row("SELECT note FROM vouchers LIMIT 1", [], |r| r.get(0))?;
    assert_eq!(note, "改备注再备份", "恢复的是第二次备份的内容");
    Ok(())
}

/// 附件集变化 → 新附件包；GFS 清理后无引用的旧附件包被回收
#[test]
fn local_incremental_gc_reclaims_orphan_bundles() -> Result<()> {
    let (dir, ctx) = test_ctx();
    // 附件集 A → 备份
    let img1 = dir.path().join("a.png");
    make_png_variant(&img1, 60);
    let s1 = ingestion::import_images(&ctx, std::slice::from_ref(&img1), None, false)?;
    backup::run_backup(&ctx)?;

    // 附件集 B（删旧图 + 加新图）→ 再备份 → 两个附件包
    voucher_mgr::delete_vouchers(&ctx, &[s1.vouchers[0].voucher_id])?;
    let img2 = dir.path().join("b.png");
    make_png_variant(&img2, 180);
    ingestion::import_images(&ctx, std::slice::from_ref(&img2), None, false)?;
    backup::run_backup(&ctx)?;
    assert_eq!(count_bundles(&ctx)?, 2, "两个附件集 → 两个附件包");

    // GFS 全删（keep=0）：快照清光 → 两个附件包均无引用 → 回收
    let mut cfg = ctx.config();
    cfg.backup.keep_daily = 0;
    cfg.backup.keep_weekly = 0;
    cfg.backup.keep_monthly = 0;
    ctx.set_config(cfg)?;
    backup::run_backup(&ctx)?; // 末尾 enforce_retention 触发清理
    assert!(
        backup::list_backups(&ctx)?.is_empty(),
        "keep 全 0：本地不留备份"
    );
    assert_eq!(count_bundles(&ctx)?, 0, "无引用附件包应被回收");
    Ok(())
}

/// 导出完整备份：快照 + 附件包合成单个全量 zip（可拷走、可恢复、进列表）
#[test]
fn export_full_backup_composes_single_zip() -> Result<()> {
    let (dir, ctx) = test_ctx();
    let img = dir.path().join("a.png");
    make_png(&img);
    ingestion::import_images(&ctx, std::slice::from_ref(&img), None, false)?;
    let info = backup::run_backup(&ctx)?;

    let exported = backup::export_full_backup(&ctx, Path::new(&info.path))?;
    let name = PathBuf::from(&exported)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        name.starts_with("voucher-backup-"),
        "导出物应是可恢复的全量命名：{name}"
    );

    // 导出的全量可直接恢复（不再依赖附件包）
    let target = dir.path().join("restored");
    backup::restore_backup(&ctx, Path::new(&exported), &target)?;
    assert!(target.join("data").join("voucher.db").exists());

    // 导出物进备份列表，类型 full
    let rows = backup::list_backups(&ctx)?;
    let row = rows.iter().find(|b| b.path == exported);
    assert!(row.is_some(), "导出物应出现在备份列表");
    assert_eq!(row.unwrap().kind, "full");
    Ok(())
}

#[test]
fn unwritable_secondary_dir_warns_but_backup_succeeds() -> Result<()> {
    let (_dir, ctx) = test_ctx();
    // 占位同名文件 → create_dir_all 失败 → 异盘副本失败
    let blocked = _dir.path().join("blocked");
    std::fs::write(&blocked, b"file not dir")?;

    let mut cfg = ctx.config();
    cfg.backup.secondary_dir = blocked.to_string_lossy().to_string();
    ctx.set_config(cfg)?;

    let info = backup::run_backup(&ctx)?;
    assert!(PathBuf::from(&info.path).is_file(), "本地主备份不受影响");
    assert!(
        !info.warnings.is_empty(),
        "异盘失败应产生告警供 UI 提示，实际: {:?}",
        info.warnings
    );
    assert!(info.warnings.iter().any(|w| w.contains("异盘")));
    Ok(())
}

/// 崩溃恢复（recovery_window 后端）：会话 marker 状态机 + 只读健康检查
#[test]
fn recovery_marker_and_health_check() -> Result<()> {
    let (dir, ctx) = test_ctx();

    // 首次启动：无 marker → 正常
    assert!(
        recovery::mark_running(&ctx)?.is_none(),
        "首次启动不应报告异常退出"
    );

    // 未正常退出（模拟崩溃）→ marker 残留 → 第二次启动检测到
    let second = recovery::mark_running(&ctx)?;
    let last = second.expect("marker 残留应被识别为异常退出");
    assert!(!last.started_at.is_empty());
    assert!(last.pid > 0);

    // 正常退出 → 清除 marker → 再启动恢复为正常
    recovery::mark_clean_exit(&ctx)?;
    assert!(recovery::mark_running(&ctx)?.is_none());

    // 健康检查：空库基线
    let report = recovery::check_health(&ctx)?;
    assert!(report.db_ok, "空库完整性应通过");
    assert_eq!(report.attachment_total, 0);
    assert_eq!(report.attachment_missing_count, 0);
    assert!(report.attachment_missing.is_empty());
    assert!(report.last_backup.is_none());

    // 带附件 + 备份的健康检查
    let img = dir.path().join("a.png");
    make_png(&img);
    let s = ingestion::import_images(&ctx, &[img], None, false)?;
    backup::run_backup(&ctx)?;
    let report2 = recovery::check_health(&ctx)?;
    assert!(report2.db_ok);
    assert_eq!(report2.attachment_total, 1);
    assert_eq!(report2.attachment_missing_count, 0);
    assert!(report2.last_backup.is_some(), "已有备份应出现在报告中");

    // 附件缺失场景：删掉物理文件 → 报告缺失
    let rel = s.vouchers[0].attachment.file_path.clone();
    std::fs::remove_file(ctx.paths.attachment_abs(&rel))?;
    let report3 = recovery::check_health(&ctx)?;
    assert_eq!(report3.attachment_missing_count, 1);
    assert_eq!(report3.attachment_missing.len(), 1);
    assert_eq!(
        report3.attachment_missing[0],
        s.vouchers[0].attachment.file_name
    );

    // 组合入口：异常退出状态正确汇入报告
    let _ = recovery::mark_clean_exit(&ctx);
    assert!(!recovery::check_recovery(&ctx)?.abnormal_exit);
    let _ = recovery::mark_running(&ctx)?; // 制造残留
    let combined = recovery::check_recovery(&ctx)?;
    assert!(
        combined.abnormal_exit,
        "check_recovery 应汇入 marker 检测结果"
    );
    assert!(combined.last_session.is_some());
    Ok(())
}
