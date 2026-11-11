//! 与策略无关的仅用于备份器的工具

use crate::*;
use anyhow::Context as _;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

/// 以普通用户备份文件：复制、校验一致性，并在目标旁写 `<dst>.sha256sum`。
pub fn backup_file(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }

    let meta = fs::metadata(src).with_context(|| format!("无法读取 {}", src.display()))?;
    anyhow::ensure!(meta.is_file(), "不是普通文件：{}", src.display());

    let src_hash = sha256_file(src)?;

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

/// 以 root 用户备份文件：复制、校验一致性，并在目标旁以普通用户写 `<dst>.sha256sum`。
pub fn backup_file_as_root(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }

    let mut cp_cmd = Command::new("sudo");
    cp_cmd.arg("cp").arg("--preserve").arg(src).arg(dst);
    let cp_status = cp_cmd
        .status()
        .with_context(|| format!("备份文件失败：{} → {}", src.display(), dst.display()))?;
    anyhow::ensure!(
        cp_status.success(),
        "备份文件失败（{cp_status}）：{} → {}",
        src.display(),
        dst.display(),
    );

    let cat_out = Command::new("sudo")
        .arg("cat")
        .arg(src)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("执行 sudo cat {} 失败", src.display()))?;
    anyhow::ensure!(
        cat_out.status.success(),
        "sudo cat 失败（{}）：{}",
        cat_out.status,
        src.display()
    );

    let src_hash = sha256_bytes(&cat_out.stdout);
    write_checksum(dst, &src_hash)?;

    // U 盘上的副本属于 root，读它同样要走 sudo。
    let sum_status = Command::new("sudo")
        .arg("sha256sum")
        .arg("--check")
        .arg(dst.with_added_extension("sha256sum"))
        .current_dir(dst.parent().context("目标文件没有父目录")?)
        .status()
        .with_context(|| format!("执行 sudo sha256sum {} 失败", dst.display()))?;
    anyhow::ensure!(
        sum_status.success(),
        "sudo sha256sum 失败（{}）：{}",
        sum_status,
        dst.display()
    );

    Ok(())
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
