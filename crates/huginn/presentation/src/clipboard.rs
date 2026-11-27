//! Bounded image ingestion from OS clipboard and pasted/dragged local file paths.
use huginn_core::{ImageAttachment, MAX_IMAGE_BYTES};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

/// Check if a string looks like an unquoted Windows absolute path or UNC path:
/// e.g. `C:\...`, `c:/...`, or `\\server\share\...`.
fn is_windows_style_path(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    if s.starts_with(r"\\") {
        return true;
    }
    false
}

/// Unquote and unescape a single dragged/pasted local file path from terminal paste.
pub fn parse_pasted_image_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains('\r') {
        return None;
    }
    let unquoted = if (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
        || (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else if is_windows_style_path(trimmed) {
        trimmed.to_string()
    } else {
        // Terminal drag-and-drop on macOS/Linux escapes spaces and shell metacharacters with `\`.
        let mut out = String::with_capacity(trimmed.len());
        let mut chars = trimmed.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else {
                out.push(ch);
            }
        }
        out
    };
    let path = PathBuf::from(&unquoted);
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp") {
        return None;
    }
    Some(path)
}

/// If `raw` looks like a pasted/dragged image file path, read (bounded) and validate it.
/// Returns `None` when `raw` is ordinary text rather than an image file path.
pub fn try_load_pasted_image_path(raw: &str) -> Option<Result<ImageAttachment, String>> {
    let path = parse_pasted_image_path(raw)?;
    if !path.exists() {
        return None;
    }
    Some(load_image_file(&path))
}

pub fn load_image_file(path: &Path) -> Result<ImageAttachment, String> {
    let meta =
        std::fs::metadata(path).map_err(|_| "could not read image file metadata".to_string())?;
    if !meta.is_file() {
        return Err("pasted path is not a regular file".into());
    }
    if meta.len() > MAX_IMAGE_BYTES as u64 {
        return Err(format!(
            "image exceeds the {} MiB per-image limit",
            MAX_IMAGE_BYTES / (1024 * 1024)
        ));
    }
    let file = File::open(path).map_err(|_| "could not open image file".to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_IMAGE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read image file".to_string())?;
    let label = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned);
    ImageAttachment::from_bytes(bytes, label).map_err(|err| err.to_string())
}

/// Attempt to read an image from the native OS clipboard (macOS / Linux).
/// Returns `Ok(None)` when the clipboard does not currently hold an image.
pub fn read_clipboard_image() -> Result<Option<ImageAttachment>, String> {
    #[cfg(target_os = "macos")]
    {
        read_macos_clipboard_image()
    }
    #[cfg(target_os = "linux")]
    {
        read_linux_clipboard_image()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err("clipboard image paste is not supported on this platform; paste an image file path instead".into())
    }
}

const CLIPBOARD_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

#[cfg(target_os = "macos")]
fn read_macos_clipboard_image() -> Result<Option<ImageAttachment>, String> {
    let temp_path =
        std::env::temp_dir().join(format!("huginn-clipboard-{}.png", uuid::Uuid::new_v4()));
    let script = format!(
        "try\n\
         set imgData to the clipboard as «class PNGf»\n\
         set f to open for access POSIX file \"{}\" with write permission\n\
         set eof of f to 0\n\
         write imgData to f\n\
         close access f\n\
         return \"ok\"\n\
         on error\n\
         return \"none\"\n\
         end try",
        temp_path.display()
    );
    let mut child = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "could not access macOS clipboard".to_string())?;

    let deadline = std::time::Instant::now() + CLIPBOARD_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(&temp_path);
                    return Err("clipboard read timed out".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&temp_path);
                return Err(format!("error waiting for clipboard child: {err}"));
            }
        }
    };

    if !status.success() {
        let _ = std::fs::remove_file(&temp_path);
        return Err("could not read image from macOS clipboard".into());
    }
    let mut stdout = String::new();
    if let Some(mut stream) = child.stdout.take() {
        let _ = stream.read_to_string(&mut stdout);
    }
    if stdout.trim() != "ok" || !temp_path.exists() {
        let _ = std::fs::remove_file(&temp_path);
        return Ok(None);
    }
    let loaded = load_image_file(&temp_path);
    let _ = std::fs::remove_file(&temp_path);
    loaded.map(Some)
}

#[cfg(target_os = "linux")]
fn read_linux_clipboard_image() -> Result<Option<ImageAttachment>, String> {
    use std::process::Stdio;
    let mut any_tool_ran = false;
    for (bin, args) in [
        ("wl-paste", &["--no-newline", "--type", "image/png"][..]),
        (
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"][..],
        ),
    ] {
        if let Ok(mut child) = std::process::Command::new(bin)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            any_tool_ran = true;
            let (tx, rx) = std::sync::mpsc::channel();
            let reader_handle = child.stdout.take().map(|mut out| {
                std::thread::spawn(move || {
                    let mut bytes = Vec::new();
                    let _ = out
                        .take((MAX_IMAGE_BYTES as u64) + 1)
                        .read_to_end(&mut bytes);
                    let _ = tx.send(bytes);
                })
            });

            let deadline = std::time::Instant::now() + CLIPBOARD_TIMEOUT;
            let mut timed_out = false;
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Some(status),
                    Ok(None) => {
                        if std::time::Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            timed_out = true;
                            break None;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(15));
                    }
                    Err(_) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                }
            };

            if let Some(handle) = reader_handle {
                let _ = handle.join();
            }

            if timed_out {
                return Err("clipboard read timed out".into());
            }

            if let Some(status) = status {
                if status.success() {
                    if let Ok(bytes) = rx.try_recv() {
                        if !bytes.is_empty() {
                            return ImageAttachment::from_bytes(
                                bytes,
                                Some("clipboard.png".into()),
                            )
                            .map(Some)
                            .map_err(|err| err.to_string());
                        }
                    }
                }
            }
        }
    }
    if any_tool_ran {
        Ok(None)
    } else {
        Err(
            "no clipboard tool found (install wl-paste or xclip, or paste an image file path)"
                .into(),
        )
    }
}
