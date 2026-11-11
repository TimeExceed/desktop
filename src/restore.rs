//! 与策略无关的、仅用于恢复器的工具

use crate::*;
use anyhow::Context as _;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;

/// 单个步骤的执行上下文。
///
/// 框架在执行每个步骤前准备好状态目录。
/// 步骤内启动的子进程输出会**同时**转发到终端与状态目录下的 `out` / `err`：
/// 必须保留终端连接，否则 `sudo` 的密码提示（走 stderr）就看不见了。
pub struct StepContext {
    /// 日志前缀：`{task}/{step-index}-{step-id}`。
    pub log_target: String,
    /// U 盘上的备份根目录。
    pub backup_dir: PathBuf,
    /// 本步骤的状态目录。
    pub state_dir: PathBuf,
    out: fs::File,
    err: fs::File,
}

impl StepContext {
    /// 创建状态目录以及 `out` / `err` 两个输出文件。
    pub fn new(
        log_target: String,
        backup_dir: PathBuf,
        state_dir: PathBuf,
    ) -> anyhow::Result<Self> {
        fs::create_dir_all(&state_dir)
            .with_context(|| format!("创建状态目录 {} 失败", state_dir.display()))?;

        let out = fs::File::create(state_dir.join("out"))
            .with_context(|| format!("创建 {} 失败", state_dir.join("out").display()))?;
        let err = fs::File::create(state_dir.join("err"))
            .with_context(|| format!("创建 {} 失败", state_dir.join("err").display()))?;

        Ok(Self {
            log_target,
            backup_dir,
            state_dir,
            out,
            err,
        })
    }

    /// 执行命令：输出实时回显到终端，同时写入本步骤的 `out` / `err`。
    pub fn run(&self, cmd: &mut Command) -> anyhow::Result<ExitStatus> {
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("启动子进程失败")?;

        let stdout = child.stdout.take().context("获取子进程 stdout 失败")?;
        let stderr = child.stderr.take().context("获取子进程 stderr 失败")?;

        let out = self.out.try_clone().context("复制 stdout 句柄失败")?;
        let err = self.err.try_clone().context("复制 stderr 句柄失败")?;

        let t_out = thread::spawn(move || pump(stdout, io::stdout(), out));
        let t_err = thread::spawn(move || pump(stderr, io::stderr(), err));

        let status = child.wait().context("等待子进程失败")?;

        t_out
            .join()
            .map_err(|_| anyhow::anyhow!("stdout 转发线程异常退出"))??;
        t_err
            .join()
            .map_err(|_| anyhow::anyhow!("stderr 转发线程异常退出"))??;

        Ok(status)
    }
}

/// 把 `reader` 的内容同时写到终端与文件。
fn pump<R: Read, W: Write>(mut reader: R, mut term: W, mut file: fs::File) -> io::Result<()> {
    let mut buf = [0u8; 8 * 1024];

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }

        term.write_all(&buf[..n])?;
        term.flush()?;
        file.write_all(&buf[..n])?;
        file.flush()?;
    }

    Ok(())
}

/// 比较两个文件内容是否一致（用 SHA-256）；目标不存在时返回 `false`。
pub fn same_content(src: &Path, dst: &Path) -> anyhow::Result<bool> {
    if !dst
        .try_exists()
        .with_context(|| format!("无法访问 {}", dst.display()))?
    {
        return Ok(false);
    }

    Ok(sha256_file(src)? == sha256_file(dst)?)
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

/// 判断 patch 是否已经应用到目标文件（反向 dry-run 成功即为已应用）。
pub fn patch_applied(patch: &Path, dest: &Path) -> anyhow::Result<bool> {
    let output = Command::new("patch")
        .arg("-R")
        .arg("--dry-run")
        .arg("-i")
        .arg(patch)
        .arg(dest)
        .output()
        .with_context(|| format!("执行 patch 检查失败：{}", dest.display()))?;

    Ok(output.status.success())
}

/// 用 `install`（必要时加 `sudo`）把备份文件恢复到目标位置。
///
/// `install -D` 会创建父目录，`-p` 保留源文件的时间戳与权限。
pub fn restore_file(
    src: &Path,
    dst: &Path,
    run_as_root: bool,
    ctx: &StepContext,
) -> anyhow::Result<()> {
    let mut cmd = if run_as_root {
        let mut c = Command::new("sudo");
        c.arg("install");
        c
    } else {
        Command::new("install")
    };
    cmd.arg("-D").arg("-p").arg(src).arg(dst);

    let status = ctx
        .run(&mut cmd)
        .with_context(|| format!("执行 install 失败：{} → {}", src.display(), dst.display()))?;

    anyhow::ensure!(
        status.success(),
        "install 失败（{status}）：{} → {}",
        src.display(),
        dst.display()
    );

    Ok(())
}
