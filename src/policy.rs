//! 策略文件的结构、解析与 schema 校验。

use crate::*;
use anyhow::Context as _;
use log::*;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// 内嵌的策略 schema（JSON Schema draft 2020-12，YAML 形式）。
///
/// 内嵌而不是读取外部文件：U 盘上只有 policy.toml，没有 doc/ 目录；
/// 备份器与恢复器都应当自带同一份 schema。
const POLICY_SCHEMA: &str = include_str!("../doc/policy.schema.yaml");

/// 策略文件的结构。
#[derive(Debug, Deserialize)]
pub struct Policy {
    pub tasks: Vec<Task>,
}

/// 一个任务，对应策略文件中的 `[[tasks]]`。
#[derive(Debug, Deserialize)]
pub struct Task {
    pub id: String,
    #[serde(rename = "display-name")]
    pub display_name: String,
    pub steps: Vec<Action>,
}

/// 一个步骤，对应 schema 里 `action.oneOf` 的九种动作。
///
/// 用 TOML 的 `id` 字段做标签区分变体。
#[derive(Debug, Deserialize)]
#[serde(tag = "id", rename_all = "kebab-case")]
pub enum Action {
    /// 把文件复制到目标目录（恢复视角）。
    #[serde(rename_all = "kebab-case")]
    CopyFiles {
        dest_dir: PathBuf,
        files: Vec<String>,
        #[serde(default)]
        run_as_root: bool,
    },

    /// 用 dnf 安装包，或安装磁盘上的本地 rpm。
    #[serde(rename_all = "kebab-case")]
    DnfInstall {
        #[serde(default)]
        packages: Vec<String>,
        #[serde(default)]
        local_rpms: Vec<PathBuf>,
    },

    /// 执行 python 脚本，并用另一个脚本校验。
    #[serde(rename_all = "kebab-case")]
    Python {
        exec: String,
        verify: String,
        #[serde(default)]
        run_as_root: bool,
    },

    /// 给目标文件打 patch。
    #[serde(rename_all = "kebab-case")]
    PatchFile {
        patch: String,
        dest: PathBuf,
        backup: PathBuf,
        #[serde(default)]
        run_as_root: bool,
    },

    /// 启用第三方 dnf 仓库。
    #[serde(rename_all = "kebab-case")]
    DnfRepo {
        repo_id: String,
        baseurl: Option<String>,
        gpgkey: Option<String>,
        #[serde(default)]
        release_rpms: Vec<String>,
    },

    /// 设置静态主机名。
    #[serde(rename_all = "kebab-case")]
    SetHostname { hostname: String },

    /// 重启机器。
    #[serde(rename_all = "kebab-case")]
    Reboot {
        prompt: Option<String>,
        delay: Option<f64>,
    },

    /// 人工确认。
    #[serde(rename_all = "kebab-case")]
    ManualCheck { prompt: String },

    /// 创建软链接。
    #[serde(rename_all = "kebab-case")]
    SoftLink {
        dest: PathBuf,
        src: PathBuf,
        #[serde(default)]
        run_as_root: bool,
    },
}

impl Action {
    /// 动作名，与策略文件里的 `id` 一致。
    pub fn id(&self) -> &'static str {
        match self {
            Action::CopyFiles { .. } => "copy-files",
            Action::DnfInstall { .. } => "dnf-install",
            Action::Python { .. } => "python",
            Action::PatchFile { .. } => "patch-file",
            Action::DnfRepo { .. } => "dnf-repo",
            Action::SetHostname { .. } => "set-hostname",
            Action::Reboot { .. } => "reboot",
            Action::ManualCheck { .. } => "manual-check",
            Action::SoftLink { .. } => "soft-link",
        }
    }
}

impl Policy {
    pub fn load(path: &Path) -> anyhow::Result<Policy> {
        let content =
            fs::read_to_string(path).with_context(|| format!("读取 {} 失败", path.display()))?;
        let value: toml::Value =
            toml::from_str(&content).with_context(|| format!("解析 {} 失败", path.display()))?;

        validate_value(&value).with_context(|| format!("{} 不符合 schema", path.display()))?;

        value
            .try_into()
            .with_context(|| format!("{} 的结构转换失败", path.display()))
    }
}

/// 读取策略文件，先用 schema 校验，再反序列化为强类型结构。
/// 用内嵌 schema 校验原始 TOML 结构。
///
/// 校验的是原始内容而不是反序列化后的 `Policy`，
/// 这样默认值、可选字段等都能按 schema 的语义检查。
fn validate_value(value: &toml::Value) -> anyhow::Result<()> {
    let schema: serde_json::Value =
        rust_yaml::from_str(POLICY_SCHEMA).context("解析内嵌 schema 失败")?;
    let validator = jsonschema::validator_for(&schema).context("构建 schema 校验器失败")?;

    let instance = serde_json::to_value(value).context("策略转 JSON 失败")?;
    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();

    if !errors.is_empty() {
        anyhow::bail!("\n{}", errors.join("\n"));
    }

    Ok(())
}

impl Action {
    /// 把该动作需要备份的数据复制到 U 盘。
    ///
    /// - `log_target`：日志里标识「任务/步骤序号-动作名」。
    /// - `backup_dir`：U 盘上的备份根目录；
    /// - `repo_root`：策略资产所在目录，脚本与 patch 从这里取；
    pub fn backup(
        &self,
        log_target: &str,
        backup_dir: &Path,
        repo_root: &Path,
    ) -> anyhow::Result<()> {
        match self {
            // 用户数据：从系统目标路径收进 U 盘，保持 files 里的相对路径。
            Action::CopyFiles {
                dest_dir, files, ..
            } => {
                for file in files {
                    let base = Path::new(file)
                        .file_name()
                        .with_context(|| format!("无法从 {file} 取文件名"))?;
                    let src = dest_dir.join(base);
                    let dst = backup_dir.join(file);
                    copy_file(&src, &dst)?;
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
                    copy_file(rpm, &dst)?;
                    info!(target: log_target, "{} → {}", rpm.display(), dst.display());
                }
            }

            // 策略资产：脚本随恢复器分发。
            Action::Python { exec, verify, .. } => {
                for rel in [exec, verify] {
                    let src = repo_root.join(rel);
                    let dst = backup_dir.join(rel);
                    copy_file(&src, &dst)?;
                    info!(target: log_target, "{} → {}", src.display(), dst.display());
                }
            }

            // patch 是数据：用 diff 生成 backup → dest 的差异，
            // 写入 `${backup_dir}/${patch}`。
            Action::PatchFile {
                patch,
                dest,
                backup,
                ..
            } => {
                let name = dest
                    .file_name()
                    .with_context(|| format!("无法从 {} 取文件名", dest.display()))?
                    .to_string_lossy()
                    .into_owned();

                let output = std::process::Command::new("diff")
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
