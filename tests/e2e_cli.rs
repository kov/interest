use assert_cmd::{cargo, prelude::*};
use predicates::prelude::*;
use std::{path::PathBuf, process::Command};
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

#[test]
fn import_dry_run_does_not_create_db() {
    let home = setup_temp_home();
    let db_path = PathBuf::from(home.path()).join(".interest").join("data.db");
    assert!(!db_path.exists(), "db should start absent");

    let mut cmd = Command::new(cargo::cargo_bin!("interest"));
    cmd.env("HOME", home.path())
        .arg("--no-color")
        .arg("import")
        .arg("tests/data/01_basic_purchase_sale.xlsx")
        .arg("--dry-run");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Found"))
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("\u{001b}[").not());

    assert!(!db_path.exists(), "dry-run should not create db");
}

#[test]
fn import_then_portfolio_shows_position() {
    let home = setup_temp_home();

    let mut import_cmd = Command::new(cargo::cargo_bin!("interest"));
    import_cmd
        .env("HOME", home.path())
        .arg("--no-color")
        .arg("import")
        .arg("tests/data/01_basic_purchase_sale.xlsx");

    import_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("Found"))
        .stdout(predicate::str::contains("\u{001b}[").not());

    let mut portfolio_cmd = Command::new(cargo::cargo_bin!("interest"));
    portfolio_cmd
        .env("HOME", home.path())
        .arg("--no-color")
        .arg("portfolio")
        .arg("show");

    portfolio_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("PETR4"))
        .stdout(predicate::str::contains("70.00"))
        .stdout(predicate::str::contains("\u{001b}[").not());
}
