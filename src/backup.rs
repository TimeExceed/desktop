//! 与策略无关的仅用于备份器的工具

use crate::*;
use anyhow::Context as _;
use std::fs;
use std::path::{Path, PathBuf};

/// policy.toml 所在目录，即策略资产（脚本、patch）的根。
pub fn repo_root(policy_path: &Path) -> PathBuf {
    policy_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

/// 备份文件：复制、校验一致性，并在目标旁写 `<dst>.sha256sum`。
pub fn backup_file(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let meta = fs::metadata(src).with_context(|| format!("无法读取 {}", src.display()))?;
    anyhow::ensure!(meta.is_file(), "不是普通文件：{}", src.display());

    let src_hash = sha256_file(src)?;

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }

    fs::copy(src, dst)
        .with_context(|| format!("复制 {} → {} 失败", src.display(), dst.display()))?;
    fs::set_permissions(dst, meta.permissions())
        .with_context(|| format!("设置 {} 权限失败", dst.display()))?;

    let dst_hash = sha256_file(dst)?;
    anyhow::ensure!(
        src_hash == dst_hash,
        "复制后校验失败：{}（{src_hash}）与 {}（{dst_hash}）不一致",
        src.display(),
        dst.display()
    );

    write_checksum(dst, &dst_hash)
}

/// 在目标文件旁写标准 `sha256sum` 格式的校验文件。
fn write_checksum(path: &Path, hash: &str) -> anyhow::Result<()> {
    let name = path
        .file_name()
        .with_context(|| format!("无法从 {} 取文件名", path.display()))?
        .to_string_lossy();

    let sum_path = path.with_added_extension("sha256sum");

    fs::write(&sum_path, format!("{hash}  {name}\n"))
        .with_context(|| format!("写入 {} 失败", sum_path.display()))?;

    Ok(())
}
