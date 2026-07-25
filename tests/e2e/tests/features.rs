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
#[derive(Debug, Default, World)]
pub struct RelensWorld {
    root: Option<TempDir>,
    template_repository: Option<PathBuf>,
    project_directory: Option<PathBuf>,
    last_cli: Option<CliOutput>,
    session_id: Option<String>,
}

#[derive(Debug)]
struct CliOutput {
    status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl RelensWorld {
    fn fresh() -> Self {
        Self {
            root: Some(tempfile::tempdir().expect("temporary world")),
            ..Self::default()
        }
    }

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
    let mut world = RelensWorld::fresh();
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
