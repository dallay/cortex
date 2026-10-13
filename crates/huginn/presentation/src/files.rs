//! Filename-only workspace index. Run on a blocking worker, never while drawing.
use crate::state::safe;
use cap_std::ambient_authority;
use huginn_core::CancellationToken;
use std::{io, path::Path};

pub struct FileIndex {
    pub paths: Vec<String>,
    pub truncated: bool,
    pub skipped_directories: usize,
}
pub fn scan(root: &Path, stop: &CancellationToken, limit: usize) -> io::Result<FileIndex> {
    #[cfg(not(test))]
    return scan_inner(root, stop, limit);
    #[cfg(test)]
    scan_inner(root, stop, limit, |_| {})
}

fn scan_inner(
    root: &Path,
    stop: &CancellationToken,
    limit: usize,
    #[cfg(test)] mut before_open_directory: impl FnMut(&str),
) -> io::Result<FileIndex> {
    let root_dir = cap_std::fs::Dir::open_ambient_dir(root, ambient_authority())?;
    let mut index = FileIndex {
        paths: vec![],
        truncated: false,
        skipped_directories: 0,
    };
    let mut pending = vec![(root_dir, String::new(), true)];
    let mut visited = 0;
    while let Some((directory, prefix, is_root)) = pending.pop() {
        if stop.is_cancelled() {
            index.paths.sort();
            index.truncated = true;
            return Ok(index);
        }
        let entries = match directory.entries() {
            Ok(entries) => entries,
            Err(error) if is_root => return Err(error),
            Err(_) => {
                index.skipped_directories += 1;
                continue;
            }
        };
        let mut children = vec![];
        for entry in entries {
            if stop.is_cancelled() {
                index.paths.sort();
                index.truncated = true;
                return Ok(index);
            }
            if visited >= limit {
                index.truncated = true;
                break;
            }
            visited += 1;
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let relative = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if matches!(
                    name,
                    ".git" | "target" | "node_modules" | ".pnpm-store" | ".cache"
                ) {
                    continue;
                }
                #[cfg(test)]
                before_open_directory(&relative);
                match entry.open_dir() {
                    Ok(child) => children.push((child, relative, false)),
                    Err(_) => index.skipped_directories += 1,
                }
            } else if kind.is_file()
                && !relative.chars().any(char::is_control)
                && safe(&relative) == relative
            {
                index.paths.push(relative);
            }
        }
        children.sort_by(|a, b| a.1.cmp(&b.1));
        pending.extend(children);
    }
    index.paths.sort();
    Ok(index)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink, sync::mpsc};

    #[test]
    fn directory_replaced_with_symlink_between_listing_and_open_is_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("child")).unwrap();
        fs::write(outside.path().join("secret"), "not indexed").unwrap();

        let root_path = root.path().to_owned();
        let outside_path = outside.path().to_owned();
        let (listed_tx, listed_rx) = mpsc::channel();
        let (replaced_tx, replaced_rx) = mpsc::channel();
        let replacer = std::thread::spawn(move || {
            listed_rx.recv().unwrap();
            fs::remove_dir(root_path.join("child")).unwrap();
            symlink(outside_path, root_path.join("child")).unwrap();
            replaced_tx.send(()).unwrap();
        });

        let result = scan_inner(root.path(), &CancellationToken::new(), 20_000, |name| {
            if name == "child" {
                listed_tx.send(()).unwrap();
                replaced_rx.recv().unwrap();
            }
        })
        .unwrap();
        replacer.join().unwrap();

        assert!(!result.paths.iter().any(|path| path.contains("secret")));
    }

    #[test]
    fn cancellation_during_scan_returns_sorted_truncated_index() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("child")).unwrap();
        fs::write(root.path().join("a.txt"), "").unwrap();
        fs::write(root.path().join("z.txt"), "").unwrap();
        let stop = CancellationToken::new();
        let cancelled = stop.clone();

        let result = scan_inner(root.path(), &stop, 20_000, move |name| {
            if name == "child" {
                cancelled.cancel();
            }
        })
        .unwrap();

        assert!(result.truncated);
        assert!(result.paths.windows(2).all(|paths| paths[0] <= paths[1]));
    }
}
