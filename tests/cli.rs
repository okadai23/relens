use assert_cmd::Command;
use predicates::prelude::*;

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
fn rejects_malformed_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "invalid = [").unwrap();

    Command::cargo_bin("relens")
        .unwrap()
        .args(["run", path.to_str().unwrap()])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("invalid configuration"));
}
