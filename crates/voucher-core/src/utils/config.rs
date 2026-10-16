//! 用户配置（对应 v4.3 的 `config.py`；密钥不入配置文件，走 keyring）

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::utils::jsonio;
use crate::Result;

/// 密钥在系统凭据库中的账号名（密钥不明文落盘）
pub const SECRET_OCR_API_KEY: &str = "ocr_api_key";
pub const SECRET_WEBDAV_PASSWORD: &str = "webdav_password";
/// v0.14-A：备份加密密码（backup.encrypted 启用时备份文件用 age passphrase 信封密封，
/// 密码存凭据管理器 SECRET_BACKUP_PASSWORD）
pub const SECRET_BACKUP_PASSWORD: &str = "backup_password";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct AppConfig {
    pub appearance: AppearanceConfig,
    pub naming: NamingConfig,
    pub ocr: OcrConfig,
    pub backup: BackupConfig,
    pub print: PrintConfig,
    /// v0.6：文档扫描矫正（5 步算法开关）
    pub enhance: EnhanceConfig,
    /// v0.10：凭证录入辅助（常用摘要模板等）
    pub voucher: VoucherConfig,
}

/// v0.10：凭证录入辅助配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct VoucherConfig {
    /// 常用摘要模板（设置页维护；编辑页联想 = 模板 + 历史高频摘要合并去重）
    pub description_templates: Vec<String>,
}

/// 侧边栏宽度钳制范围与默认值（前端 `src/composables/useSidebarResize.ts` 保持同值）
/// 2026-10-16 收窄：去掉侧栏左上角图标、徽标改为压在图标上的角标后，行内容只需
/// 图标 24 + 间距 + 标签（最长「打印与导出」约 75px）+ 内边距 ≈ 145px，
/// 故最小宽度 220 → **184**、默认宽度 260 → **220**（用户要求"缩减侧栏占用的 UI 面积"）。
pub const SIDEBAR_WIDTH_MIN: u32 = 184;
pub const SIDEBAR_WIDTH_MAX: u32 = 400;
/// 自动备份间隔的合法区间（小时）：下限 1h、上限 30 天。见 `normalize` 的注释（审计 B10）。
pub const MIN_BACKUP_INTERVAL_HOURS: u64 = 1;
pub const MAX_BACKUP_INTERVAL_HOURS: u64 = 24 * 30;
pub const SIDEBAR_WIDTH_DEFAULT: u32 = 220;

/// 界面字号档位（倍数）。**默认偏大**：用户视力不好，明确要求"字号要大一些"
/// （2026-09-24 反馈；当时正文 13px 被评价为太小）。
/// 该值写入 CSS 变量 `--font-scale`，只作用于设计令牌定义的排版与控件高度。
pub const UI_SCALE_DEFAULT: f32 = 1.15;
pub const UI_SCALE_MIN: f32 = 1.0;
pub const UI_SCALE_MAX: f32 = 1.5;

/// 外观：light / dark / system + 侧边栏宽度 + 侧边栏是否收起
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppearanceConfig {
    pub theme: String,
    /// 侧边栏宽度（px）。手改 app.json 可能越界，读取与写入都经 [`AppConfig::normalize`] 钳制
    pub sidebar_width: u32,
    /// 侧边栏是否收起（用户 2026-10-16 要求：收起后只显示图标，56px 图标栏）。
    /// 布尔值无需钳制；旧配置文件无此字段时由容器级 `#[serde(default)]` 补齐为 false（展开）。
    pub sidebar_collapsed: bool,
    /// 界面字号倍数（1.0–1.5）。前端写入 `--font-scale`；越界同样经 normalize 钳制。
    /// 缺省 1.15（大字号基线），旧配置文件无此字段时由此 `#[serde(default)]` 补齐。
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
}

fn default_ui_scale() -> f32 {
    UI_SCALE_DEFAULT
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            sidebar_width: SIDEBAR_WIDTH_DEFAULT,
            sidebar_collapsed: false,
            ui_scale: UI_SCALE_DEFAULT,
        }
    }
}

/// 凭证编号命名规则：模板令牌 {date}/{date-}（YYYYMMDD/连字符日期）与 {seq}/{n}（三位/两位序号）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NamingConfig {
    pub template: String,
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            template: "{date-}-{n}".into(),
        }
    }
}

/// OCR：本地（ort + PP-OCRv5，离线）优先；云端（HTTP API）备选
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OcrConfig {
    /// local | cloud
    pub engine: String,
    pub enabled: bool,
    /// 云端 OCR API 地址
    pub api_url: String,
    pub model: String,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            engine: "local".into(),
            enabled: true,
            api_url: String::new(),
            model: String::new(),
        }
    }
}

/// 备份：本地定时 zip + 可选 WebDAV 上传
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BackupConfig {
    pub auto_enabled: bool,
    /// 自动备份间隔（小时）
    pub interval_hours: u64,
    /// 关闭应用前自动备份（本地 zip，跳过 WebDAV/异盘双写；避免关闭卡顿）
    pub auto_backup_on_close: bool,
    /// GFS 分层保留：近 N 天每日备份全保留
    pub keep_daily: u32,
    /// GFS 分层保留：再保留 N 周的每周最新一份
    pub keep_weekly: u32,
    /// GFS 分层保留：再保留 N 月的每月最新一份
    pub keep_monthly: u32,
    /// 备用份目录（异盘双写；空 = 不启用；须指向另一物理盘/移动硬盘才有意义）
    pub secondary_dir: String,
    /// v0.14-A：备份加密（age passphrase 信封整文件密封，AEAD 防篡改；密码存凭据
    /// 管理器 SECRET_BACKUP_PASSWORD，开启后本地/异盘/云端备份全部加密，恢复时自动解密）
    pub encrypted: bool,
    pub webdav: WebdavConfig,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            auto_enabled: true,
            interval_hours: 24,
            auto_backup_on_close: false,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            secondary_dir: String::new(),
            encrypted: false,
            webdav: WebdavConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebdavConfig {
    pub enabled: bool,
    /// WebDAV 服务器地址，如 https://dav.example.com/voucher/
    pub url: String,
    pub username: String,
    /// 密码存 keyring（SECRET_WEBDAV_PASSWORD），配置里只存是否已设置
    pub password_set: bool,
    /// 远端保留份数（超出删最旧；0 = 不清理）
    pub remote_retention: u32,
    /// v0.8：云端每日最多上传一份（坚果云等免费服务有上传流量配额，
    /// 备份 zip 随附件增长后高频全量上传会耗尽配额；当日云端已有备份则跳过）
    pub once_daily: bool,
}

impl Default for WebdavConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            username: String::new(),
            password_set: false,
            remote_retention: 10,
            once_daily: false,
        }
    }
}

/// 打印/导出选项（v0.22 起：日报是唯一主线，卡片版式与单份导出已下线）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintConfig {
    /// 整理单标题
    pub title: String,
    /// 日报是否附带附件图片页
    pub include_images: bool,
    /// v0.8 日报模板：compact（默认，省纸：信息横幅 + 表格 38 行/页 + 竖票 3×3）
    /// | archive（档案装订：独立封面 + 表格 34 行/页 + 图片 2×2）
    pub daily_template: String,
    /// v0.8 打印是否优先使用增强版图片（仅扫描件有增强版；关闭则始终用原图）
    pub prefer_enhanced_images: bool,
}

impl Default for PrintConfig {
    fn default() -> Self {
        Self {
            title: "原始凭证".into(),
            include_images: true,
            daily_template: "compact".into(),
            prefer_enhanced_images: true,
        }
    }
}

/// v0.6.1：文档扫描矫正（7 步算法，逐项可关）
///
/// 7 个开关一一对应流水线步骤（v0.6.1 起锐化 / 去噪已实装）：
///   deskew → dewarp → autocrop → denoise → sharpen → grayscale → binarize
/// 去噪 / 锐化默认关（需真机票据评估收益后再开）。
///
/// 单步失败跳过由编排器天然兜底，关闭后该步记 off 不执行。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EnhanceConfig {
    /// 总开关；关闭后导入图片不再跑 7 步流水线
    pub enabled: bool,
    /// 步骤 1：倾斜矫正（Hough 直线）
    pub step_deskew: bool,
    /// 步骤 2：透视矫正（最大四边形）
    pub step_dewarp: bool,
    /// 步骤 3：自动裁边（去白边）
    pub step_autocrop: bool,
    /// 步骤 4：灰度化
    pub step_grayscale: bool,
    /// 步骤 5：自适应二值化
    pub step_binarize: bool,
    /// 步骤 6：锐化（Unsharp Mask，v0.6.1 实装，默认关）
    pub step_sharpen: bool,
    /// 步骤 7：去噪（中值 3×3，v0.6.1 实装，默认关）
    pub step_denoise: bool,
    /// 仅在导入「扫描件」（非截图）时走流水线；截图已接近二值图无收益
    /// 默认 false：全量走；true = 仅对扫描件走
    pub only_scans: bool,
}

impl Default for EnhanceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            step_deskew: true,
            step_dewarp: true,
            step_autocrop: true,
            step_grayscale: true,
            step_binarize: true,
            step_sharpen: false,
            step_denoise: false,
            only_scans: false,
        }
    }
}

impl AppConfig {
    /// 归一化越界字段（手改 app.json 不得破坏布局）。读取与写入两条路径都要过一遍。
    pub fn normalize(&mut self) {
        self.appearance.sidebar_width = self
            .appearance
            .sidebar_width
            .clamp(SIDEBAR_WIDTH_MIN, SIDEBAR_WIDTH_MAX);
        // 界面字号倍数：手改 app.json 可能写入 0 或 99（0 会让界面无法阅读、99 直接撑爆布局），
        // 故与侧栏宽度同口径在读取/写入两侧都收口。
        self.appearance.ui_scale = if self.appearance.ui_scale.is_finite() {
            self.appearance.ui_scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX)
        } else {
            UI_SCALE_DEFAULT
        };
        // 自动备份间隔（小时）：下限 1h、上限 30 天。
        // - 0 会让调度器每 5 分钟就判定「到期」→ 备份风暴（VACUUM + zip + 云端上传）；
        // - 超大值会走到 chrono 的 `Duration::hours` 越界 panic，而该 panic 被
        //   `spawn_blocking` 的 JoinError 吞掉 → 自动备份**永久静默失效**（不报错也不备份）。
        // `app.json` 允许手改（见模块头注释），故必须在读入时收口。2026-09-23 审计 B10。
        self.backup.interval_hours = self
            .backup
            .interval_hours
            .clamp(MIN_BACKUP_INTERVAL_HOURS, MAX_BACKUP_INTERVAL_HOURS);
    }

    /// 读取配置；文件不存在时返回默认值（并在下次保存时重建）
    pub fn load(path: &Path) -> Result<Self> {
        match jsonio::read_json::<Self>(path)? {
            Some(mut cfg) => {
                cfg.normalize();
                Ok(cfg)
            }
            None => Ok(Self::default()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        jsonio::write_json_atomic(path, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-09-23 缺陷锁（审计 B10）：自动备份间隔必须被收口。
    /// - `0` → 调度器每 5 分钟就判定「到期」→ 备份风暴（VACUUM + zip + 云端上传）；
    /// - `u64::MAX` → `interval_hours as i64` 变负 / chrono `Duration::hours` 越界 panic，
    ///   而该 panic 被 `spawn_blocking` 的 JoinError 吞掉 → **自动备份永久静默失效**。
    ///
    /// `app.json` 允许手改，故读取与写入两条路径都必须经 `normalize`（与 sidebar 同口径）。
    #[test]
    fn normalize_clamps_backup_interval() {
        let mut cfg = AppConfig::default();
        cfg.backup.interval_hours = 0;
        cfg.normalize();
        assert_eq!(cfg.backup.interval_hours, MIN_BACKUP_INTERVAL_HOURS);

        let mut cfg = AppConfig::default();
        cfg.backup.interval_hours = u64::MAX;
        cfg.normalize();
        assert_eq!(cfg.backup.interval_hours, MAX_BACKUP_INTERVAL_HOURS);

        let mut cfg = AppConfig::default();
        cfg.backup.interval_hours = 24;
        cfg.normalize();
        assert_eq!(cfg.backup.interval_hours, 24, "合法值不得被改动");
    }

    /// 界面字号倍数：默认偏大、越界与非法值收口（手改 app.json 不得让界面不可读或撑爆布局）
    #[test]
    fn normalize_clamps_ui_scale() {
        assert_eq!(
            AppConfig::default().appearance.ui_scale,
            UI_SCALE_DEFAULT,
            "默认应为大字号基线"
        );

        for (input, expect) in [
            (0.1_f32, UI_SCALE_MIN),
            (99.0, UI_SCALE_MAX),
            (1.3, 1.3),
            (f32::NAN, UI_SCALE_DEFAULT),
            (f32::INFINITY, UI_SCALE_DEFAULT),
        ] {
            let mut cfg = AppConfig::default();
            cfg.appearance.ui_scale = input;
            cfg.normalize();
            assert_eq!(
                cfg.appearance.ui_scale, expect,
                "输入 {input} 应归一化为 {expect}"
            );
        }
    }

    /// 旧配置（无 `uiScale` / `sidebarCollapsed` 字段）反序列化后应补上默认值，而不是 0（0 会让界面无法阅读）
    #[test]
    fn legacy_config_without_ui_scale_gets_default() {
        let json = r#"{"appearance":{"theme":"dark","sidebarWidth":280}}"#;
        let cfg: AppConfig = serde_json::from_str(json).expect("旧配置应能反序列化");
        assert_eq!(cfg.appearance.ui_scale, UI_SCALE_DEFAULT);
        assert_eq!(cfg.appearance.sidebar_width, 280);
        // 侧栏收起是 2026-10-16 新增字段：旧配置缺它 → 默认展开
        assert!(!cfg.appearance.sidebar_collapsed);
    }

    /// 侧栏收起（用户 2026-10-16 要求）：默认展开，按 `sidebarCollapsed` 落盘并能读回
    #[test]
    fn sidebar_collapsed_roundtrips_as_camel_case() {
        assert!(
            !AppConfig::default().appearance.sidebar_collapsed,
            "默认应为展开"
        );

        let mut cfg = AppConfig::default();
        cfg.appearance.sidebar_collapsed = true;
        let json = serde_json::to_string(&cfg).expect("应能序列化");
        assert!(json.contains("\"sidebarCollapsed\":true"), "{json}");

        let back: AppConfig = serde_json::from_str(&json).expect("应能反序列化");
        assert!(back.appearance.sidebar_collapsed);
    }
}
