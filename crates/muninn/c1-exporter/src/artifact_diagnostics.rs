//! Test-only filesystem diagnostics. Never format native error messages or paths.

use std::cell::RefCell;

fn format_error(operation: &'static str, error: &std::io::Error) -> String {
    format!(
        "[c1-fs-test] operation={operation} errno={:?}",
        error.raw_os_error()
    )
}

pub(super) fn report(operation: &'static str, error: &std::io::Error) {
    eprintln!("{}", format_error(operation, error));
}

thread_local! {
    static PRE_RENAME_PROBE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Removes the installed pre-rename probe when it goes out of scope.
pub(super) struct PreRenameProbeGuard;

impl Drop for PreRenameProbeGuard {
    fn drop(&mut self) {
        PRE_RENAME_PROBE.with(|probe| *probe.borrow_mut() = None);
    }
}

/// Installs a probe that runs immediately before the publication rename.
pub(super) fn set_pre_rename_probe(probe: impl Fn() + 'static) -> PreRenameProbeGuard {
    PRE_RENAME_PROBE.with(|slot| *slot.borrow_mut() = Some(Box::new(probe)));
    PreRenameProbeGuard
}

/// Runs the installed pre-rename probe, if any.
pub(super) fn probe_pre_rename() {
    PRE_RENAME_PROBE.with(|probe| {
        if let Some(probe) = probe.borrow().as_ref() {
            probe();
        }
    });
}

/// Reports whether an open descriptor still refers to the given inode.
/// Returns None when a failed scan cannot establish that it is closed.
pub(super) fn descriptor_open_for(device: u64, inode: u64) -> Option<bool> {
    let directory = if cfg!(target_os = "macos") {
        "/dev/fd"
    } else {
        "/proc/self/fd"
    };

    // A single descriptor for the directory keeps every lookup relative to it,
    // so no path is ever built from a directory entry name.
    let descriptors = rustix::fs::open(
        directory,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()?;

    // Snapshot the slot names before resolving them. Resolving a slot reopens a
    // descriptor, and the descriptor directory lists descriptors as they live.
    let mut slots = Vec::new();
    for entry in std::fs::read_dir(directory).ok()? {
        let Ok(entry) = entry else {
            return None;
        };
        let name = entry.file_name();
        // The descriptor directory only holds decimal slot names. Rebuilding the
        // name from the parsed number keeps the lookup free of any entry text.
        let Some(slot) = name.to_str().and_then(|text| text.parse::<u32>().ok()) else {
            continue;
        };
        slots.push(slot.to_string());
    }

    let mut conclusive = true;
    for slot in &slots {
        match slot_matches(&descriptors, slot, device, inode) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => conclusive = false,
        }
    }
    conclusive.then_some(false)
}

/// Classifies one descriptor slot against the target identity.
/// Returns None when the slot cannot be identified with certainty.
#[cfg(target_os = "macos")]
fn slot_matches(
    descriptors: &rustix::fd::OwnedFd,
    slot: &str,
    device: u64,
    inode: u64,
) -> Option<bool> {
    let stat = match rustix::fs::statat(descriptors, slot, rustix::fs::AtFlags::empty()) {
        Ok(stat) => stat,
        // A closed slot proves that no descriptor refers to the target.
        Err(rustix::io::Errno::BADF) => return Some(false),
        Err(_) => return None,
    };
    // macOS exposes every descriptor as a node of the descriptor filesystem. The
    // node carries the inode of the referenced object but not its device, so the
    // device is only readable from a reopened descriptor. The reopen is
    // non-blocking, so a descriptor on a FIFO cannot stall the scan.
    match rustix::fs::openat(
        descriptors,
        slot,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(handle) => {
            let reopened = rustix::fs::fstat(&handle).ok()?;
            let reopened_device = u64::try_from(reopened.st_dev).ok()?;
            Some(reopened_device == device && reopened.st_ino == inode)
        }
        // The slot cannot be reopened, for example because it is write-only, so
        // only the inode stays verifiable.
        Err(_) => Some(stat.st_ino == inode),
    }
}

/// Classifies one descriptor slot against the target identity.
/// Returns None when the slot cannot be identified with certainty.
#[cfg(not(target_os = "macos"))]
fn slot_matches(
    descriptors: &rustix::fd::OwnedFd,
    slot: &str,
    device: u64,
    inode: u64,
) -> Option<bool> {
    match rustix::fs::statat(descriptors, slot, rustix::fs::AtFlags::empty()) {
        // Linux exposes every descriptor as a symlink to the object it refers
        // to, so the link target already carries the real device and inode.
        Ok(stat) => Some(stat.st_dev == device && stat.st_ino == inode),
        Err(error) => {
            if error != rustix::io::Errno::NOENT {
                return None;
            }
            // The link target is gone. The descriptor is closed only when the
            // link itself is gone too.
            match rustix::fs::statat(descriptors, slot, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => None,
                Err(_) => Some(false),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_error;

    #[test]
    fn descriptor_probe_does_not_block_on_a_fifo_without_writers() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().expect("fixture");
        let path = root.path().join("fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("create fixture fifo");
        assert!(status.success(), "mkfifo command failed: {status}");
        let fifo = std::fs::File::from(
            rustix::fs::open(
                &path,
                rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK,
                rustix::fs::Mode::empty(),
            )
            .expect("nonblocking reader"),
        );
        let metadata = fifo.metadata().expect("fifo metadata");
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = super::descriptor_open_for(metadata.dev(), metadata.ino());
            drop(fifo);
            sender.send(result).expect("probe result");
        });
        let result = receiver.recv_timeout(std::time::Duration::from_secs(5));
        assert!(
            result.is_ok(),
            "metadata probing must complete without waiting for a writer"
        );
        assert_eq!(
            result.expect("probe result received"),
            Some(true),
            "the probe must detect the open FIFO descriptor"
        );
        worker.join().expect("probe worker");
    }

    #[test]
    fn descriptor_scan_reports_a_closed_descriptor_as_conclusive() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().expect("fixture");
        let path = root.path().join("closed");
        std::fs::write(&path, b"content").expect("fixture file");
        let metadata = std::fs::metadata(&path).expect("fixture metadata");
        assert_eq!(
            super::descriptor_open_for(metadata.dev(), metadata.ino()),
            Some(false),
            "a closed descriptor must be reported as closed, not as inconclusive"
        );
    }

    #[test]
    fn diagnostics_include_operation_and_errno_without_error_payloads() {
        let native = std::io::Error::from_raw_os_error(13);
        assert_eq!(
            format_error("publish.rename_noreplace", &native),
            "[c1-fs-test] operation=publish.rename_noreplace errno=Some(13)"
        );
        let sensitive = std::io::Error::other("/private/sensitive/model: secret payload");
        assert_eq!(
            format_error("publish.pre_sync", &sensitive),
            "[c1-fs-test] operation=publish.pre_sync errno=None"
        );
    }
}
