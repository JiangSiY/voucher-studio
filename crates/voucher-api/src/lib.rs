//! Center 命令层（voucher-api）：把桌面版「Tauri 命令」抽成与运行时无关的纯函数。
//!
//! 为什么要有这一层：桌面版（v0.25 冻结）与网页版（`center-server`）用的是**同一套业务核心**
//! `voucher-core`，差别只在「怎么被调用」——桌面走 Tauri IPC，网页走 HTTP JSON。
//! 把命令实现收在这里，两边共享同一份参数契约（`dispatch` 的 cmd 名 = 桌面命令名），
//! 网页端就能复用前端 `src/api/client.ts` 里已有的 96 个调用点，一行不改。
//!
//! 约定：
//! - 命令名与 Tauri 命令**逐字相同**（`list_vouchers` / `get_voucher` …），参数名走 camelCase；
//! - 函数体只调 `voucher-core` 服务，不碰 Tauri（无 `State` / `AppHandle` / 对话框 / 事件）；
//! - 对话框类命令（`pick_*`）在网页端不存在——浏览器用 `<input type=file>` + `/api/upload` 取代；
//! - 事件推送（`ocr:progress` / `enhance:progress` / `dup:changed`）由服务端用 SSE 补，
//!   本层只返回结果。

pub mod accounting;
pub mod backup;
pub mod biz;
pub mod enhance;
pub mod export;
pub mod ingest;
pub mod ledger_check;
pub mod links;
pub mod pack;
pub mod settings;
pub mod vouchers;

use serde::Serialize;
use serde_json::Value;

pub use voucher_core::CoreContext;
use voucher_core::CoreError;

/// 结构化错误（与桌面版 `ApiError` 同形，另带 `fields` 供必填校验展示）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

impl ApiError {
    pub fn from_core(e: CoreError) -> Self {
        let code = match &e {
            CoreError::Io(_) => "io",
            CoreError::Db(_) => "db",
            CoreError::Json(_) => "json",
            CoreError::Keyring(_) => "keyring",
            CoreError::Http(_) => "http",
            CoreError::Pdf(_) => "pdf",
            CoreError::Zip(_) => "zip",
            CoreError::Xlsx(_) => "xlsx",
            CoreError::NotImplemented(_) => "not_implemented",
            CoreError::LocalOcr(_) => "local_ocr",
            CoreError::Cv(_) => "cv",
            CoreError::NotFound(_) => "not_found",
            CoreError::Invalid(_) => "invalid",
            CoreError::RequiredMissing(_) => "required_missing",
            CoreError::LockHeld => "lock_held",
        };
        let fields = match &e {
            CoreError::RequiredMissing(f) => Some(f.clone()),
            _ => None,
        };
        Self {
            code,
            message: e.to_string(),
            fields,
        }
    }

    pub fn plain(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            fields: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::plain("internal", message)
    }
}

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        ApiError::from_core(e)
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

/// `voucher-core::Result<T>` → `ApiResult<T>`
pub fn core<T>(r: voucher_core::Result<T>) -> ApiResult<T> {
    r.map_err(ApiError::from_core)
}

/// 从命令参数对象里取一个 camelCase 字段。
///
/// 缺失一律按 `null` 处理：`Option<T>` 得到 `None`，必填的 `T` 得到一条人话错误——
/// 与 Tauri 的「缺参数」行为一致，但错误文案更清楚。
pub fn get<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> ApiResult<T> {
    let wire = to_camel(key);
    let v = args
        .get(&wire)
        .or_else(|| args.get(key))
        .cloned()
        .unwrap_or(Value::Null);
    serde_json::from_value(v)
        .map_err(|e| ApiError::plain("invalid_args", format!("参数 {wire} 解析失败：{e}")))
}

/// snake_case → camelCase（Tauri 的线上参数名口径）
pub fn to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper = false;
    for c in s.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// 序列化返回值（`()` → `null`，与 Tauri 一致）
pub fn to_value<T: Serialize>(v: T) -> ApiResult<Value> {
    serde_json::to_value(v).map_err(|e| ApiError::plain("json", format!("响应序列化失败：{e}")))
}

/// 执行一条命令。`cmd` 与桌面版 Tauri 命令同名，`args` 是前端传来的参数对象。
pub fn dispatch(ctx: &CoreContext, cmd: &str, args: Value) -> ApiResult<Value> {
    use serde_json::json;
    let v = match cmd {
        // ---------- 凭证 ----------
        "create_memo_voucher" => to_value(vouchers::create_memo_voucher(
            ctx,
            get(&args, "new_voucher")?,
        )?)?,
        "list_vouchers" => to_value(vouchers::list_vouchers(ctx, get(&args, "query")?)?)?,
        "get_voucher" => to_value(vouchers::get_voucher(ctx, get(&args, "id")?)?)?,
        "get_voucher_neighbor" => to_value(vouchers::get_voucher_neighbor(
            ctx,
            get(&args, "id")?,
            get(&args, "query")?,
        )?)?,
        "update_voucher" => to_value(vouchers::update_voucher(
            ctx,
            get(&args, "id")?,
            get(&args, "update")?,
        )?)?,
        "delete_vouchers" => to_value(vouchers::delete_vouchers(ctx, get(&args, "ids")?)?)?,
        "batch_update_status" => to_value(vouchers::batch_update_status(
            ctx,
            get(&args, "ids")?,
            get(&args, "status")?,
        )?)?,
        "batch_edit" => to_value(vouchers::batch_edit(
            ctx,
            get(&args, "ids")?,
            get(&args, "patch")?,
        )?)?,
        "voucher_stats" => to_value(vouchers::voucher_stats(ctx)?)?,
        "monthly_summary" => to_value(vouchers::monthly_summary(ctx, get(&args, "months")?)?)?,
        "top_counterparties" => to_value(vouchers::top_counterparties(
            ctx,
            get(&args, "months")?,
            get(&args, "limit")?,
            get(&args, "date_from")?,
            get(&args, "date_to")?,
        )?)?,
        "tag_stats" => to_value(vouchers::tag_stats(
            ctx,
            get(&args, "months")?,
            get(&args, "limit")?,
            get(&args, "date_from")?,
            get(&args, "date_to")?,
        )?)?,
        "frequent_descriptions" => {
            to_value(vouchers::frequent_descriptions(ctx, get(&args, "limit")?)?)?
        }
        "check_voucher_duplicates" => {
            to_value(vouchers::check_voucher_duplicates(ctx, get(&args, "id")?)?)?
        }
        "scan_duplicate_review" => to_value(vouchers::scan_duplicate_review(ctx)?)?,
        "duplicate_pending_summary" => to_value(vouchers::duplicate_pending_summary(ctx)?)?,
        "list_duplicate_decisions" => to_value(vouchers::list_duplicate_decisions(
            ctx,
            get(&args, "limit")?,
        )?)?,
        "resolve_duplicate_pair" => to_value(vouchers::resolve_duplicate_pair(
            ctx,
            get(&args, "a_id")?,
            get(&args, "b_id")?,
            get(&args, "note")?,
        )?)?,
        "remove_duplicate_voucher" => to_value(vouchers::remove_duplicate_voucher(
            ctx,
            get(&args, "keep_id")?,
            get(&args, "remove_id")?,
            get(&args, "note")?,
        )?)?,
        "dashboard_overview" => to_value(vouchers::dashboard_overview(
            ctx,
            get(&args, "date_from")?,
            get(&args, "date_to")?,
            get(&args, "status")?,
        )?)?,
        "dashboard_todos" => to_value(vouchers::dashboard_todos(ctx)?)?,
        "data_health" => to_value(vouchers::data_health(ctx)?)?,
        "duplicate_pending_count" => to_value(vouchers::duplicate_pending_count(ctx)?)?,
        "monthly_breakdown" => to_value(vouchers::monthly_breakdown(
            ctx,
            get(&args, "date_from")?,
            get(&args, "date_to")?,
            get(&args, "dim")?,
            get(&args, "per_month")?,
        )?)?,
        "export_coverage" => to_value(vouchers::export_coverage(
            ctx,
            get(&args, "date_from")?,
            get(&args, "date_to")?,
        )?)?,
        "storage_usage" => to_value(vouchers::storage_usage(
            ctx,
            get(&args, "with_enhanced_bytes")?,
        )?)?,
        "list_voucher_ids" => to_value(vouchers::list_voucher_ids(ctx, get(&args, "query")?)?)?,
        "list_period_locks" => to_value(vouchers::list_period_locks(ctx)?)?,
        "lock_period" => to_value(vouchers::lock_period(ctx, get(&args, "period")?)?)?,
        "unlock_period" => to_value(vouchers::unlock_period(ctx, get(&args, "period")?)?)?,
        "yearly_summary" => to_value(vouchers::yearly_summary(ctx, get(&args, "year")?)?)?,
        "search_vouchers" => to_value(vouchers::search_vouchers(
            ctx,
            get(&args, "keyword")?,
            get(&args, "filters")?,
            get(&args, "limit")?,
        )?)?,

        // ---------- 导入 / 附件 / OCR / 台账 ----------
        "attachment_abs_path" => to_value(ingest::attachment_abs_path(ctx, get(&args, "id")?)?)?,
        "crop_attachment" => to_value(ingest::crop_attachment(
            ctx,
            get(&args, "attachment_id")?,
            get(&args, "rect")?,
            get(&args, "rotation_deg")?,
        )?)?,
        "run_ocr" => to_value(ingest::run_ocr(ctx, get(&args, "attachment_id")?)?)?,
        "ocr_preview_fields" => to_value(ingest::ocr_preview_fields(
            ctx,
            get(&args, "attachment_id")?,
        )?)?,
        "run_ocr_pending" => to_value(ingest::run_ocr_pending(ctx, get(&args, "limit")?)?)?,
        "re_enhance_all" => to_value(ingest::re_enhance_all(ctx)?)?,
        "local_ocr_status" => to_value(ingest::local_ocr_status())?,
        "ocr_health" => to_value(ingest::ocr_health(ctx))?,
        "cli_import_ledger" => to_value(ingest::cli_import_ledger(ctx, get(&args, "args")?)?)?,
        "cli_list_pending" => to_value(ingest::cli_list_pending(ctx)?)?,
        "ledger_counts" => to_value(ingest::ledger_counts(ctx)?)?,
        "list_ledger_items" => to_value(ingest::list_ledger_items(ctx, get(&args, "stage")?)?)?,
        "list_import_batches" => to_value(ingest::list_import_batches(ctx, get(&args, "limit")?)?)?,
        "undo_import_batch" => to_value(ingest::undo_import_batch(ctx, get(&args, "batch_id")?)?)?,
        "set_ledger_booked" => to_value(ingest::set_ledger_booked(
            ctx,
            get(&args, "voucher_id")?,
            get(&args, "booked")?,
        )?)?,

        // ---------- 台账对账（M4） ----------
        "ledger_check" => ledger_check::ledger_check(ctx, args)?,
        "ledger_check_export" => ledger_check::ledger_check_export(ctx, args)?,

        // ---------- 台账字段（免复核闸门 + 归档改名） ----------
        "get_biz_fields" => to_value(biz::get_biz_fields(ctx, get(&args, "voucher_id")?)?)?,
        "save_biz_fields" => to_value(biz::save_biz_fields(
            ctx,
            get(&args, "voucher_id")?,
            get(&args, "fields")?,
        )?)?,
        "apply_archive_name" => to_value(biz::apply_archive_name(ctx, get(&args, "voucher_id")?)?)?,

        // ---------- 会计投影（M2） ----------
        "gl_build_period" => to_value(accounting::build_period(ctx, get(&args, "period")?)?)?,
        "gl_post_period" => to_value(accounting::post_period(ctx, get(&args, "period")?)?)?,
        "gl_list_entries" => to_value(accounting::list_entries(ctx, get(&args, "period")?)?)?,
        "gl_export_kis" => to_value(accounting::export_kis(ctx, get(&args, "period")?)?)?,
        "gl_export_copy_sheet" => {
            to_value(accounting::export_copy_sheet(ctx, get(&args, "period")?)?)?
        }
        "gl_export_v5_entries" => {
            to_value(accounting::export_v5_entries(ctx, get(&args, "period")?)?)?
        }
        "gl_list_category_maps" => to_value(accounting::list_category_maps(ctx)?)?,
        "gl_upsert_category_map" => {
            to_value(accounting::upsert_category_map(ctx, get(&args, "map")?)?)?
        }
        "gl_list_account_maps" => to_value(accounting::list_account_maps(ctx)?)?,
        "gl_upsert_account_map" => {
            to_value(accounting::upsert_account_map(ctx, get(&args, "map")?)?)?
        }
        "gl_list_periods" => to_value(accounting::list_periods(ctx)?)?,
        "gl_set_period_status" => to_value(accounting::set_period_status(
            ctx,
            get(&args, "period")?,
            get(&args, "status")?,
        )?)?,

        // ---------- 导出 ----------
        "export_pdf_daily" => to_value(export::export_pdf_daily(
            ctx,
            get(&args, "voucher_ids")?,
            get(&args, "options")?,
        )?)?,
        "pack_build" => to_value(pack::pack_build(
            ctx,
            get(&args, "voucherIds")?,
            get(&args, "kind")?,
        )?)?,
        "export_table" => to_value(export::export_table(
            ctx,
            get(&args, "voucher_ids")?,
            get(&args, "format")?,
        )?)?,
        "list_export_batches" => to_value(export::list_export_batches(ctx, get(&args, "limit")?)?)?,
        "export_batch_vouchers" => {
            to_value(export::export_batch_vouchers(ctx, get(&args, "batch_id")?)?)?
        }

        // ---------- 设置 / 凭据 ----------
        "get_config" => to_value(settings::get_config(ctx))?,
        "save_config" => to_value(settings::save_config(ctx, get(&args, "config")?)?)?,
        "validate_naming_template" => {
            to_value(settings::validate_naming_template(get(&args, "template")?)?)?
        }
        "preview_voucher_no" => to_value(settings::preview_voucher_no(ctx, get(&args, "date")?)?)?,
        "list_secrets" => to_value(settings::list_secrets(ctx))?,
        "set_secret" => to_value(settings::set_secret(
            ctx,
            get(&args, "alias")?,
            get(&args, "value")?,
        )?)?,
        "delete_secret" => to_value(settings::delete_secret(ctx, get(&args, "alias")?)?)?,

        // ---------- 关联 / 事项组 ----------
        "link_vouchers" => to_value(links::link_vouchers(
            ctx,
            get(&args, "a_id")?,
            get(&args, "b_id")?,
            get(&args, "kind")?,
            get(&args, "note")?,
        )?)?,
        "unlink_vouchers" => to_value(links::unlink_vouchers(
            ctx,
            get(&args, "a_id")?,
            get(&args, "b_id")?,
            get(&args, "kind")?,
        )?)?,
        "list_voucher_links" => {
            to_value(links::list_voucher_links(ctx, get(&args, "voucher_id")?)?)?
        }
        "list_voucher_link_events" => to_value(links::list_voucher_link_events(
            ctx,
            get(&args, "voucher_id")?,
        )?)?,
        "create_voucher_group" => to_value(links::create_voucher_group(
            ctx,
            get(&args, "title")?,
            get(&args, "note")?,
        )?)?,
        "add_vouchers_to_group" => to_value(links::add_vouchers_to_group(
            ctx,
            get(&args, "group_id")?,
            get(&args, "voucher_ids")?,
        )?)?,
        "remove_voucher_from_group" => to_value(links::remove_voucher_from_group(
            ctx,
            get(&args, "group_id")?,
            get(&args, "voucher_id")?,
        )?)?,
        "voucher_groups_of" => to_value(links::voucher_groups_of(ctx, get(&args, "voucher_id")?)?)?,
        "list_voucher_groups" => to_value(links::list_voucher_groups(ctx)?)?,
        "delete_voucher_group" => {
            to_value(links::delete_voucher_group(ctx, get(&args, "group_id")?)?)?
        }
        "voucher_link_counts" => {
            to_value(links::voucher_link_counts(ctx, get(&args, "voucher_ids")?)?)?
        }
        "list_voucher_group_events" => to_value(links::list_voucher_group_events(
            ctx,
            get(&args, "group_id")?,
        )?)?,

        // ---------- 增强缓存 ----------
        "enhance_status" => to_value(enhance::enhance_status(ctx))?,
        "enhance_status_v2" => to_value(enhance::enhance_status_v2(ctx)?)?,
        "clear_enhance_cache_v2" => to_value(enhance::clear_enhance_cache_v2(
            ctx,
            get(&args, "strategy")?,
            get(&args, "threshold")?,
        )?)?,
        "clear_enhance_cache" => to_value(enhance::clear_enhance_cache(ctx)?)?,
        "enhance_hit_trend" => to_value(enhance::enhance_hit_trend(ctx, get(&args, "days")?)?)?,
        "recent_enhanced" => to_value(enhance::recent_enhanced(ctx, get(&args, "limit")?)?)?,
        "record_viewer_hit" => to_value(enhance::record_viewer_hit(
            ctx,
            get(&args, "attachment_id")?,
        )?)?,
        "delete_enhanced_one" => to_value(enhance::delete_enhanced_one(
            ctx,
            get(&args, "attachment_id")?,
        )?)?,

        // ---------- 备份 / 恢复 / WebDAV ----------
        "run_backup" => to_value(backup::run_backup(ctx)?)?,
        "list_backups" => to_value(backup::list_backups(ctx)?)?,
        "inspect_backup" => to_value(backup::inspect_backup(ctx, get(&args, "path")?)?)?,
        "export_full_backup" => to_value(backup::export_full_backup(ctx, get(&args, "path")?)?)?,
        "drill_backup" => to_value(backup::drill_backup(ctx, get(&args, "path")?)?)?,
        "upload_backup" => to_value(backup::upload_backup(ctx, get(&args, "path")?)?)?,
        "test_webdav" => to_value(backup::test_webdav(ctx)?)?,
        "list_webdav_backups" => to_value(backup::list_webdav_backups(ctx)?)?,
        "download_webdav_backup" => to_value(backup::download_webdav_backup(
            ctx,
            get(&args, "file_name")?,
        )?)?,
        // 恢复要重启进程才生效：服务端只落 restore.pending，由启动时执行
        "restore_backup" => to_value(backup::restore_backup(ctx, get(&args, "path")?)?)?,

        // ---------- 仅桌面端有意义的命令 ----------
        // 网页端由前端换实现（上传 / 浏览器下载），这些名字只在桌面壳里被调用；
        // 真被调到（例如旧前端缓存）时给一句人话，而不是「未知命令」。
        "open_path"
        | "restart_app"
        | "skip_closing_backup"
        | "pick_and_import"
        | "pick_and_replace_image"
        | "pick_and_reconcile"
        | "pick_and_attach_image"
        | "pick_ledger_csv" => {
            return Err(ApiError::plain(
                "unsupported",
                "该操作只在桌面版可用；网页版请用页面上传或浏览器下载",
            ));
        }

        // ---------- 服务端元信息 ----------
        "get_app_info" => to_value(vouchers::app_info(ctx))?,
        "get_recovery_report" => to_value(vouchers::recovery_report(ctx)?)?,
        "capabilities" => json!({
            "platform": "web",
            "localOcr": cfg!(target_os = "macos"),
            "upload": true,
        }),

        other => {
            return Err(ApiError::plain(
                "unknown_command",
                format!("未知命令：{other}"),
            ))
        }
    };
    Ok(v)
}
