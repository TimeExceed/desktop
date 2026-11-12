//! 与策略无关的通用工具

use anyhow::Context as _;
use std::process::Command;

/// 读取当前静态主机名。
pub fn current_hostname() -> anyhow::Result<String> {
    let output = Command::new("hostnamectl")
        .arg("--static")
        .output()
        .context("执行 hostnamectl --static 失败")?;

    anyhow::ensure!(
        output.status.success(),
        "hostnamectl --static 失败（{}）",
        output.status
    );

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
