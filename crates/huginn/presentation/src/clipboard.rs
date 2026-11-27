//! Bounded image ingestion from OS clipboard and pasted/dragged local file paths.
use huginn_core::{ImageAttachment, MAX_IMAGE_BYTES};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

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
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|_| "could not access macOS clipboard".to_string())?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&temp_path);
        return Err("could not read image from macOS clipboard".into());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
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
    for (bin, args) in [
        ("wl-paste", &["--no-newline", "--type", "image/png"][..]),
        (
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"][..],
        ),
    ] {
        if let Ok(output) = std::process::Command::new(bin).args(args).output() {
            if output.status.success() && !output.stdout.is_empty() {
                return ImageAttachment::from_bytes(output.stdout, Some("clipboard.png".into()))
                    .map(Some)
                    .map_err(|err| err.to_string());
            }
            return Ok(None);
        }
    }
    Err("no clipboard tool found (install wl-paste or xclip, or paste an image file path)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_and_escaped_image_paths_only() {
        assert_eq!(
            parse_pasted_image_path("'/tmp/my shot.png'"),
            Some(PathBuf::from("/tmp/my shot.png"))
        );
        assert_eq!(
            parse_pasted_image_path(r"/tmp/my\ shot.webp"),
            Some(PathBuf::from("/tmp/my shot.webp"))
        );
        assert_eq!(parse_pasted_image_path("hello world"), None);
        assert_eq!(parse_pasted_image_path("/tmp/notes.txt"), None);
    }

    #[test]
    fn loads_valid_png_path_and_ui_state_manages_preview_and_removal() {
        let temp =
            std::env::temp_dir().join(format!("huginn-img-test-{}.png", uuid::Uuid::new_v4()));
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&32u32.to_be_bytes());
        png.extend_from_slice(&16u32.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0]);
        std::fs::write(&temp, &png).unwrap();

        let loaded = try_load_pasted_image_path(&format!("'{}'", temp.display()))
            .unwrap()
            .unwrap();
        let _ = std::fs::remove_file(&temp);

        let mut state = crate::state::UiState::default();
        for _ in 0..huginn_core::MAX_IMAGES_PER_MESSAGE {
            state.attach_image(loaded.clone()).unwrap();
        }
        assert!(state.attach_image(loaded).is_err());
        assert_eq!(state.attachment_chips()[0], "[Image 1: PNG 32x16, 29 B]");
        assert!(state.remove_last_attachment());
        assert_eq!(
            state.attachments.len(),
            huginn_core::MAX_IMAGES_PER_MESSAGE - 1
        );
    }
}
