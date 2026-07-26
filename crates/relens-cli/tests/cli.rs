use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command as ProcessCommand};

#[test]
fn shows_version() {
    Command::cargo_bin("relens")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("relens 0.1.0"));
}

#[test]
fn invalid_argument_goes_to_stderr() {
    Command::cargo_bin("relens")
        .unwrap()
        .arg("--unknown-option")
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
fn initializes_and_inspects_json() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    Command::cargo_bin("relens")
        .unwrap()
        .args(["init", path.to_str().unwrap()])
        .assert()
        .success();
    Command::cargo_bin("relens")
        .unwrap()
        .args(["run", path.to_str().unwrap(), "--output", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"action\":\"inspected\""));
}

#[test]
fn update_conflict_json_is_one_value_and_diagnostics_are_stderr_only() {
    let root = tempfile::tempdir().unwrap();
    let template = root.path().join("template");
    fs::create_dir_all(&template).unwrap();
    fs::write(
        template.join("relens.toml"),
        "[questions.name]\ntype='string'\n",
    )
    .unwrap();
    fs::write(template.join("README.md.j2"), "# {{ name }}\nOriginal\n").unwrap();
    git_commit(&template, "v1");

    let project = root.path().join("project");
    generate(&template, &project);
    fs::write(project.join("README.md"), "# demo\nLocal\n").unwrap();
    fs::write(template.join("README.md.j2"), "# {{ name }}\nTemplate\n").unwrap();
    git_commit(&template, "v2");

    let output = Command::cargo_bin("relens")
        .unwrap()
        .args(["update", project.to_str().unwrap(), "--output", "json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, json!({"status":"conflict","files":["README.md"]}));
    assert!(!output.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("{\"status\""));
}

#[test]
fn export_verification_failure_json_is_one_value_and_preserves_failure_status() {
    let root = tempfile::tempdir().unwrap();
    let template = root.path().join("template");
    fs::create_dir_all(&template).unwrap();
    fs::write(
        template.join("relens.toml"),
        "[questions.name]\ntype='string'\n",
    )
    .unwrap();
    fs::write(template.join("README.md.j2"), "# {{ name }}\n").unwrap();
    git_commit(&template, "v1");
    let revision = git_output(&template, &["rev-parse", "HEAD"]);
    let project = root.path().join("project");
    generate(&template, &project);
    let sessions = project.join(".relens/sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join("failed.json"),
        serde_json::to_vec(&json!({
            "id": "failed",
            "project": project,
            "template": {"locator": template, "revision": revision.trim()},
            "state": "Reviewing",
            "edits": [],
            "divergences": [{"path":"README.md","start":0,"end":8}]
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::cargo_bin("relens")
        .unwrap()
        .args([
            "lift",
            project.to_str().unwrap(),
            "--export",
            "--output",
            "json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        json!({"status":"verification_failed","locations":"README.md:0..8"})
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("lift verification failed"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("{\"status\""));
}

fn generate(template: &Path, project: &Path) {
    Command::cargo_bin("relens")
        .unwrap()
        .args([
            "new",
            template.to_str().unwrap(),
            "-d",
            project.to_str().unwrap(),
            "-a",
            "name=demo",
        ])
        .assert()
        .success();
}

fn git_commit(repository: &Path, message: &str) {
    if !repository.join(".git").exists() {
        ProcessCommand::new("git")
            .args(["init", "--quiet"])
            .current_dir(repository)
            .status()
            .unwrap();
    }
    ProcessCommand::new("git")
        .args(["add", "."])
        .current_dir(repository)
        .status()
        .unwrap();
    ProcessCommand::new("git")
        .args([
            "-c",
            "user.name=Relens",
            "-c",
            "user.email=relens@example.invalid",
            "commit",
            "--quiet",
            "-m",
            message,
        ])
        .current_dir(repository)
        .status()
        .unwrap();
}

fn git_output(repository: &Path, args: &[&str]) -> String {
    String::from_utf8(
        ProcessCommand::new("git")
            .args(args)
            .current_dir(repository)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
}
