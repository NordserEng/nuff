use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

const SOURCE: &str = "import os\n\n\ndef f():\n    import sys\n\n    return sys\n";

fn check(project: &Path) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_nuff"))
        .args(["check", "--no-cache", "--output-format", "concise"])
        .current_dir(project)
        .env_remove("NUFF_OUTPUT_FORMAT")
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (name, contents) in files {
        fs::write(dir.path().join(name), contents).unwrap();
    }
    dir
}

#[test]
fn reads_tool_nuff_from_pyproject() {
    let dir = project(&[
        (
            "pyproject.toml",
            "[tool.nuff.lint]\nselect = [\"PLC0415\"]\n",
        ),
        ("module.py", SOURCE),
    ]);
    let (code, stdout) = check(dir.path());
    assert_eq!(code, 1);
    assert!(stdout.contains("module.py:5:5: PLC0415"), "{stdout}");
    assert!(!stdout.contains("F401"), "{stdout}");
}

#[test]
fn reads_lint_section_from_nuff_toml() {
    let dir = project(&[
        ("nuff.toml", "[lint]\nselect = [\"PLC0415\"]\n"),
        ("module.py", SOURCE),
    ]);
    let (code, stdout) = check(dir.path());
    assert_eq!(code, 1);
    assert!(stdout.contains("module.py:5:5: PLC0415"), "{stdout}");
    assert!(!stdout.contains("F401"), "{stdout}");
}

#[test]
fn defaults_to_pyflakes() {
    let dir = project(&[("module.py", SOURCE)]);
    let (code, stdout) = check(dir.path());
    assert_eq!(code, 1);
    assert!(stdout.contains("module.py:1:8: F401"), "{stdout}");
    assert!(!stdout.contains("PLC0415"), "{stdout}");
}

#[test]
fn file_level_noqa_silences_the_file() {
    let dir = project(&[("module.py", &format!("# nuff: noqa: F401\n{SOURCE}"))]);
    let (code, stdout) = check(dir.path());
    assert_eq!(code, 0, "{stdout}");
}
