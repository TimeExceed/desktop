//! `restore`：按策略文件从 U 盘恢复系统。
//!
//! 约定：本程序与 `policy.toml` 位于同一个目录（也就是备份根目录），
//! 因此不需要任何参数；保留 clap 只是为了 `--help` 与 `--version`。
//!
//! 执行每个步骤前，框架会在 `~/.local/state/setup/{task}/{step-index}-{step-id}`
//! 下准备状态目录，并把该步骤子进程的 stdout/stderr 写进其中的 `out` 与 `err`
//! （同时回显到终端，以免藏住 `sudo` 的密码提示）。

use anyhow::Context as _;
use clap::{Command, crate_version};
use fedora_setup::Policy;
use fedora_setup::restore::StepContext;
use log::error;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Setup {
        policy,
        state_root,
        backup_dir,
    } = match init() {
        Ok(x) => x,
        Err(e) => {
            error!(target: "main", "{e:?}");
            return ExitCode::from(1);
        }
    };

    for task in &policy.tasks {
        for (idx, step) in task.steps.iter().enumerate() {
            let log_target = format!("{}/{}-{}", task.id, idx, step.id());
            let state_dir = state_root
                .join(&task.id)
                .join(format!("{idx}-{}", step.id()));

            let ctx = match StepContext::new(log_target, backup_dir.clone(), state_dir) {
                Ok(ctx) => ctx,
                Err(err) => {
                    error!(target: "main", "{err:?}");
                    return ExitCode::from(1);
                }
            };

            if let Err(err) = step.restore(&ctx) {
                error!(target: &ctx.log_target, "{err:?}");
                return ExitCode::from(2);
            }
        }
    }

    ExitCode::SUCCESS
}

fn init() -> anyhow::Result<Setup> {
    Command::new("restore")
        .version(crate_version!())
        .about("按策略文件从 U 盘恢复系统")
        .get_matches();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .format_target(true)
        .init();

    // 恢复器与 policy.toml 以及备份数据同在 U 盘根目录。
    let exe = std::env::current_exe().context("定位当前可执行文件失败")?;
    let backup_dir = exe
        .parent()
        .context("无法确定可执行文件所在目录")?
        .to_path_buf();
    let policy_file = backup_dir.join("policy.toml");

    let home = std::env::var_os("HOME").context("HOME 未设置")?;
    let state_root = PathBuf::from(home).join(".local/state/setup");
    let policy = Policy::load(&policy_file)?;

    Ok(Setup {
        policy,
        state_root,
        backup_dir,
    })
}

struct Setup {
    policy: Policy,
    state_root: PathBuf,
    backup_dir: PathBuf,
}
