//! 与策略无关的文件操作，以及每种动作的备份实现。

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

/// 复制普通文件，并保留权限位。
pub fn copy_file(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let meta = fs::metadata(src).with_context(|| format!("无法读取 {}", src.display()))?;
    anyhow::ensure!(meta.is_file(), "不是普通文件：{}", src.display());

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }

    fs::copy(src, dst)
        .with_context(|| format!("复制 {} → {} 失败", src.display(), dst.display()))?;
    fs::set_permissions(dst, meta.permissions())
        .with_context(|| format!("设置 {} 权限失败", dst.display()))?;

    Ok(())
}
