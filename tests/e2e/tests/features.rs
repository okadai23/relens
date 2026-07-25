use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use assert_cmd::Command as CliCommand;
use cucumber::World;
use tempfile::TempDir;

/// Scenario-isolated state for executable feature steps.
#[derive(Debug, World)]
pub struct RelensWorld {
    root: Option<TempDir>,
    template_repository: Option<PathBuf>,
    project_directory: Option<PathBuf>,
    last_cli: Option<CliOutput>,
    session_id: Option<String>,
}

impl Default for RelensWorld {
    fn default() -> Self {
        Self {
            root: Some(tempfile::tempdir().expect("temporary world")),
            template_repository: None,
            project_directory: None,
            last_cli: None,
            session_id: None,
        }
    }
}

#[derive(Debug)]
struct CliOutput {
    status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl RelensWorld {
    fn git_fixture(&mut self, name: &str) -> &Path {
        let path = self.root.as_ref().expect("world root").path().join(name);
        fs::create_dir_all(&path).expect("create git fixture");
        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&path)
            .output()
            .expect("git must be available for acceptance tests");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.template_repository = Some(path);
        self.template_repository.as_deref().unwrap()
    }

    fn run_cli(&mut self, args: &[&str]) {
        let output = CliCommand::cargo_bin("relens")
            .expect("relens binary")
            .args(args)
            .output()
            .expect("run relens CLI");
        self.last_cli = Some(CliOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
        });
    }

    fn assert_file(&self, relative: &str, expected: &[u8]) {
        let actual = fs::read(
            self.project_directory
                .as_ref()
                .expect("project directory")
                .join(relative),
        )
        .expect("read project file");
        assert_eq!(actual, expected);
    }
}

#[test]
fn discovers_every_japanese_feature_with_the_cucumber_parser() {
    let feature_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../features");
    let mut discovered = BTreeSet::new();
    for entry in fs::read_dir(&feature_dir).expect("feature directory") {
        let path = entry.expect("feature entry").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "feature")
        {
            let source = fs::read_to_string(&path).expect("read feature");
            let feature = gherkin::Feature::parse(
                &source,
                gherkin::GherkinEnv::new("ja").expect("Japanese Gherkin keywords"),
            )
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_eq!(feature.keyword, "機能");
            discovered.insert(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert_eq!(
        discovered,
        BTreeSet::from([
            "lift.feature".into(),
            "matrix.feature".into(),
            "render.feature".into(),
            "roundtrip.feature".into(),
            "update.feature".into(),
        ])
    );
}

#[test]
fn fixture_builder_and_cli_runner_are_ready_for_scenario_steps() {
    let mut world = RelensWorld::default();
    assert!(world.git_fixture("template").join(".git").is_dir());
    world.run_cli(&["--version"]);
    let output = world.last_cli.as_ref().unwrap();
    assert_eq!(output.status, 0);
    assert!(output.stdout.starts_with(b"relens "));
    assert!(output.stderr.is_empty());

    let project = world.root.as_ref().unwrap().path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("result.txt"), b"expected").unwrap();
    world.project_directory = Some(project);
    world.session_id = Some("session-1".into());
    world.assert_file("result.txt", b"expected");
    assert_eq!(world.session_id.as_deref(), Some("session-1"));
}

/// Executable coverage for all four M1 acceptance scenarios. The assertions use
/// the same process boundary and isolated world as cucumber steps.
#[test]
fn m1_render_and_get_put_scenarios() {
    let root = tempfile::tempdir().unwrap();
    let template = root.path().join("python-lib");
    fs::create_dir_all(template.join("{{ project_name }}")).unwrap();
    fs::write(
        template.join("relens.toml"),
        r#"
[questions.project_name]
type = "string"
default = "sample"
[questions.use_docker]
type = "bool"
default = false
"#,
    )
    .unwrap();
    fs::write(
        template.join("README.md.j2"),
        "# {{ project_name }}\n定型の説明文",
    )
    .unwrap();
    fs::write(
        template.join("{{ project_name }}/main.py.j2"),
        "print(\"{{ project_name }}\")",
    )
    .unwrap();
    fs::write(
        template.join("Dockerfile.j2"),
        "{% if use_docker %}FROM python{% endif %}",
    )
    .unwrap();
    Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&template)
        .status()
        .unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&template)
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-c",
            "user.name=Relens",
            "-c",
            "user.email=relens@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ])
        .current_dir(&template)
        .status()
        .unwrap();

    let first = root.path().join("first");
    let second = root.path().join("second");
    for destination in [&first, &second] {
        CliCommand::cargo_bin("relens")
            .unwrap()
            .args([
                "new",
                template.to_str().unwrap(),
                "--destination",
                destination.to_str().unwrap(),
                "--answer",
                "project_name=myapp",
                "--answer",
                "use_docker=false",
            ])
            .assert()
            .success();
    }
    assert!(
        fs::read_to_string(first.join("README.md"))
            .unwrap()
            .starts_with("# myapp")
    );
    assert!(first.join("myapp/main.py").is_file());
    assert!(!first.join("Dockerfile").exists());
    let answers = fs::read_to_string(first.join(".relens/answers.toml")).unwrap();
    let revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&template)
        .output()
        .unwrap();
    assert!(answers.contains(String::from_utf8(revision.stdout).unwrap().trim()));
    assert_eq!(snapshot(&first), snapshot(&second));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(first.join(".relens/lock.json")).unwrap()).unwrap();
    for file in lock["files"].as_object().unwrap().values() {
        let spans = file["source_map"]["spans"].as_array().unwrap();
        assert!(!spans.is_empty());
        assert_eq!(spans.first().unwrap()["start"], 0);
        for pair in spans.windows(2) {
            assert_eq!(pair[0]["end"], pair[1]["start"]);
        }
    }
    CliCommand::cargo_bin("relens")
        .unwrap()
        .args(["lift", first.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("no-patch"));
}

fn snapshot(root: &Path) -> BTreeSet<(String, Vec<u8>)> {
    walk(root)
        .into_iter()
        .map(|path| {
            (
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                fs::read(path).unwrap(),
            )
        })
        .collect()
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}
