//! 用户配置（对应 v4.3 的 `config.py`；密钥不入配置文件，走 keyring）

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::utils::jsonio;
use crate::Result;

/// 密钥在系统凭据库中的账号名（密钥不明文落盘）
pub const SECRET_OCR_API_KEY: &str = "ocr_api_key";
pub const SECRET_WEBDAV_PASSWORD: &str = "webdav_password";

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
}

/// 外观：light / dark / system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppearanceConfig {
    pub theme: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "system".into(),
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

/// OCR：本地（ort + PP-OCRv4，离线）优先；云端（HTTP API）备选
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
    /// GFS 分层保留：近 N 天每日备份全保留
    pub keep_daily: u32,
    /// GFS 分层保留：再保留 N 周的每周最新一份
    pub keep_weekly: u32,
    /// GFS 分层保留：再保留 N 月的每月最新一份
    pub keep_monthly: u32,
    /// 备用份目录（异盘双写；空 = 不启用；须指向另一物理盘/移动硬盘才有意义）
    pub secondary_dir: String,
    pub webdav: WebdavConfig,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            auto_enabled: true,
            interval_hours: 24,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            secondary_dir: String::new(),
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
}

impl Default for WebdavConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            username: String::new(),
            password_set: false,
            remote_retention: 10,
        }
    }
}

/// 打印/导出选项（语义对齐 UI 方案：标题、卡片风格、列数、省墨）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintConfig {
    /// 整理单标题
    pub title: String,
    /// 卡片风格：standard | clean | ledger
    pub card_style: String,
    /// 打印列数 1 | 2 | 3
    pub columns: u8,
    /// 凭证卡片页是否附带附件图片页
    pub include_images: bool,
    /// 按天分组导出（每天一份「凭证日报」PDF）；false = 合并单份
    pub group_by_day: bool,
}

impl Default for PrintConfig {
    fn default() -> Self {
        Self {
            title: "原始凭证".into(),
            card_style: "standard".into(),
            columns: 3,
            include_images: true,
            group_by_day: true,
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
    /// 读取配置；文件不存在或损坏时返回默认值（并在下次保存时重建）
    pub fn load(path: &Path) -> Result<Self> {
        match jsonio::read_json::<Self>(path)? {
            Some(cfg) => Ok(cfg),
            None => Ok(Self::default()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        jsonio::write_json_atomic(path, self)
    }
}
