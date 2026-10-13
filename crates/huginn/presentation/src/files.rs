//! Filename-only workspace index. Run on a blocking worker, never while drawing.
use crate::state::safe;
use huginn_core::CancellationToken;
use std::{fs, io, path::Path};

pub struct FileIndex {
    pub paths: Vec<String>,
    pub truncated: bool,
    pub skipped_directories: usize,
}
pub fn scan(root: &Path, stop: &CancellationToken, limit: usize) -> io::Result<FileIndex> {
    let root = root.canonicalize()?;
    let mut index = FileIndex {
        paths: vec![],
        truncated: false,
        skipped_directories: 0,
    };
    let mut pending = vec![root.clone()];
    let mut visited = 0;
    while let Some(directory) = pending.pop() {
        if stop.is_cancelled() {
            return Ok(index);
        }
        // Recheck before enumerating: do not knowingly follow a directory
        // replaced with a symlink after its parent was scanned.
        if directory != root
            && (!fs::symlink_metadata(&directory)
                .is_ok_and(|metadata| metadata.file_type().is_dir())
                || !directory
                    .canonicalize()
                    .is_ok_and(|path| path.starts_with(&root)))
        {
            index.skipped_directories += 1;
            continue;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory == root => return Err(error),
            Err(_) => {
                index.skipped_directories += 1;
                continue;
            }
        };
        let mut children = vec![];
        for entry in entries {
            if stop.is_cancelled() {
                return Ok(index);
            }
            if visited >= limit {
                index.truncated = true;
                break;
            }
            visited += 1;
            match entry {
                Ok(entry) => children.push(entry),
                Err(_) => index.skipped_directories += 1,
            }
        }
        children.sort_by_key(|entry| entry.file_name());
        for entry in children {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if kind.is_dir() {
                if !matches!(
                    name,
                    ".git" | "target" | "node_modules" | ".pnpm-store" | ".cache"
                ) {
                    pending.push(path);
                }
            } else if kind.is_file() {
                let Some(relative) = path.strip_prefix(&root).ok().and_then(|path| path.to_str())
                else {
                    continue;
                };
                if !relative.chars().any(char::is_control) && safe(relative) == relative {
                    index.paths.push(relative.into());
                }
            }
        }
        if index.truncated {
            break;
        }
    }
    index.paths.sort();
    Ok(index)
}
