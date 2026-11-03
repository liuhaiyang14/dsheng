//! 输出工具：envelope 序列化与安全文本写文件（导出用）。

use crate::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;

/// 写输出文件（导出路径由用户 --out 给出；父目录必须已存在或可创建）。
pub fn write_out_file(path: &str, bytes: &[u8]) -> CliResult<u64> {
    let target = std::path::Path::new(path);
    if let Some(parent) = target.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CliError::new(CliErrorCode::ExportFailed, format!("输出目录创建失败: {e}"))
            })?;
        }
    }
    std::fs::write(target, bytes)
        .map_err(|e| CliError::new(CliErrorCode::ExportFailed, format!("输出文件写入失败: {e}")))?;
    Ok(bytes.len() as u64)
}

/// 导出成功后的 stdout envelope data（不含本机绝对路径内容）。
pub fn export_written(path_label: &str, bytes: u64) -> serde_json::Value {
    serde_json::json!({
        "written": true,
        "out": path_label,
        "bytes": bytes,
    })
}
