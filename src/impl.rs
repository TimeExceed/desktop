//! 与策略无关的文件操作，以及单个步骤的执行上下文。

use anyhow::Context as _;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;

/// 计算文件的 SHA-256（十六进制小写）。
pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("打开 {} 失败", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];

    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

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
