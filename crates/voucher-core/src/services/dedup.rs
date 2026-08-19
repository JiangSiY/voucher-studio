//! MD5 去重（对应 v4.3 的 `dedup.py`）

use std::path::Path;

use rusqlite::Connection;

use crate::db::repos::attachment_repo;
use crate::models::Attachment;
use crate::Result;

/// 文件内容 MD5（十六进制小写）
pub fn md5_file(path: &Path) -> Result<String> {
    let data = std::fs::read(path)?;
    Ok(md5_bytes(&data))
}

/// 内存数据 MD5
pub fn md5_bytes(data: &[u8]) -> String {
    format!("{:x}", md5::compute(data))
}

/// 同内容附件是否已存在（含已删除凭证之外的全部附件）
pub fn find_duplicate(conn: &Connection, hash: &str) -> Result<Option<Attachment>> {
    attachment_repo::find_by_hash(conn, hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"hello voucher").unwrap();
        assert_eq!(md5_file(&p).unwrap(), md5_bytes(b"hello voucher"));
        assert_eq!(md5_bytes(b"hello voucher").len(), 32);
    }
}
