use crate::backup::*;
use crate::restore::*;
use crate::*;
use anyhow::Context as _;
use log::*;
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

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
                    backup_file(&src, &dst)?;
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
                    backup_file(rpm, &dst)?;
                    info!(target: log_target, "{} → {}", rpm.display(), dst.display());
                }
            }

            // 策略资产：脚本随恢复器分发。
            Action::Python { exec, verify, .. } => {
                for rel in [exec, verify] {
                    let src = repo_root.join(rel);
                    let dst = backup_dir.join(rel);
                    backup_file(&src, &dst)?;
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

                let output = Command::new("diff")
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

    /// 把该动作需要恢复的数据从 U 盘写回系统。
    ///
    /// 目前只实现 `copy-files`；其余动作暂不处理。
    pub fn restore(&self, ctx: &StepContext) -> anyhow::Result<()> {
        match self {
            Action::CopyFiles {
                dest_dir,
                files,
                run_as_root,
            } => {
                for file in files {
                    let base = Path::new(file)
                        .file_name()
                        .with_context(|| format!("无法从 {file} 取文件名"))?;
                    let src = ctx.backup_dir.join(file);
                    let dst = dest_dir.join(base);

                    // 幂等：内容已一致就跳过。
                    if same_content(&src, &dst)? {
                        info!(target: &ctx.log_target, "{} 已是最新", dst.display());
                        continue;
                    }

                    restore_file(&src, &dst, *run_as_root, ctx)?;
                    info!(target: &ctx.log_target, "{} → {}", src.display(), dst.display());
                }
            }

            // 安装 dnf 包，或安装回传到 U 盘的本地 rpm。
            Action::DnfInstall {
                packages,
                local_rpms,
            } => {
                // 先把本地 rpm 恢复到策略里的原路径。
                for rpm in local_rpms {
                    let base = rpm
                        .file_name()
                        .with_context(|| format!("无法从 {} 取文件名", rpm.display()))?;
                    let src = ctx.backup_dir.join(base);

                    if !same_content(&src, rpm)? {
                        restore_file(&src, rpm, false, ctx)?;
                        info!(target: &ctx.log_target, "{} → {}", src.display(), rpm.display());
                    }
                }

                // 只把尚未安装的包交给 dnf。
                let mut pending: Vec<String> = Vec::new();
                for pkg in packages {
                    if !dnf_installed(pkg)? {
                        pending.push(pkg.clone());
                    }
                }
                for rpm in local_rpms {
                    pending.push(rpm.to_string_lossy().into_owned());
                }

                if pending.is_empty() {
                    info!(target: &ctx.log_target, "所需的包均已安装");
                } else {
                    let mut cmd = Command::new("sudo");
                    cmd.arg("dnf").arg("install").arg("-y").args(&pending);

                    let status = ctx
                        .run(&mut cmd)
                        .with_context(|| format!("执行 dnf install 失败：{}", pending.join(" ")))?;
                    anyhow::ensure!(
                        status.success(),
                        "dnf install 失败（{status}）：{}",
                        pending.join(" ")
                    );

                    info!(target: &ctx.log_target, "安装 {}", pending.join(" "));
                }
            }

            // 设置静态主机名。
            Action::SetHostname { hostname } => {
                let current = current_hostname()?;

                if current == *hostname {
                    info!(target: &ctx.log_target, "主机名已是 {hostname}");
                } else {
                    let mut cmd = Command::new("sudo");
                    cmd.arg("hostnamectl").arg("set-hostname").arg(hostname);

                    let status = ctx
                        .run(&mut cmd)
                        .with_context(|| format!("执行 hostnamectl 失败：{hostname}"))?;
                    anyhow::ensure!(status.success(), "设置主机名失败（{status}）：{hostname}");

                    info!(target: &ctx.log_target, "主机名 {current} → {hostname}");
                }
            }

            // 重启系统；重启后再次运行恢复器即可从下一步继续。
            Action::Reboot { prompt, delay } => {
                let boot_id_file = ctx.state_dir.join("boot-id");
                let current = boot_id()?;

                // 状态文件记录的是上次请求重启时的 boot-id；
                // 与当前不同，说明重启已经发生过。
                if let Ok(prev) = fs::read_to_string(&boot_id_file) {
                    if prev.trim() != current {
                        info!(target: &ctx.log_target, "重启已完成");
                        return Ok(());
                    }
                }

                fs::write(&boot_id_file, &current)
                    .with_context(|| format!("写入 {} 失败", boot_id_file.display()))?;

                if let Some(p) = prompt {
                    info!(target: &ctx.log_target, "{p}");
                }

                let delay = delay.unwrap_or(30.0);
                if delay > 0.0 {
                    info!(target: &ctx.log_target, "{delay} 秒后重启");
                    sleep(Duration::from_secs_f64(delay));
                }

                let mut cmd = Command::new("sudo");
                cmd.arg("systemctl").arg("reboot");

                let status = ctx
                    .run(&mut cmd)
                    .with_context(|| "执行 systemctl reboot 失败")?;
                anyhow::ensure!(status.success(), "重启命令失败（{status}）");

                info!(target: &ctx.log_target, "已请求重启，等待系统关闭…");

                // 等系统杀掉本进程；正常情况下不会走到 loop 之后。
                loop {
                    sleep(Duration::from_secs(3600));
                }
            }

            // 人工确认：显示提示，用户否认即中止恢复。
            Action::ManualCheck { prompt } => {
                let confirmed_file = ctx.state_dir.join("confirmed");

                // 幂等：确认过就不再打扰。
                if confirmed_file.exists() {
                    info!(target: &ctx.log_target, "已确认过，跳过");
                    return Ok(());
                }

                info!(target: &ctx.log_target, "{prompt}");
                print!("是否已完成？[y/N] ");
                io::stdout().flush().context("刷新标准输出失败")?;

                let mut answer = String::new();
                io::stdin()
                    .read_line(&mut answer)
                    .context("读取标准输入失败")?;

                match answer.trim().to_ascii_lowercase().as_str() {
                    "y" | "yes" => {
                        fs::write(&confirmed_file, "yes\n")
                            .with_context(|| format!("写入 {} 失败", confirmed_file.display()))?;
                        info!(target: &ctx.log_target, "已确认");
                    }
                    _ => anyhow::bail!("用户未确认：{prompt}"),
                }
            }

            // 添加 dnf 仓库：安装 release rpm，或写入 .repo 文件。
            Action::DnfRepo {
                repo_id,
                baseurl,
                gpgkey,
                release_rpms,
            } => {
                // 1. release-rpms：直接安装仓库 rpm（$releasever 交给 dnf 展开）。
                if !release_rpms.is_empty() && !repo_enabled(repo_id)? {
                    let mut cmd = std::process::Command::new("sudo");
                    cmd.arg("dnf").arg("install").arg("-y").args(release_rpms);

                    let status = ctx
                        .run(&mut cmd)
                        .with_context(|| format!("安装仓库 rpm 失败：{repo_id}"))?;
                    anyhow::ensure!(status.success(), "安装仓库 rpm 失败（{status}）：{repo_id}");

                    info!(target: &ctx.log_target, "已添加仓库 {repo_id}");
                }

                // 2. baseurl：写 /etc/yum.repos.d/<repo-id>.repo。
                if let Some(url) = baseurl {
                    let mut content = format!(
                        "[{repo_id}]\nname={repo_id}\nbaseurl={url}\nenabled=1\ngpgcheck=1\n"
                    );
                    if let Some(key) = gpgkey {
                        content.push_str(&format!("gpgkey={key}\n"));
                    }

                    let repo_file =
                        PathBuf::from("/etc/yum.repos.d").join(format!("{repo_id}.repo"));
                    let staged = ctx.state_dir.join(format!("{repo_id}.repo"));
                    fs::write(&staged, content)
                        .with_context(|| format!("写入 {} 失败", staged.display()))?;

                    if same_content(&staged, &repo_file)? {
                        info!(target: &ctx.log_target, "仓库配置已是最新：{repo_id}");
                    } else {
                        restore_file(&staged, &repo_file, true, ctx)?;
                        info!(target: &ctx.log_target, "已写入 {}", repo_file.display());
                    }
                }
            }

            // 给目标文件打 patch；已打过就跳过。
            Action::PatchFile {
                patch,
                dest,
                backup,
                run_as_root,
            } => {
                let patch_file = ctx.backup_dir.join(patch);

                // 反向 dry-run 成功说明补丁已经打上。
                if patch_applied(&patch_file, dest)? {
                    info!(target: &ctx.log_target, "patch 已应用：{}", dest.display());
                    return Ok(());
                }

                // 刷新基准：把当前文件存一份到 backup。
                restore_file(dest, backup, *run_as_root, ctx)
                    .with_context(|| format!("备份基准失败：{}", backup.display()))?;

                // 打 patch。
                let mut cmd = if *run_as_root {
                    let mut c = std::process::Command::new("sudo");
                    c.arg("patch");
                    c
                } else {
                    std::process::Command::new("patch")
                };
                cmd.arg("-i").arg(&patch_file).arg(dest);

                let status = ctx
                    .run(&mut cmd)
                    .with_context(|| format!("打 patch 失败：{}", dest.display()))?;
                anyhow::ensure!(
                    status.success(),
                    "打 patch 失败（{status}）：{}",
                    dest.display()
                );

                info!(target: &ctx.log_target, "已打 patch：{}", dest.display());
            }

            // 创建软链接：不存在则创建，已指向目标视为完成，其余情况报错。
            Action::SoftLink {
                dest,
                src,
                run_as_root,
            } => {
                match fs::symlink_metadata(dest) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        let current = fs::read_link(dest)
                            .with_context(|| format!("读取链接失败：{}", dest.display()))?;

                        if current == *src {
                            info!(target: &ctx.log_target, "软链接已存在：{}", dest.display());
                            return Ok(());
                        }

                        anyhow::bail!(
                            "{} 已指向 {}，与期望的 {} 不符",
                            dest.display(),
                            current.display(),
                            src.display()
                        );
                    }
                    Ok(_) => {
                        anyhow::bail!("{} 已存在且不是软链接", dest.display());
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(e).with_context(|| format!("无法访问 {}", dest.display()));
                    }
                }

                // 父目录不存在就先建出来。
                if let Some(parent) = dest.parent() {
                    if !parent.exists() {
                        if *run_as_root {
                            let mut cmd = std::process::Command::new("sudo");
                            cmd.arg("install").arg("-d").arg(parent);

                            let status = ctx
                                .run(&mut cmd)
                                .with_context(|| format!("创建目录失败：{}", parent.display()))?;
                            anyhow::ensure!(
                                status.success(),
                                "创建目录失败（{status}）：{}",
                                parent.display()
                            );
                        } else {
                            fs::create_dir_all(parent)
                                .with_context(|| format!("创建 {} 失败", parent.display()))?;
                        }
                    }
                }

                let mut cmd = if *run_as_root {
                    let mut c = std::process::Command::new("sudo");
                    c.arg("ln");
                    c
                } else {
                    std::process::Command::new("ln")
                };
                cmd.arg("-s").arg(src).arg(dest);

                let status = ctx
                    .run(&mut cmd)
                    .with_context(|| format!("创建软链接失败：{}", dest.display()))?;
                anyhow::ensure!(
                    status.success(),
                    "创建软链接失败（{status}）：{} → {}",
                    dest.display(),
                    src.display()
                );

                info!(target: &ctx.log_target, "已创建软链接 {}", dest.display());
            }

            // 执行 python 脚本：verify 通过即视为完成，否则运行 exec。
            Action::Python {
                exec,
                verify,
                run_as_root,
            } => {
                let exec_script = ctx.backup_dir.join(exec);
                let verify_script = ctx.backup_dir.join(verify);

                // 1. 验证脚本通过说明这一步已经做过。
                let mut verify_cmd = std::process::Command::new("python3");
                verify_cmd.arg(&verify_script);

                let status = ctx
                    .run(&mut verify_cmd)
                    .with_context(|| format!("执行验证脚本失败：{verify}"))?;

                if status.success() {
                    info!(target: &ctx.log_target, "{verify} 通过，跳过");
                    return Ok(());
                }

                // 2. 执行安装脚本。
                let mut exec_cmd = if *run_as_root {
                    let mut c = std::process::Command::new("sudo");
                    c.arg("python3");
                    c
                } else {
                    std::process::Command::new("python3")
                };
                exec_cmd.arg(&exec_script);

                let status = ctx
                    .run(&mut exec_cmd)
                    .with_context(|| format!("执行脚本失败：{exec}"))?;
                anyhow::ensure!(status.success(), "脚本失败（{status}）：{exec}");

                info!(target: &ctx.log_target, "已执行 {exec}");
            }

            // 其余动作尚未实现恢复。
            _ => {}
        }

        Ok(())
    }
}
