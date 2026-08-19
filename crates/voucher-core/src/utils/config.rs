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
    pub wechat: WechatConfig,
    pub print: PrintConfig,
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

/// 凭证编号命名规则：模板支持 {date}（YYYYMMDD）与 {seq}（三位序号）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NamingConfig {
    pub template: String,
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            template: "{date}-{seq}".into(),
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
    /// 本地保留份数
    pub retention: u32,
    pub webdav: WebdavConfig,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            auto_enabled: true,
            interval_hours: 24,
            retention: 30,
            webdav: WebdavConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct WebdavConfig {
    pub enabled: bool,
    /// WebDAV 服务器地址，如 https://dav.example.com/voucher/
    pub url: String,
    pub username: String,
    /// 密码存 keyring（SECRET_WEBDAV_PASSWORD），配置里只存是否已设置
    pub password_set: bool,
}

/// 微信收件夹监听：新图片自动入库
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WechatConfig {
    pub enabled: bool,
    pub watch_dir: String,
    /// 监听模式：events（notify 事件）| poll（轮询兜底）
    pub mode: String,
}

impl Default for WechatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            watch_dir: String::new(),
            mode: "events".into(),
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
}

impl Default for PrintConfig {
    fn default() -> Self {
        Self {
            title: "凭证整理单".into(),
            card_style: "standard".into(),
            columns: 3,
            include_images: true,
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
