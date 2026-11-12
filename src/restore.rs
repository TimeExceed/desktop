//! 恢复器：动作的恢复实现与工具函数

use crate::{RunAs::*, *};
use anyhow::Context as _;
use log::info;
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::Duration;

impl Action {
    /// 把该动作需要恢复的数据从 U 盘写回系统。
    pub fn restore(&self, ctx: &StepContext) -> anyhow::Result<()> {
        match self {
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
                    let src = ctx.backup_dir.join(file);
                    let dst = dest_dir.join(base);

                    // 幂等：内容已一致就跳过。
                    if run_as.same_content(&src, &dst)? {
                        info!(target: &ctx.log_target, "{} 已是最新", dst.display());
                        continue;
                    }

                    run_as.restore_file(&src, &dst)?;
                    info!(target: &ctx.log_target, "{} → {}", src.display(), dst.display());
                }
            }

            // 安装 dnf 包，或安装回传到 U 盘的本地 rpm。
            Action::DnfInstall {
                packages,
                local_rpms,
                exclude,
                setopt,
                allowerasing,
            } => {
                // 先把本地 rpm 恢复到策略里的原路径。
                for rpm in local_rpms {
                    let base = rpm
                        .file_name()
                        .with_context(|| format!("无法从 {} 取文件名", rpm.display()))?;
                    let src = ctx.backup_dir.join(base);

                    if !RunAsNormal.same_content(&src, rpm)? {
                        RunAsNormal.restore_file(&src, rpm)?;
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
                    cmd.arg("dnf").arg("install").arg("-y");

                    for s in setopt {
                        cmd.arg(format!("--setopt={s}"));
                    }
                    for e in exclude {
                        cmd.arg(format!("--exclude={e}"));
                    }
                    if *allowerasing {
                        cmd.arg("--allowerasing");
                    }

                    cmd.args(&pending);

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
                if let Ok(prev) = fs::read_to_string(&boot_id_file)
                    && prev.trim() != current
                {
                    info!(target: &ctx.log_target, "重启已完成");
                    return Ok(());
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

                    // 与写入侧一致用 root 读取：/etc/yum.repos.d 下可能已有
                    // 普通用户读不到的旧文件。
                    if RunAsRoot.same_content(&staged, &repo_file)? {
                        info!(target: &ctx.log_target, "仓库配置已是最新：{repo_id}");
                    } else {
                        RunAsRoot.restore_file(&staged, &repo_file)?;
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
                let run_as = if *run_as_root { RunAsRoot } else { RunAsNormal };
                let patch_file = ctx.backup_dir.join(patch);

                // 反向 dry-run 成功说明补丁已经打上。
                if run_as.patch_applied(&patch_file, dest)? {
                    info!(target: &ctx.log_target, "patch 已应用：{}", dest.display());
                    return Ok(());
                }

                // 刷新基准：把当前文件存一份到 backup。
                run_as
                    .restore_file(dest, backup)
                    .with_context(|| format!("备份基准失败：{}", backup.display()))?;

                // 打 patch。
                let mut cmd = run_as.command("patch");
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
                let run_as = if *run_as_root { RunAsRoot } else { RunAsNormal };

                match run_as.link_target(dest)? {
                    Some(current) if current == *src => {
                        info!(target: &ctx.log_target, "软链接已存在：{}", dest.display());
                        return Ok(());
                    }
                    Some(current) => anyhow::bail!(
                        "{} 已指向 {}，与期望的 {} 不符",
                        dest.display(),
                        current.display(),
                        src.display()
                    ),
                    None => {}
                }

                // 父目录不存在就先建出来。
                if let Some(parent) = dest.parent()
                    && !run_as.exists(parent)?
                {
                    run_as.make_dirs(parent)?;
                }

                let mut cmd = run_as.command("ln");
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

            // 刷新字体缓存；fc-cache 本身幂等，直接执行。
            Action::UpdateFcCache => {
                let mut cmd = Command::new("sudo");
                cmd.arg("fc-cache").arg("-fv");

                let status = ctx.run(&mut cmd).with_context(|| "执行 fc-cache 失败")?;
                anyhow::ensure!(status.success(), "fc-cache 失败（{status}）");

                info!(target: &ctx.log_target, "已刷新字体缓存");
            }

            // 自动挑选数据分区，写入 fstab 并挂载。
            Action::Mount {
                mount_point,
                min_size_gb,
                options,
            } => {
                mount_disk(ctx, mount_point, *min_size_gb, options)?;
            }
        }

        Ok(())
    }
}

/// 单个步骤的执行上下文。
///
/// 框架在执行每个步骤前准备好状态目录。
/// 步骤内启动的子进程直接继承终端的 stdout/stderr，不重定向：
/// 否则 `sudo` 的密码提示（走 stderr）就看不见了。
pub struct StepContext {
    /// 日志前缀：`{task}/{step-index}-{step-id}`。
    pub log_target: String,
    /// U 盘上的备份根目录。
    pub backup_dir: PathBuf,
    /// 本步骤的状态目录。
    pub state_dir: PathBuf,
}

impl StepContext {
    /// 创建状态目录。
    pub fn new(
        log_target: String,
        backup_dir: PathBuf,
        state_dir: PathBuf,
    ) -> anyhow::Result<Self> {
        fs::create_dir_all(&state_dir)
            .with_context(|| format!("创建状态目录 {} 失败", state_dir.display()))?;

        Ok(Self {
            log_target,
            backup_dir,
            state_dir,
        })
    }

    /// 执行命令，stdout/stderr 直接继承终端（`sudo` 的密码提示因此可见）。
    pub fn run(&self, cmd: &mut Command) -> anyhow::Result<ExitStatus> {
        let status = cmd.status().context("启动子进程失败")?;
        Ok(status)
    }
}

/// 判断一个包或组是否已安装。
///
/// `@name` 形式按 dnf 组处理，其余按 rpm 包名处理。
pub fn dnf_installed(pkg: &str) -> anyhow::Result<bool> {
    if let Some(group) = pkg.strip_prefix('@') {
        let output = Command::new("dnf")
            .args(["group", "info", group])
            .output()
            .with_context(|| format!("执行 dnf group info {group} 失败"))?;

        if !output.status.success() {
            return Ok(false);
        }

        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.lines().any(|line| {
            let line = line.trim();
            line.starts_with("Installed:") && line.contains("yes")
        }))
    } else {
        let output = Command::new("rpm")
            .args(["-q", pkg])
            .output()
            .with_context(|| format!("执行 rpm -q {pkg} 失败"))?;

        Ok(output.status.success())
    }
}

/// 读取本次启动的唯一标识；重启后会变化。
pub fn boot_id() -> anyhow::Result<String> {
    let id = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .context("读取 /proc/sys/kernel/random/boot_id 失败")?;
    Ok(id.trim().to_string())
}

/// 判断 dnf 仓库是否已启用。
pub fn repo_enabled(repo_id: &str) -> anyhow::Result<bool> {
    let output = Command::new("dnf")
        .args(["repolist", "--enabled"])
        .output()
        .context("执行 dnf repolist 失败")?;

    anyhow::ensure!(
        output.status.success(),
        "dnf repolist 失败（{}）",
        output.status
    );

    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .any(|line| line.split_whitespace().next() == Some(repo_id)))
}

/// `lsblk --json` 输出的最小结构。
#[derive(Debug, Deserialize)]
struct LsblkOutput {
    #[serde(default)]
    blockdevices: Vec<BlockDevice>,
}

#[derive(Debug, Deserialize)]
struct BlockDevice {
    #[serde(default)]
    children: Vec<BlockDevice>,
    size: u64,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    fstype: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    mountpoints: Vec<Option<String>>,
}

/// 自动挑选一块数据分区，写入 `/etc/fstab` 并挂载到 `mount_point`。
///
/// 候选条件：容量不小于 `min_size_gb`、有 UUID、不是 swap、
/// 且没有承载 `/`、`/boot`、`/boot/efi` 中的任何一个。
/// 必须恰好一个候选，否则报错。
pub fn mount_disk(
    ctx: &StepContext,
    mount_point: &Path,
    min_size_gb: u64,
    options: &[String],
) -> anyhow::Result<()> {
    if is_mounted(mount_point) {
        // 这里有一点小漏洞：
        // 有可能 `mount_point` 已经挂载，但 `/etc/fstab` 尚未修改的可能。
        // 但这种可能只在手工 mount 过才会发生，可能性很小。
        // 就偷个懒吧。
        info!(target: &ctx.log_target, "已挂载 {}", mount_point.display());
        return Ok(());
    }

    let output = Command::new("lsblk")
        .args([
            "--json",
            "-b",
            "-o",
            "NAME,SIZE,TYPE,FSTYPE,UUID,MOUNTPOINTS",
        ])
        .output()
        .context("执行 lsblk 失败")?;
    anyhow::ensure!(output.status.success(), "lsblk 失败（{}）", output.status);

    let parsed: LsblkOutput =
        serde_json::from_slice(&output.stdout).context("解析 lsblk 输出失败")?;

    let min_bytes = min_size_gb * 1024 * 1024 * 1024;
    let system_mounts = ["/", "/boot", "/boot/efi"];

    let mut candidates: Vec<(String, String)> = Vec::new(); // (UUID, fstype)
    for disk in &parsed.blockdevices {
        for part in &disk.children {
            if part.kind != "part" {
                continue;
            }

            let fstype = match &part.fstype {
                None => {
                    continue;
                }
                Some(fstype) if fstype == "swap" || fstype.is_empty() => {
                    continue;
                }
                Some(fstype) => fstype,
            };
            let uuid = if let Some(uuid) = &part.uuid {
                uuid
            } else {
                continue;
            };
            if part.size < min_bytes {
                continue;
            }

            let carries_system = part
                .mountpoints
                .iter()
                .flatten()
                .any(|m| system_mounts.contains(&m.as_str()));
            if carries_system {
                continue;
            }

            candidates.push((uuid.clone(), fstype.clone()));
        }
    }

    anyhow::ensure!(
        candidates.len() == 1,
        "期望恰好一个数据盘候选（≥{min_size_gb}G 且非系统分区），实际 {} 个",
        candidates.len()
    );

    let (uuid, fstype) = candidates.pop().unwrap();

    let opts = if options.is_empty() {
        default_mount_options(&fstype)
    } else {
        options.join(",")
    };

    let point = format!("{}", mount_point.display());
    let expected = [format!("UUID={uuid}"), point.clone(), fstype, opts];

    // 1. fstab：已有条目则核对，没有则追加。
    let fstab = fs::read_to_string("/etc/fstab").context("读取 /etc/fstab 失败")?;
    let existing = fstab.lines().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }

        let fields: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        if fields.len() >= 2 && fields[1] == point {
            Some(fields)
        } else {
            None
        }
    });

    match existing {
        Some(fields) => {
            anyhow::ensure!(
                fields.len() >= 4 && fields[..4] == expected[..],
                "fstab 中 {} 已有不同条目：{}",
                point,
                fields.join(" ")
            );
            info!(target: &ctx.log_target, "fstab 条目已正确：{point}");
        }
        None => {
            let staged = ctx.state_dir.join("fstab");
            let mut content = fstab;
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&expected.join(" "));
            content.push_str(" 0 2\n");
            fs::write(&staged, content)
                .with_context(|| format!("写入 {} 失败", staged.display()))?;

            RunAsRoot
                .restore_file(&staged, Path::new("/etc/fstab"))
                .context("写入 /etc/fstab 失败")?;
            info!(target: &ctx.log_target, "已写入 fstab 条目：{point}");
        }
    }

    // 2. 挂载点目录。
    if !mount_point.exists() {
        RunAsRoot.make_dirs(mount_point)?;
    }

    // 3. 挂载。
    let status = RunAsRoot.command("mount").arg(mount_point).status()?;
    anyhow::ensure!(status.success(), "挂载失败：{point}");
    info!(target: &ctx.log_target, "已挂载 {point}");

    Ok(())
}

fn default_mount_options(fstype: &str) -> String {
    if fstype == "btrfs" {
        "defaults,noatime,compress=zstd".to_string()
    } else {
        "defaults,noatime".to_string()
    }
}

fn is_mounted(point: &Path) -> bool {
    Command::new("findmnt")
        .arg("-n")
        .arg(point)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
