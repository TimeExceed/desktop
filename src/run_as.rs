use anyhow::Context as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunAs {
    RunAsNormal,
    RunAsRoot,
}

impl RunAs {
    /// 以 `run_as` 的身份构造命令：root 时经 `sudo` 调用 `program`。
    pub(crate) fn command(&self, program: &str) -> Command {
        match self {
            Self::RunAsRoot => {
                let mut cmd = Command::new("sudo");
                cmd.arg(program);
                cmd
            }
            Self::RunAsNormal => Command::new(program),
        }
    }

    pub(crate) fn sha256_file(&self, path: &Path) -> anyhow::Result<String> {
        let output = self
            .command("sha256sum")
            .arg("--binary")
            .arg("--")
            .arg(path)
            .stderr(Stdio::inherit())
            .output()
            .with_context(|| format!("执行 sha256sum {} 失败", path.display()))?;

        anyhow::ensure!(
            output.status.success(),
            "sha256sum 失败（{}）：{}",
            output.status,
            path.display()
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let hex = stdout
            .split_whitespace()
            .next()
            .with_context(|| format!("sha256sum 输出为空：{}", path.display()))?;

        Ok(hex.to_string())
    }

    pub(crate) fn exists(&self, path: &Path) -> anyhow::Result<bool> {
        let status = self
            .command("test")
            .arg("-e")
            .arg(path)
            .status()
            .with_context(|| format!("执行 test -e {} 失败", path.display()))?;

        Ok(status.success())
    }

    /// 读取符号链接的目标。
    ///
    /// `Ok(None)` 表示路径不存在；路径存在但不是符号链接时返回 `Err`。
    /// 走 `test` / `readlink` 而不是 `fs::read_link`，是为了在 root 文件上
    /// 也能工作（普通用户可能连父目录都进不去）。
    pub(crate) fn link_target(&self, path: &Path) -> anyhow::Result<Option<PathBuf>> {
        let is_link = self
            .command("test")
            .arg("-L")
            .arg(path)
            .status()
            .with_context(|| format!("执行 test -L {} 失败", path.display()))?
            .success();

        if !is_link {
            anyhow::ensure!(!self.exists(path)?, "{} 已存在且不是软链接", path.display());
            return Ok(None);
        }

        let output = self
            .command("readlink")
            .arg("--")
            .arg(path)
            .stderr(Stdio::inherit())
            .output()
            .with_context(|| format!("执行 readlink {} 失败", path.display()))?;
        anyhow::ensure!(
            output.status.success(),
            "readlink 失败（{}）：{}",
            output.status,
            path.display()
        );

        let target = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Some(PathBuf::from(target)))
    }

    /// 比较两个文件内容是否一致（用 SHA-256）；目标不存在时返回 `false`。
    ///
    /// `run_as` 同时作用于 `src` 与 `dst`，前提是两者读取权限一致：
    /// 备份副本由 `RunAs::backup_file` 生成时会保留原文件的 root 属主与权限，
    /// 所以 root 文件与它的副本总是同属 root。
    pub(crate) fn same_content(&self, src: &Path, dst: &Path) -> anyhow::Result<bool> {
        if !self.exists(dst)? {
            return Ok(false);
        }

        Ok(self.sha256_file(src)? == self.sha256_file(dst)?)
    }

    /// 判断 patch 是否已经应用到目标文件（反向 dry-run 成功即为已应用）。
    pub(crate) fn patch_applied(&self, patch: &Path, dest: &Path) -> anyhow::Result<bool> {
        let status = self
            .command("patch")
            .arg("-R")
            .arg("--dry-run")
            .arg("-i")
            .arg(patch)
            .arg(dest)
            .stdout(Stdio::null())
            .status()
            .with_context(|| format!("执行 patch 检查失败：{}", dest.display()))?;

        Ok(status.success())
    }

    /// 用 `install`（必要时加 `sudo`）把备份文件恢复到目标位置。
    ///
    /// * `-D` 会创建父目录。
    /// * `-p` 保留源文件的时间戳与权限。
    pub(crate) fn restore_file(&self, src: &Path, dst: &Path) -> anyhow::Result<()> {
        let status = self
            .command("install")
            .arg("-D")
            .arg("-p")
            .arg(src)
            .arg(dst)
            .status()
            .with_context(|| format!("执行 install 失败：{} → {}", src.display(), dst.display()))?;

        anyhow::ensure!(
            status.success(),
            "install 失败（{status}）：{} → {}",
            src.display(),
            dst.display()
        );

        Ok(())
    }

    pub(crate) fn backup_file(&self, src: &Path, dst: &Path) -> anyhow::Result<()> {
        // 目的地是 U 盘，目录归当前用户：必须用普通用户创建，
        // 否则后面以普通用户写 `.sha256sum` 会被拒绝。
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建 {} 失败", parent.display()))?;
        }

        let is_file = self
            .command("test")
            .arg("-f")
            .arg(src)
            .status()
            .with_context(|| format!("执行 test -f {} 失败", src.display()))?
            .success();
        anyhow::ensure!(is_file, "不是普通文件或不存在：{}", src.display());

        let cp_status = self
            .command("cp")
            .arg("--preserve")
            .arg(src)
            .arg(dst)
            .status()
            .with_context(|| format!("备份文件失败：{} → {}", src.display(), dst.display()))?;
        anyhow::ensure!(
            cp_status.success(),
            "备份文件失败（{cp_status}）：{} → {}",
            src.display(),
            dst.display(),
        );

        let src_chksum = self.sha256_file(src)?;
        let dst_chksum = self.sha256_file(dst)?;
        anyhow::ensure!(
            src_chksum == dst_chksum,
            "复制后校验失败：{}（{src_chksum}）与 {}（{dst_chksum}）不一致",
            src.display(),
            dst.display()
        );

        write_checksum(dst, &src_chksum)
    }

    pub(crate) fn make_dirs(&self, path: &Path) -> anyhow::Result<()> {
        let status = self.command("install").arg("-d").arg(path).status()?;
        anyhow::ensure!(status.success(), "创建目录失败：{}", path.display());
        Ok(())
    }
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
