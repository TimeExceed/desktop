//! 备份器共享逻辑的集成测试。
//!
//! 临时目录由 `tempfile` 创建，测试结束时自动清理；
//! 静态夹具位于 `tests/fixtures/`。

use fedora_setup::{Action, Policy, copy_file, repo_root};
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
    assert_eq!(repo_root(&policy_path), dir.path());
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
fn copy_file_keeps_content() {
    let dir = scratch("copy");
    let src = dir.path().join("src.txt");
    let dst = dir.path().join("nested/dst.txt");
    fs::write(&src, "hello\n").expect("写入源文件");

    copy_file(&src, &dst).expect("复制文件");

    assert_eq!(fs::read_to_string(&dst).expect("读取目标文件"), "hello\n");
}

#[test]
fn missing_source_is_error() {
    let dir = scratch("missing");
    let src = dir.path().join("nope.txt");
    let dst = dir.path().join("out.txt");

    assert!(copy_file(&src, &dst).is_err(), "缺失的源文件应当报错");
}

#[test]
fn directory_is_not_a_copyable_file() {
    let dir = scratch("dir");
    let src = dir.path().join("a-directory");
    fs::create_dir_all(&src).expect("创建目录");
    let dst = dir.path().join("out");

    assert!(copy_file(&src, &dst).is_err(), "目录不应被当作普通文件复制");
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
                Action::Python { exec, verify, .. } => {
                    for rel in [exec, verify] {
                        let src = fixtures.join(rel);
                        assert!(src.is_file(), "缺少脚本：{}", src.display());
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

    assert_eq!(checked, 6, "夹具应覆盖 6 个源文件");

    // 夹具内容确实可以复制出来
    let out = scratch("fixture-copy");
    let dst = out.path().join("dotfile.conf");
    copy_file(&fixtures.join("src/dotfile.conf"), &dst).expect("复制夹具文件");
    assert!(
        fs::read_to_string(&dst)
            .expect("读取复制后的夹具")
            .contains("theme = \"light\"")
    );
}
