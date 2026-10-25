use huginn_application::skills::{SkillCatalogQuery, SourceScope};
use huginn_runtime::skills::FilesystemSkillCatalog;
use std::{fs, path::Path};

fn write(root: &Path, directory: &str, content: impl AsRef<[u8]>) {
    let path = root.join(".agents/skills").join(directory);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("SKILL.md"), content).unwrap();
}
fn valid(name: &str) -> String {
    format!("---\nname: {name}\ndescription: A useful workflow\n---\nBody\n")
}
fn snapshot(workspace: &Path, home: &Path) -> huginn_application::skills::SkillCatalog {
    FilesystemSkillCatalog
        .refresh(workspace, Some(home))
        .unwrap()
}

#[cfg(unix)]
#[test]
fn project_skills_root_symlink_outside_repository_is_rejected() {
    use std::os::unix::fs::symlink;

    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::write(repo.path().join(".git"), "gitdir: /elsewhere").unwrap();
    write(external.path(), "outside", valid("outside"));
    let project_skills = repo.path().join(".agents/skills");
    fs::create_dir_all(project_skills.parent().unwrap()).unwrap();
    symlink(external.path().join(".agents/skills"), &project_skills).unwrap();

    let catalog = snapshot(repo.path(), home.path());

    assert!(catalog.skills.is_empty());
    let diagnostic = catalog
        .diagnostics
        .iter()
        .find(|d| d.code == "project_escape")
        .unwrap();
    assert_eq!(diagnostic.severity, "security");
}

#[cfg(unix)]
#[test]
fn non_utf8_skill_parent_path_is_rejected_without_panicking_during_serialization() {
    use std::os::unix::ffi::OsStrExt;

    let temporary = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let non_utf8 = std::ffi::OsStr::from_bytes(b"bad-\xff");
    let repo_path = temporary.path().join(non_utf8).join("repo");
    fs::create_dir_all(&repo_path).unwrap();
    fs::write(repo_path.join(".git"), "gitdir: /elsewhere").unwrap();
    let directory = repo_path.join(".agents/skills/valid");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("SKILL.md"), valid("valid")).unwrap();

    let catalog = snapshot(&repo_path, home.path());

    assert_eq!(catalog.skills.len(), 1);
    let identity = catalog.skills[0].identity();
    let identity_fields: serde_json::Value = serde_json::from_str(&identity).unwrap();
    assert!(identity_fields[1].as_str().unwrap().contains("6261642dff"));
    let json = catalog.to_json();
    assert_eq!(
        json["skills"][0]["logical_path"],
        catalog.skills[0].logical_path.to_string_lossy().as_ref()
    );
    let selected = [identity.clone()];
    let announcement = catalog.announcement(&selected).unwrap();
    let entries: serde_json::Value =
        serde_json::from_str(announcement.lines().last().unwrap()).unwrap();
    assert_eq!(entries[0]["identity"], identity);
}

#[test]
fn scopes_nearest_git_precedence_sorting_and_no_recursion() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fs::write(repo.path().join(".git"), "gitdir: /elsewhere").unwrap();
    let nested = repo.path().join("packages/app");
    fs::create_dir_all(&nested).unwrap();
    write(repo.path(), "same", valid("same"));
    write(home.path(), "same", valid("same"));
    write(home.path(), "alpha", valid("alpha"));
    write(&nested, "nested", valid("nested"));
    write(repo.path(), "collection/hidden", valid("hidden"));
    write(repo.path(), "Code-Review", valid("code-review"));
    let catalog = snapshot(&nested, home.path());
    assert_eq!(
        catalog
            .skills
            .iter()
            .map(|s| (s.name.as_str(), s.source_scope, s.selected))
            .collect::<Vec<_>>(),
        vec![
            ("alpha", SourceScope::Global, true),
            ("same", SourceScope::Project, true),
            ("same", SourceScope::Global, false)
        ]
    );
    assert!(catalog
        .diagnostics
        .iter()
        .any(|d| d.code == "name_mismatch"));
    assert!(catalog
        .skills
        .iter()
        .all(|s| s.canonical_path.is_absolute()));
    assert_eq!(catalog.to_json()["skills"][0]["trust"], "untrusted");
}

#[test]
fn malformed_entries_are_isolated_and_numeric_limits_enforced() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    write(repo.path(), "good", valid("good"));
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("yaml", b"---\nname: [\n---\n".to_vec(), "invalid_yaml"),
        ("utf8", vec![0xff], "invalid_utf8"),
        ("missing", b"name: missing".to_vec(), "frontmatter_missing"),
        (
            "type",
            b"---\nname: 123\ndescription: text\n---".to_vec(),
            "metadata_type",
        ),
        ("upper", valid("UPPER").into_bytes(), "invalid_name"),
        ("dash", valid("a--b").into_bytes(), "invalid_name"),
        ("mismatch", valid("other").into_bytes(), "name_mismatch"),
        (
            "long",
            format!("---\nname: long\ndescription: {}\n---", "é".repeat(1025)).into_bytes(),
            "invalid_description",
        ),
        (
            "blank",
            b"---\nname: blank\ndescription: ' '\n---".to_vec(),
            "invalid_description",
        ),
        (
            "compat",
            b"---\nname: compat\ndescription: text\ncompatibility: ''\n---".to_vec(),
            "invalid_compatibility",
        ),
        (
            "tools",
            b"---\nname: tools\ndescription: text\nallowed-tools: [all]\n---".to_vec(),
            "metadata_type",
        ),
        (
            "meta",
            b"---\nname: meta\ndescription: text\nmetadata: {version: 1}\n---".to_vec(),
            "metadata_type",
        ),
        ("large", vec![b'a'; 65_537], "file_limit"),
        (
            "duplicate",
            b"---\nname: duplicate\nname: other\ndescription: text\n---".to_vec(),
            "invalid_yaml",
        ),
    ];
    for (name, content, _) in &cases {
        write(repo.path(), name, content);
    }
    let catalog = snapshot(repo.path(), home.path());
    assert_eq!(catalog.skills.len(), 1);
    for (name, _, code) in cases {
        assert!(
            catalog
                .diagnostics
                .iter()
                .any(|d| d.code == code && d.path.parent().unwrap().ends_with(name)),
            "missing {name}/{code}: {:?}",
            catalog.diagnostics
        );
    }
}

#[test]
fn overflow_discards_whole_root_but_not_other_scope() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    write(repo.path(), "project", valid("project"));
    for n in 0..257 {
        let name = format!("skill-{n}");
        write(home.path(), &name, valid(&name));
    }
    let catalog = snapshot(repo.path(), home.path());
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.diagnostics.len(), 1);
    assert_eq!(catalog.diagnostics[0].code, "root_limit");
    assert_eq!(catalog.diagnostics[0].scope, SourceScope::Global);
}

#[test]
fn refresh_rehashes_same_mtime_content_and_removes_deleted_entries() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    write(repo.path(), "good", valid("good"));
    let first = snapshot(repo.path(), home.path());
    assert_eq!(
        first.fingerprint,
        snapshot(repo.path(), home.path()).fingerprint
    );
    let path = repo.path().join(".agents/skills/good/SKILL.md");
    let old_time = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, valid("good").replace("Body", "Edit")).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old_time)
        .unwrap();
    assert_ne!(
        first.fingerprint,
        snapshot(repo.path(), home.path()).fingerprint
    );
    fs::remove_file(&path).unwrap();
    let deleted = snapshot(repo.path(), home.path());
    assert!(deleted.skills.is_empty());
    assert_ne!(first.fingerprint, deleted.fingerprint);
}

#[cfg(unix)]
#[test]
fn symlink_policy_aliases_broken_loops_and_nonregular_files() {
    use std::os::unix::fs::symlink;
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    write(external.path(), "original", valid("alias"));
    fs::create_dir_all(home.path().join(".agents/skills")).unwrap();
    fs::create_dir_all(repo.path().join(".agents/skills/file")).unwrap();
    let target = external.path().join(".agents/skills/original");
    symlink(&target, home.path().join(".agents/skills/alias")).unwrap();
    symlink(&target, repo.path().join(".agents/skills/escape")).unwrap();
    symlink(
        target.join("SKILL.md"),
        repo.path().join(".agents/skills/file/SKILL.md"),
    )
    .unwrap();
    symlink("missing", repo.path().join(".agents/skills/broken")).unwrap();
    symlink("loop", repo.path().join(".agents/skills/loop")).unwrap();
    fs::create_dir_all(repo.path().join(".agents/skills/nonregular/SKILL.md")).unwrap();
    let catalog = snapshot(repo.path(), home.path());
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.skills[0].name, "alias");
    assert_eq!(catalog.skills[0].source_scope, SourceScope::Global);
    assert_ne!(
        catalog.skills[0].logical_path,
        catalog.skills[0].canonical_path
    );
    assert_eq!(
        catalog
            .diagnostics
            .iter()
            .filter(|d| d.code == "project_escape")
            .count(),
        2
    );
    assert_eq!(
        catalog
            .diagnostics
            .iter()
            .filter(|d| d.code == "path_resolution")
            .count(),
        2
    );
    assert!(catalog.diagnostics.iter().any(|d| d.code == "not_regular"));
    let first = catalog.fingerprint;
    write(external.path(), "second", valid("alias"));
    fs::remove_file(home.path().join(".agents/skills/alias")).unwrap();
    symlink(
        external.path().join(".agents/skills/second"),
        home.path().join(".agents/skills/alias"),
    )
    .unwrap();
    assert_ne!(first, snapshot(repo.path(), home.path()).fingerprint);
}

#[test]
fn exact_file_and_root_budgets_are_accepted_and_unicode_lengths_count_characters() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut content = format!("---\nname: boundary\ndescription: {}\ncompatibility: {}\nlicense: MIT\nallowed-tools: shell\nmetadata: {{version: '1'}}\n---\n", "é".repeat(1024), "界".repeat(500));
    content.push_str(&"x".repeat(65_536 - content.len()));
    write(repo.path(), "boundary", content);
    for n in 0..256 {
        let name = format!("skill-{n}");
        write(home.path(), &name, valid(&name));
    }
    let catalog = snapshot(repo.path(), home.path());
    assert_eq!(catalog.skills.len(), 257);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
}

#[cfg(unix)]
#[test]
fn escaped_project_root_is_rejected_and_internal_alias_is_allowed() {
    use std::os::unix::fs::symlink;
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    write(external.path(), "external", valid("external"));
    fs::create_dir(repo.path().join(".agents")).unwrap();
    symlink(
        external.path().join(".agents/skills"),
        repo.path().join(".agents/skills"),
    )
    .unwrap();
    let escaped = snapshot(repo.path(), home.path());
    assert!(escaped.skills.is_empty());
    assert_eq!(escaped.diagnostics[0].code, "project_escape");
    fs::remove_file(repo.path().join(".agents/skills")).unwrap();
    fs::create_dir(repo.path().join(".agents/skills")).unwrap();
    fs::create_dir(repo.path().join("resources")).unwrap();
    fs::write(repo.path().join("resources/SKILL.md"), valid("alias")).unwrap();
    symlink(
        repo.path().join("resources"),
        repo.path().join(".agents/skills/alias"),
    )
    .unwrap();
    assert_eq!(snapshot(repo.path(), home.path()).skills[0].name, "alias");
}
