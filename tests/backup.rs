//! 备份器共享逻辑的集成测试。
//!
//! 临时目录由 `tempfile` 创建，测试结束时自动清理；
//! 静态夹具位于 `tests/fixtures/`。

use fedora_setup::backup::*;
use fedora_setup::restore::*;
use fedora_setup::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// 为单个测试创建独立的临时目录（drop 时自动删除）。
fn scratch(name: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("fedora-setup-{name}-"))
        .tempdir()
        .expect("创建临时目录")
}

/// 把夹具模板渲染成临时策略文件，返回 (临时目录, policy 路径, fixtures 目录)。
fn render_fixture_policy(name: &str) -> (TempDir, PathBuf, PathBuf) {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let template = fs::read_to_string(fixtures.join("policy.toml")).expect("读取夹具策略");
    let rendered = template.replace("{{FIXTURES}}", &fixtures.display().to_string());

    let dir = scratch(name);
    let policy_path = dir.path().join("policy.toml");
    fs::write(&policy_path, rendered).expect("写入渲染后的策略");
    (dir, policy_path, fixtures)
}

#[test]
fn loads_policy_as_typed_structure() {
    let dir = scratch("policy");
    let policy_path = dir.path().join("policy.toml");
    fs::write(
        &policy_path,
        r#"
[[tasks]]
id = "demo"
display-name = "演示"

[[tasks.steps]]
id = "copy-files"
dest-dir = "/tmp/x"
files = ["demo/a.txt"]
"#,
    )
    .expect("写入策略");

    let policy = Policy::load(&policy_path).expect("解析策略");

    assert_eq!(policy.tasks.len(), 1);
    let task = &policy.tasks[0];
    assert_eq!(task.id, "demo");
    assert_eq!(task.display_name, "演示");
    assert_eq!(task.steps.len(), 1);
    assert_eq!(task.steps[0].id(), "copy-files");
}

#[test]
fn rejects_policy_violating_schema() {
    let dir = scratch("invalid");
    let policy_path = dir.path().join("policy.toml");
    // 缺少 display-name，schema 校验应当失败。
    fs::write(
        &policy_path,
        r#"
[[tasks]]
id = "bad"

[[tasks.steps]]
id = "copy-files"
dest-dir = "/tmp/x"
files = ["a.txt"]
"#,
    )
    .expect("写入策略");

    let err = Policy::load(&policy_path).expect_err("应当被 schema 拒绝");
    let text = format!("{err:#}");
    assert!(text.contains("display-name"), "错误应指出缺失字段：{text}");
}

#[test]
fn backup_file_keeps_content() {
    let dir = scratch("copy");
    let src = dir.path().join("src.txt");
    let dst = dir.path().join("nested/dst.txt");
    fs::write(&src, "hello\n").expect("写入源文件");

    backup_file(&src, &dst).expect("备份文件");

    assert_eq!(fs::read_to_string(&dst).expect("读取目标文件"), "hello\n");
}

#[test]
fn backup_file_writes_checksum() {
    let dir = scratch("checksum");
    let src = dir.path().join("src.txt");
    let dst = dir.path().join("out.txt");
    fs::write(&src, "hello\n").expect("写入源文件");

    backup_file(&src, &dst).expect("备份文件");

    let sum_path = dir.path().join("out.txt.sha256sum");
    let content = fs::read_to_string(&sum_path).expect("checksum 文件应存在");

    assert!(
        content.starts_with("5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"),
        "checksum 内容不符：{content}"
    );
    assert!(
        content.contains("out.txt"),
        "checksum 应包含文件名：{content}"
    );
}

#[test]
fn missing_source_is_error() {
    let dir = scratch("missing");
    let src = dir.path().join("nope.txt");
    let dst = dir.path().join("out.txt");

    assert!(backup_file(&src, &dst).is_err(), "缺失的源文件应当报错");
}

#[test]
fn directory_is_not_a_copyable_file() {
    let dir = scratch("dir");
    let src = dir.path().join("a-directory");
    fs::create_dir_all(&src).expect("创建目录");
    let dst = dir.path().join("out");

    assert!(
        backup_file(&src, &dst).is_err(),
        "目录不应被当作普通文件复制"
    );
}

/// 静态夹具必须覆盖备份器支持的全部四类数据源，
/// 且每个被引用的文件都真实存在。
#[test]
fn static_fixture_covers_every_backup_source() {
    let (_dir, policy_path, fixtures) = render_fixture_policy("fixture");

    let policy = Policy::load(&policy_path).expect("解析夹具策略");
    assert_eq!(policy.tasks.len(), 1, "夹具应只有一个 task");

    let mut checked = 0usize;

    for task in &policy.tasks {
        for step in &task.steps {
            match step {
                Action::CopyFiles {
                    dest_dir, files, ..
                } => {
                    for file in files {
                        let base = Path::new(file).file_name().expect("basename");
                        let src = dest_dir.join(base);
                        assert!(src.is_file(), "缺少 copy-files 源：{}", src.display());
                        checked += 1;
                    }
                }
                Action::DnfInstall { local_rpms, .. } => {
                    for rpm in local_rpms {
                        assert!(rpm.is_file(), "缺少 rpm：{}", rpm.display());
                        checked += 1;
                    }
                }
                Action::PatchFile { dest, backup, .. } => {
                    assert!(dest.is_file(), "缺少 patch 目标：{}", dest.display());
                    assert!(backup.is_file(), "缺少 patch 基准：{}", backup.display());
                    checked += 2;
                }
                other => panic!("夹具出现未预期的动作：{}", other.id()),
            }
        }
    }

    assert_eq!(checked, 4, "夹具应覆盖 4 个源文件");

    // 夹具内容确实可以复制出来
    let out = scratch("fixture-copy");
    let dst = out.path().join("dotfile.conf");
    backup_file(&fixtures.join("src/dotfile.conf"), &dst).expect("复制夹具文件");
    assert!(
        fs::read_to_string(&dst)
            .expect("读取复制后的夹具")
            .contains("theme = \"light\"")
    );
}

/// 组与包名的检测。
#[test]
fn dnf_installed_detects_packages() {
    assert!(dnf_installed("bash").expect("检查 bash"), "bash 应当已安装");
    assert!(
        !dnf_installed("definitely-not-a-real-package-xyz").expect("检查不存在的包"),
        "不存在的包应当返回 false"
    );
}

/// `copy-files` 的恢复：从 U 盘写回目标目录，且重复执行是幂等的。
#[test]
fn restore_copy_files_and_is_idempotent() {
    let dir = scratch("restore");
    let backup_dir = dir.path().join("usb");
    let dest_dir = dir.path().join("dest");

    fs::create_dir_all(backup_dir.join("demo")).expect("创建备份目录");
    fs::write(backup_dir.join("demo/a.txt"), "hello\n").expect("写入备份");

    let policy_path = dir.path().join("policy.toml");
    fs::write(
        &policy_path,
        format!(
            r#"
[[tasks]]
id = "demo"
display-name = "演示"

[[tasks.steps]]
id = "copy-files"
dest-dir = "{}"
files = ["demo/a.txt"]
"#,
            dest_dir.display()
        ),
    )
    .expect("写入策略");

    let policy = Policy::load(&policy_path).expect("解析策略");
    let step = &policy.tasks[0].steps[0];

    let ctx = StepContext::new("test".into(), backup_dir.clone(), dir.path().join("state"))
        .expect("创建步骤上下文");

    step.restore(&ctx).expect("恢复");
    assert_eq!(
        fs::read_to_string(dest_dir.join("a.txt")).expect("读取恢复后的文件"),
        "hello\n"
    );

    // 再跑一次：内容一致，应当跳过而不是报错。
    step.restore(&ctx).expect("幂等恢复");
}

/// 仓库启用状态的检测。
#[test]
fn repo_enabled_detects_fedora_repo() {
    assert!(
        repo_enabled("fedora").expect("检查 fedora 仓库"),
        "fedora 仓库应当已启用"
    );
    assert!(
        !repo_enabled("no-such-repo-xyz").expect("检查不存在的仓库"),
        "不存在的仓库应当返回 false"
    );
}

/// `patch-file` 应当能打上补丁，且重复执行是幂等的。
#[test]
fn patch_file_applies_and_is_idempotent() {
    let dir = scratch("patch");
    let dest = dir.path().join("cfg.txt");
    let backup = dir.path().join("cfg.txt.bak");
    let usb = dir.path().join("usb");

    fs::write(&dest, "theme = \"dark\"\nfont = \"Maple\"\n").expect("写入目标文件");

    fs::create_dir_all(usb.join("demo")).expect("创建备份目录");
    fs::write(
        usb.join("demo/cfg.patch"),
        "--- a/cfg.txt\n+++ b/cfg.txt\n@@ -1,2 +1,2 @@\n-theme = \"dark\"\n+theme = \"light\"\n font = \"Maple\"\n",
    )
    .expect("写入 patch");

    let policy_path = dir.path().join("policy.toml");
    fs::write(
        &policy_path,
        format!(
            r#"
[[tasks]]
id = "demo"
display-name = "演示"

[[tasks.steps]]
id = "patch-file"
patch = "demo/cfg.patch"
dest = "{}"
backup = "{}"
"#,
            dest.display(),
            backup.display()
        ),
    )
    .expect("写入策略");

    let policy = Policy::load(&policy_path).expect("解析策略");
    let step = &policy.tasks[0].steps[0];

    let ctx = StepContext::new("test".into(), usb.clone(), dir.path().join("state"))
        .expect("创建步骤上下文");

    step.restore(&ctx).expect("打 patch");
    assert_eq!(
        fs::read_to_string(&dest).expect("读取目标文件"),
        "theme = \"light\"\nfont = \"Maple\"\n"
    );

    // 幂等：再跑一次不应重复应用。
    step.restore(&ctx).expect("幂等");
    assert_eq!(
        fs::read_to_string(&dest).expect("读取目标文件"),
        "theme = \"light\"\nfont = \"Maple\"\n"
    );
}

/// `soft-link` 创建链接，并且重复执行是幂等的。
#[test]
fn soft_link_creates_and_is_idempotent() {
    let dir = scratch("softlink");
    let target = dir.path().join("target");
    let dest = dir.path().join("bin/link");

    fs::create_dir_all(&target).expect("创建目标目录");

    let policy_path = dir.path().join("policy.toml");
    fs::write(
        &policy_path,
        format!(
            r#"
[[tasks]]
id = "demo"
display-name = "演示"

[[tasks.steps]]
id = "soft-link"
dest = "{}"
src = "{}"
"#,
            dest.display(),
            target.display()
        ),
    )
    .expect("写入策略");

    let policy = Policy::load(&policy_path).expect("解析策略");
    let step = &policy.tasks[0].steps[0];

    let ctx = StepContext::new(
        "test".into(),
        dir.path().to_path_buf(),
        dir.path().join("state"),
    )
    .expect("创建步骤上下文");

    step.restore(&ctx).expect("创建软链接");
    assert!(dest.is_symlink(), "应当创建软链接");
    assert_eq!(fs::read_link(&dest).expect("读取链接"), target);

    // 幂等：再跑一次不报错。
    step.restore(&ctx).expect("幂等");
}

/// 已指向别处的软链接应当报错，而不是被覆盖。
#[test]
fn soft_link_rejects_other_target() {
    use std::os::unix::fs::symlink;

    let dir = scratch("softlink-bad");
    let target = dir.path().join("target");
    let other = dir.path().join("other");
    let dest = dir.path().join("link");

    fs::create_dir_all(&target).expect("创建目标目录");
    fs::create_dir_all(&other).expect("创建另一个目录");
    symlink(&other, &dest).expect("预置错误链接");

    let policy_path = dir.path().join("policy.toml");
    fs::write(
        &policy_path,
        format!(
            r#"
[[tasks]]
id = "demo"
display-name = "演示"

[[tasks.steps]]
id = "soft-link"
dest = "{}"
src = "{}"
"#,
            dest.display(),
            target.display()
        ),
    )
    .expect("写入策略");

    let policy = Policy::load(&policy_path).expect("解析策略");
    let step = &policy.tasks[0].steps[0];

    let ctx = StepContext::new(
        "test".into(),
        dir.path().to_path_buf(),
        dir.path().join("state"),
    )
    .expect("创建步骤上下文");

    assert!(step.restore(&ctx).is_err(), "指向别处应当报错");
}

/// `manual-check` 在已确认时应当直接跳过，不再读取 stdin。
#[test]
fn manual_check_skips_when_confirmed() {
    let dir = scratch("manual");
    let state_dir = dir.path().join("state");
    fs::create_dir_all(&state_dir).expect("创建状态目录");
    fs::write(state_dir.join("confirmed"), "yes\n").expect("写入确认标记");

    let policy_path = dir.path().join("policy.toml");
    fs::write(
        &policy_path,
        r#"
[[tasks]]
id = "demo"
display-name = "演示"

[[tasks.steps]]
id = "manual-check"
prompt = "请确认"
"#,
    )
    .expect("写入策略");

    let policy = Policy::load(&policy_path).expect("解析策略");
    let step = &policy.tasks[0].steps[0];

    let ctx = StepContext::new("test".into(), dir.path().to_path_buf(), state_dir)
        .expect("创建步骤上下文");

    step.restore(&ctx).expect("已确认应跳过");
}

/// 主机名应当可读。
#[test]
fn current_hostname_is_readable() {
    let name = fedora_setup::current_hostname().expect("读取主机名");
    assert!(!name.is_empty(), "主机名不应为空");
}

/// 步骤输出应当同时回显到终端并写入 `out` 文件。
#[test]
fn step_context_tees_output_to_file() {
    let dir = scratch("ctx");
    let state_dir = dir.path().join("state");
    let ctx = StepContext::new("test".into(), dir.path().to_path_buf(), state_dir.clone())
        .expect("创建步骤上下文");

    let mut cmd = std::process::Command::new("echo");
    cmd.arg("hello-tee");
    let status = ctx.run(&mut cmd).expect("运行命令");
    assert!(status.success());

    let out = fs::read_to_string(state_dir.join("out")).expect("读取 out");
    assert!(out.contains("hello-tee"), "out 应包含命令输出：{out}");
}
