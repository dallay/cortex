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

    // Keep lookups relative to the descriptor directory. Rebuilding each slot
    // name from its numeric value avoids using untrusted directory-entry text.
    let descriptors = rustix::fs::open(
        directory,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()?;

    let mut slots = Vec::new();
    for entry in std::fs::read_dir(directory).ok()? {
        let Ok(entry) = entry else {
            return None;
        };
        let Some(slot) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
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
        Err(rustix::io::Errno::BADF) => return Some(false),
        Err(_) => return None,
    };

    // /dev/fd entries identify the descriptor slot, but reopening it is needed
    // to read the referenced object's device. NONBLOCK prevents FIFO stalls.
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
        // A write-only or otherwise non-reopenable descriptor can still be
        // identified by the inode reported for its /dev/fd slot.
        Err(_) => Some(stat.st_ino == inode),
    }
}

/// On Linux, descriptor entries are symlinks to the referenced objects.
#[cfg(target_os = "linux")]
fn slot_matches(
    descriptors: &rustix::fd::OwnedFd,
    slot: &str,
    device: u64,
    inode: u64,
) -> Option<bool> {
    match rustix::fs::statat(descriptors, slot, rustix::fs::AtFlags::empty()) {
        Ok(stat) => Some(stat.st_dev == device && stat.st_ino == inode),
        Err(error) => {
            if error != rustix::io::Errno::NOENT {
                return None;
            }
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn descriptor_probe_finds_a_known_live_file_descriptor() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().expect("fixture");
        let path = root.path().join("open");
        std::fs::write(&path, b"content").expect("fixture file");
        let file = std::fs::File::open(&path).expect("open fixture");
        let metadata = file.metadata().expect("open file metadata");

        assert_eq!(
            super::descriptor_open_for(metadata.dev(), metadata.ino()),
            Some(true),
            "the probe must find a descriptor that remains open"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn descriptor_probe_does_not_block_on_a_fifo_without_writers() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().expect("fixture");
        let path = root.path().join("fifo");
        #[cfg(target_os = "linux")]
        rustix::fs::mkfifoat(rustix::fs::CWD, &path, rustix::fs::Mode::RUSR).expect("fixture fifo");

        #[cfg(target_os = "macos")]
        {
            let status = std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .expect("create fixture fifo");
            assert!(status.success(), "mkfifo command failed: {status}");
        }
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
        assert_eq!(
            receiver.recv_timeout(std::time::Duration::from_secs(5)),
            Ok(Some(true)),
            "metadata probing must detect a live FIFO without waiting for a writer"
        );
        worker.join().expect("probe worker");
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
