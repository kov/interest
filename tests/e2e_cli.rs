use assert_cmd::{cargo, prelude::*};
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn setup_temp_home() -> TempDir {
    TempDir::new().expect("failed to create temp home")
}

#[test]
fn portfolio_show_empty_db_no_color_when_piped() {
    // Arrange: temp HOME so the app uses an isolated DB
    let home = setup_temp_home();

    // Act: run the CLI with stdout captured (piped)
    let mut cmd = Command::new(cargo::cargo_bin!("interest"));
    cmd.env("HOME", home.path());
    cmd.arg("portfolio").arg("show").arg("--no-color");

    // Assert: success and friendly empty message without ANSI escapes
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("No positions found"))
        .stdout(predicate::str::contains("\u{001b}[").not());
}
