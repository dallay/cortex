//! TEMPORARY DIAGNOSTIC. Delete this file once the report is read.
//!
//! Reports how the current volume handles `renameatx_np(RENAME_EXCL)` for
//! directories, which is what `ExclusiveStage::publish_directory` uses, and it
//! mimics the exact state the production publisher hands to the syscall.
//!
//! It only runs in `Test (Muninn C1 opt-in)` (macos-15), the only place these
//! primitives execute. Local macOS 26/27 does not reproduce the failure, so the
//! report has to come from the runner.
//!
//! The test prints the report and stays green; the probe workflow reads it with
//! `--nocapture`.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use muninn_c1_exporter::artifact_fs::staging::ExclusiveStage;
use muninn_c1_exporter::artifact_fs::ArtifactDir;
use rustix::fs::RenameFlags;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

struct Case<'a> {
    label: &'a str,
    source_mode: Option<u32>,
    nested: bool,
    hold_child: bool,
    hold_source: bool,
    flags: RenameFlags,
    freeze_after: bool,
}

fn probe(root: &Path, case: &Case<'_>) -> String {
    let source = root.join(format!(".probe-{}", case.label));
    let target = format!("{:0>64}", case.label.replace('-', ""));
    fs::create_dir(&source).expect("probe directory");
    if case.nested {
        let child = source.join("1_Pooling");
        fs::create_dir(&child).expect("probe child");
        let config = child.join("config.json");
        fs::write(&config, b"abc").expect("probe bytes");
        fs::set_permissions(&config, fs::Permissions::from_mode(0o400)).expect("probe file mode");
        fs::set_permissions(&child, fs::Permissions::from_mode(0o500)).expect("probe child mode");
    }
    fs::write(source.join("marker"), b"abc").expect("probe marker");
    if let Some(mode) = case.source_mode {
        fs::set_permissions(&source, fs::Permissions::from_mode(mode)).expect("probe source mode");
    }
    let child_fd = case
        .hold_child
        .then(|| fs::File::open(source.join("1_Pooling")).expect("held child"));
    let source_fd = case
        .hold_source
        .then(|| fs::File::open(&source).expect("held source"));
    let directory = fs::File::open(root).expect("probe root");
    let result = rustix::fs::renameat_with(
        &directory,
        source.as_path(),
        &directory,
        target.as_str(),
        case.flags,
    );
    drop((child_fd, source_fd));
    let outcome = match result {
        Ok(()) => "ok".to_owned(),
        Err(error) => format!("errno={} ({error})", error.raw_os_error()),
    };
    let frozen_after = if case.freeze_after && outcome == "ok" {
        match fs::set_permissions(root.join(&target), fs::Permissions::from_mode(0o500)) {
            Ok(()) => " then freeze: ok".to_owned(),
            Err(error) => format!(" then freeze: {error}"),
        }
    } else {
        String::new()
    };
    for path in [&source, &root.join(&target)] {
        if path.is_dir() {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        }
    }
    format!("{:<26} -> {outcome}{frozen_after}", case.label)
}

fn staging_probe(root: &Path, label: &str, freeze: bool, target_byte: char) -> String {
    let parent = match ArtifactDir::open_root(root) {
        Ok(parent) => parent,
        Err(error) => return format!("{label:<26} -> open_root: {error:?}"),
    };
    let stage = match ExclusiveStage::begin(&parent) {
        Ok(stage) => stage,
        Err(error) => return format!("{label:<26} -> begin: {error:?}"),
    };
    if freeze {
        let staged = fs::read_dir(root)
            .expect("probe scan")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".stage-"))
            });
        match staged {
            Some(path) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o500))
                    .expect("freeze stage");
            }
            None => return format!("{label:<26} -> stage dir not found"),
        }
    }
    let mode = fs::read_dir(root)
        .expect("probe scan")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".stage-"))
        })
        .and_then(|path| fs::metadata(path).ok())
        .map(|metadata| format!("{:o}", metadata.mode() & 0o7777))
        .unwrap_or_else(|| "gone".to_owned());
    let target = target_byte.to_string().repeat(64);
    let outcome = match stage.publish_directory(&target) {
        Ok(_) => "ok".to_owned(),
        Err(error) => format!("{error:?}"),
    };
    format!("{label:<26} -> {outcome} (mode at publish: {mode})")
}

fn filesystem_report(root: &Path) -> Vec<String> {
    let mut report = vec![format!("root: {}", root.display())];
    if let Ok(metadata) = fs::metadata(root) {
        report.push(format!("device: {}", metadata.dev()));
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("stat")
            .args(["-f", "%T %Sd %b blocks free=%a", root.to_str().unwrap_or(".")])
            .output();
        if let Ok(output) = output {
            report.push(format!(
                "stat: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }
        let output = std::process::Command::new("sw_vers").output();
        if let Ok(output) = output {
            let text = String::from_utf8_lossy(&output.stdout).replace('\n', " ");
            report.push(format!("sw_vers: {}", text.trim()));
        }
    }
    report
}

#[test]
fn temporary_rename_capability_report() {
    let temp_parent = std::env::temp_dir()
        .canonicalize()
        .expect("physical temp parent");
    let root: PathBuf = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(temp_parent)
        .expect("probe fixture")
        .keep();

    let mut report = filesystem_report(&root);
    report.push("--- raw renameatx_np ---".to_owned());
    let cases = [
        Case {
            label: "excl-empty",
            source_mode: None,
            nested: false,
            hold_child: false,
            hold_source: false,
            flags: RenameFlags::NOREPLACE,
            freeze_after: false,
        },
        Case {
            label: "excl-nested",
            source_mode: None,
            nested: true,
            hold_child: false,
            hold_source: false,
            flags: RenameFlags::NOREPLACE,
            freeze_after: false,
        },
        Case {
            label: "excl-frozen-0500",
            source_mode: Some(0o500),
            nested: false,
            hold_child: false,
            hold_source: false,
            flags: RenameFlags::NOREPLACE,
            freeze_after: false,
        },
        Case {
            label: "excl-frozen-nested",
            source_mode: Some(0o500),
            nested: true,
            hold_child: false,
            hold_source: false,
            flags: RenameFlags::NOREPLACE,
            freeze_after: false,
        },
        Case {
            label: "excl-frozen-child-fd",
            source_mode: Some(0o500),
            nested: true,
            hold_child: true,
            hold_source: false,
            flags: RenameFlags::NOREPLACE,
            freeze_after: false,
        },
        Case {
            label: "excl-frozen-source-fd",
            source_mode: Some(0o500),
            nested: true,
            hold_child: false,
            hold_source: true,
            flags: RenameFlags::NOREPLACE,
            freeze_after: false,
        },
        Case {
            label: "excl-empty-child-fd",
            source_mode: None,
            nested: true,
            hold_child: true,
            hold_source: false,
            flags: RenameFlags::NOREPLACE,
            freeze_after: false,
        },
        Case {
            label: "plain-frozen-0500",
            source_mode: Some(0o500),
            nested: true,
            hold_child: false,
            hold_source: false,
            flags: RenameFlags::empty(),
            freeze_after: false,
        },
        Case {
            label: "rename-then-freeze",
            source_mode: None,
            nested: true,
            hold_child: false,
            hold_source: false,
            flags: RenameFlags::NOREPLACE,
            freeze_after: true,
        },
    ];
    for case in &cases {
        report.push(probe(&root, case));
    }

    report.push("--- production ExclusiveStage ---".to_owned());
    report.push(staging_probe(&root, "stage-default", false, 'a'));
    report.push(staging_probe(&root, "stage-frozen-0500", true, 'b'));

    let _ = fs::remove_dir_all(&root);
    // The probe must stay green so repository hooks and unrelated suites keep
    // working; the workflow reads the report through `--nocapture`.
    println!("\n{}\n", report.join("\n"));
}
