//! 备份器：动作的备份实现与文件工具

use crate::{RunAs::*, *};
use anyhow::Context as _;
use log::*;
use std::fs;
use std::path::Path;

/// 以普通用户备份文件：复制、校验一致性，并在目标旁写 `<dst>.sha256sum`。
pub fn backup_file(src: &Path, dst: &Path) -> anyhow::Result<()> {
    RunAsNormal.backup_file(src, dst)
}

impl Action {
    /// 把该动作需要备份的数据复制到 U 盘。
    ///
    /// - `log_target`：日志里标识「任务/步骤序号-动作名」；
    /// - `backup_dir`：U 盘上的备份根目录。
    pub fn backup(&self, log_target: &str, backup_dir: &Path) -> anyhow::Result<()> {
        match self {
            // 用户数据：从系统目标路径收进 U 盘，保持 files 里的相对路径。
            Action::CopyFiles {
                dest_dir,
                files,
                run_as_root,
            } => {
                let run_as = if *run_as_root { RunAsRoot } else { RunAsNormal };
                for file in files {
                    let base = Path::new(file)
                        .file_name()
                        .with_context(|| format!("无法从 {file} 取文件名"))?;
                    let src = dest_dir.join(base);
                    let dst = backup_dir.join(file);
                    run_as.backup_file(&src, &dst)?;
                    info!(target: log_target, "{} → {}", src.display(), dst.display());
                }
            }

            // 本地 rpm：放到恢复器根目录，文件名不变。
            Action::DnfInstall { local_rpms, .. } => {
                for rpm in local_rpms {
                    let base = rpm
                        .file_name()
                        .with_context(|| format!("无法从 {} 取文件名", rpm.display()))?;
                    let dst = backup_dir.join(base);
                    RunAsNormal.backup_file(rpm, &dst)?;
                    info!(target: log_target, "{} → {}", rpm.display(), dst.display());
                }
            }

            // patch 是数据：用 diff 生成 backup → dest 的差异，
            // 写入 `${backup_dir}/${patch}`。
            Action::PatchFile {
                patch,
                dest,
                backup,
                run_as_root,
            } => {
                let run_as = if *run_as_root { RunAsRoot } else { RunAsNormal };
                let name = dest
                    .file_name()
                    .with_context(|| format!("无法从 {} 取文件名", dest.display()))?
                    .to_string_lossy()
                    .into_owned();

                let output = run_as
                    .command("diff")
                    .arg("-u")
                    .arg("--label")
                    .arg(format!("a/{name}"))
                    .arg("--label")
                    .arg(format!("b/{name}"))
                    .arg(backup)
                    .arg(dest)
                    .output()
                    .with_context(|| {
                        format!("执行 diff（{} - {}）失败", dest.display(), backup.display())
                    })?;

                match output.status.code() {
                    // 0 = 无差异，1 = 有差异，都算成功。
                    Some(0) | Some(1) => {}
                    _ => anyhow::bail!(
                        "生成 patch 失败（{} - {}）：{}",
                        dest.display(),
                        backup.display(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                }

                let dst = backup_dir.join(patch);
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("创建 {} 失败", parent.display()))?;
                }
                fs::write(&dst, &output.stdout)
                    .with_context(|| format!("写入 {} 失败", dst.display()))?;
                info!(target: log_target, "{patch}");
            }

            // 其余动作没有需要备份的数据。
            _ => {}
        }

        Ok(())
    }
}
