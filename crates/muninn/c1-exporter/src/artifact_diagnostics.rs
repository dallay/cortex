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
    use std::os::unix::fs::MetadataExt;

    let directory = if cfg!(target_os = "macos") {
        "/dev/fd"
    } else {
        "/proc/self/fd"
    };
    let entries = std::fs::read_dir(directory).ok()?;
    let mut conclusive = true;
    for entry in entries {
        let Ok(entry) = entry else {
            conclusive = false;
            continue;
        };
        // Follow the descriptor link for metadata only. Opening it for I/O can
        // block on inherited FIFOs and needlessly reopens other objects.
        let Ok(metadata) = std::fs::metadata(entry.path()) else {
            conclusive = false;
            continue;
        };
        if metadata.dev() == device && metadata.ino() == inode {
            return Some(true);
        }
    }
    conclusive.then_some(false)
}

#[cfg(test)]
mod tests {
    use super::format_error;

    #[test]
    fn descriptor_probe_does_not_block_on_a_fifo_without_writers() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().expect("fixture");
        let path = root.path().join("fifo");
        rustix::fs::mkfifoat(rustix::fs::CWD, &path, rustix::fs::Mode::RUSR).expect("fixture fifo");
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
