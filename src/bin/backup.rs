//! `backup`：按策略文件把本机数据备份到 U 盘。

use anyhow::Context as _;
use clap::{Arg, Command, crate_version, value_parser};
use fedora_setup::backup::*;
use fedora_setup::*;
use log::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .format_target(true)
        .init();

    if let Err(e) = run() {
        eprintln!("错误：{e:?}");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.backup()
}

struct Cli {
    policy_file: PathBuf,
    backup_dir: PathBuf,
}

impl Cli {
    fn parse() -> Self {
        let matches = Command::new("backup")
            .version(crate_version!())
            .about("把本机数据备份到 U 盘")
            .arg(
                Arg::new("policy")
                    .value_name("FILE")
                    .help("策略文件路径")
                    .value_parser(value_parser!(PathBuf))
                    .required(true),
            )
            .arg(
                Arg::new("backup-dir")
                    .long("backup-dir")
                    .value_name("DIR")
                    .help("备份目标目录，通常是 U 盘的挂载点")
                    .value_parser(value_parser!(PathBuf))
                    .required(true),
            )
            .get_matches();

        let policy_file = matches.get_one::<PathBuf>("policy").unwrap().clone();
        let backup_dir = matches.get_one::<PathBuf>("backup-dir").unwrap().clone();
        Self {
            policy_file,
            backup_dir,
        }
    }

    fn backup(&self) -> anyhow::Result<()> {
        let policy = Policy::load(&self.policy_file)?;

        fs::create_dir_all(&self.backup_dir)
            .with_context(|| format!("创建备份目录 {} 失败", self.backup_dir.display()))?;

        for task in &policy.tasks {
            for (idx, step) in task.steps.iter().enumerate() {
                let target = format!("{}/{}-{}", task.id, idx, step.id());
                step.backup(&target, &self.backup_dir)?;
            }
        }

        backup_file(&self.policy_file, &self.backup_dir.join("policy.toml"))?;
        info!(target: "main", "复制 `policy.toml`");

        copy_restore_binary(&self.backup_dir)?;

        Ok(())
    }
}

/// 把与 backup 同目录的 restore 一并放进 U 盘，使 U 盘可以独立恢复。
fn copy_restore_binary(backup_dir: &Path) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("定位当前可执行文件失败")?;
    let dir = exe.parent().context("无法确定可执行文件所在目录")?;
    let restore = dir.join("restore");

    if !restore.is_file() {
        anyhow::bail!("未找到 {}", restore.display());
    }

    let dst = backup_dir.join("restore");
    backup_file(&restore, &dst).context("复制恢复器失败")?;
    info!(target: "main", "复制 `{}`", restore.display());
    Ok(())
}
