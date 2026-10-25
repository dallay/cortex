use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn command(home: &Path, project: &Path, db: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_huginn"));
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .args(["--provider", "mock", "--json", "--workspace"])
        .arg(project)
        .arg("--db")
        .arg(db);
    cmd
}
fn user_text(db: &Path) -> String {
    let connection = rusqlite::Connection::open(db).unwrap();
    let text: String = connection
        .query_row("SELECT data FROM sessions LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let messages = value["messages"].as_array().unwrap();
    let user = messages.iter().find(|m| m["role"] == "user").unwrap();
    user["content"].as_str().unwrap().to_string()
}
#[test]
fn run_and_line_mode_store_the_same_rendered_user_message_and_valid_json() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let prompts = project.path().join(".huginn/prompts");
    fs::create_dir_all(&prompts).unwrap();
    fs::write(
        prompts.join("review.md"),
        "Review $1 with focus $2; context: $@",
    )
    .unwrap();
    let input = "/prompts:review src/lib.rs \"thread safety\"";
    let expected = "Review src/lib.rs with focus thread safety; context: src/lib.rs thread safety";
    let run_db = home.path().join("run.db");
    let output = command(home.path(), project.path(), &run_db)
        .args(["run", input])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    assert_eq!(user_text(&run_db), expected);
    let line_db = home.path().join("line.db");
    let mut child = command(home.path(), project.path(), &line_db)
        .arg("--line-mode")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{input}\n/exit").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    assert_eq!(user_text(&line_db), expected);
}
#[test]
fn missing_arguments_fail_before_any_user_turn_is_persisted() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let prompts = project.path().join(".huginn/prompts");
    fs::create_dir_all(&prompts).unwrap();
    fs::write(prompts.join("review.md"), "$2").unwrap();
    let db = home.path().join("error.db");
    let output = command(home.path(), project.path(), &db)
        .args(["run", "/prompts:review one"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("$2"));
    if db.exists() {
        let c = rusqlite::Connection::open(db).unwrap();
        let count: i64 = c
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
