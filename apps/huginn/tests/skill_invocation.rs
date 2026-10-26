use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn command(home: &Path, project: &Path, db: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_huginn"));
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home)
        .args(["--provider", "mock", "--json", "--workspace"])
        .arg(project)
        .arg("--db")
        .arg(db);
    command
}

fn skill(root: &Path, body: &str) {
    let directory = root.join(".agents/skills/review");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("SKILL.md"),
        format!("---\nname: review\ndescription: Review local changes\n---\n{body}"),
    )
    .unwrap();
}

fn session(db: &Path) -> serde_json::Value {
    let connection = rusqlite::Connection::open(db).unwrap();
    let data: String = connection
        .query_row("SELECT data FROM sessions LIMIT 1", [], |row| row.get(0))
        .unwrap();
    serde_json::from_str(&data).unwrap()
}

fn assert_ndjson(output: &[u8]) -> Vec<serde_json::Value> {
    assert!(!output.contains(&0x1b));
    String::from_utf8_lossy(output)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn run_line_json_and_resume_preserve_task_and_project_selection() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    skill(home.path(), "GLOBAL OVERRIDDEN BODY");
    skill(project.path(), "PROJECT BODY");
    let task = " \"quoted\" \\path  ";
    let input = format!("/skill:review {task}");
    for line_mode in [false, true] {
        let db = home.path().join(format!("mode-{line_mode}.db"));
        let output = if line_mode {
            let mut child = command(home.path(), project.path(), &db)
                .arg("--line-mode")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            writeln!(
                child.stdin.take().unwrap(),
                "/skill:unknown\n{input}\n/quit"
            )
            .unwrap();
            child.wait_with_output().unwrap()
        } else {
            command(home.path(), project.path(), &db)
                .args(["run", &input])
                .output()
                .unwrap()
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events = assert_ndjson(&output.stdout);
        assert_eq!(
            events
                .iter()
                .filter(|event| event["type"] == "turn_started")
                .count(),
            1
        );
        let saved = session(&db);
        let user = saved["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "user")
            .unwrap();
        let envelope: serde_json::Value =
            serde_json::from_str(user["content"].as_str().unwrap().lines().last().unwrap())
                .unwrap();
        assert_eq!(envelope["task"], task);
        assert_eq!(envelope["skill"]["scope"], "project");
        assert!(envelope["skill"]["instructions"]
            .as_str()
            .unwrap()
            .contains("PROJECT BODY"));
        assert!(!user["content"]
            .as_str()
            .unwrap()
            .contains("GLOBAL OVERRIDDEN BODY"));
        let id = saved["id"].as_str().unwrap();
        skill(project.path(), "UPDATED PROJECT BODY");
        let output = command(home.path(), project.path(), &db)
            .args(["resume", id, "--prompt", "/skill:review resumed"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_ndjson(&output.stdout);
        let saved = session(&db);
        assert_eq!(
            saved["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["role"] == "user")
                .count(),
            2
        );
        assert!(saved.to_string().contains("UPDATED PROJECT BODY"));
        skill(project.path(), "PROJECT BODY");
    }
}

#[test]
fn invalid_unknown_and_unavailable_skills_never_start_a_json_turn() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    skill(project.path(), &"x".repeat(20000));
    for (index, input) in [
        "/skill:",
        "/skill:../review",
        "/skill:missing",
        "/skill:review",
    ]
    .iter()
    .enumerate()
    {
        let db = home.path().join(format!("failure-{index}.db"));
        let output = command(home.path(), project.path(), &db)
            .args(["run", input])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(assert_ndjson(&output.stdout).is_empty());
        let connection = rusqlite::Connection::open(db).unwrap();
        let count: i64 = connection
            .query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn templates_cannot_implicitly_activate_project_skills() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    skill(project.path(), "must be explicitly selected");
    let directory = project.path().join(".huginn/prompts");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("indirect.md"), "/skill:review task").unwrap();
    let output = command(
        home.path(),
        project.path(),
        &home.path().join("indirect.db"),
    )
    .args(["run", "/prompts:indirect"])
    .output()
    .unwrap();
    assert!(!output.status.success());
    assert!(assert_ndjson(&output.stdout).is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot invoke skills"));
}
