//! Bounded image ingestion from OS clipboard and pasted/dragged local file paths.
use huginn_core::{CancellationToken, ImageAttachment, MAX_IMAGE_BYTES};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::mpsc::{self, TryRecvError},
    thread,
    time::{Duration, Instant},
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
pub fn read_clipboard_image(stop: &CancellationToken) -> Result<Option<ImageAttachment>, String> {
    #[cfg(target_os = "macos")]
    {
        read_macos_clipboard_image(stop)
    }
    #[cfg(target_os = "linux")]
    {
        read_linux_clipboard_image(stop)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = stop;
        Err("clipboard image paste is not supported on this platform; paste an image file path instead".into())
    }
}

const CLIPBOARD_TIMEOUT: Duration = Duration::from_millis(1500);

fn stop_child(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(raw_pid) = i32::try_from(child.id()) {
        if let Some(pid) = rustix::process::Pid::from_raw(raw_pid) {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn join_output_reader(reader: Option<thread::JoinHandle<()>>) {
    if let Some(reader) = reader {
        let _ = reader.join();
    }
}

fn finish_clipboard_output(
    child: &mut Child,
    reader: Option<thread::JoinHandle<()>>,
    stop: &CancellationToken,
    status: ExitStatus,
    stdout: Vec<u8>,
) -> Result<Output, String> {
    join_output_reader(reader);
    if stop.is_cancelled() {
        stop_child(child);
        return Err("clipboard read cancelled".into());
    }
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

fn run_clipboard_command(
    command: &mut Command,
    stop: &CancellationToken,
    timeout: Duration,
) -> Result<Output, String> {
    if stop.is_cancelled() {
        return Err("clipboard read cancelled".into());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "could not start clipboard command".to_string())?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = child.stdout.take().map(|stdout| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout
                .take((MAX_IMAGE_BYTES as u64) + 1)
                .read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        })
    });
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut stdout = None;
    loop {
        if stop.is_cancelled() {
            stop_child(&mut child);
            join_output_reader(reader);
            return Err("clipboard read cancelled".into());
        }

        if status.is_none() {
            match child.try_wait() {
                Ok(child_status) => status = child_status,
                Err(error) => {
                    stop_child(&mut child);
                    join_output_reader(reader);
                    return Err(format!("error waiting for clipboard child: {error}"));
                }
            }
        }
        if stdout.is_none() {
            match receiver.try_recv() {
                Ok(bytes) => stdout = Some(bytes),
                Err(TryRecvError::Disconnected) => {
                    stop_child(&mut child);
                    join_output_reader(reader);
                    return Err("could not read clipboard command output".into());
                }
                Err(TryRecvError::Empty) => {}
            }
        }

        if let Some(status) = status.as_ref() {
            if let Some(stdout) = stdout.take() {
                return finish_clipboard_output(
                    &mut child,
                    reader,
                    stop,
                    status.to_owned(),
                    stdout,
                );
            }
        }
        if Instant::now() >= deadline {
            stop_child(&mut child);
            join_output_reader(reader);
            return Err("clipboard read timed out".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(all(feature = "test-hooks", debug_assertions))]
#[doc(hidden)]
pub fn cancel_after_child_exit_for_test(
    command: &mut Command,
    stop: &CancellationToken,
) -> Result<(), String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "could not start clipboard command".to_string())?;
    let status = child
        .wait()
        .map_err(|error| format!("error waiting for clipboard child: {error}"))?;
    stop.cancel();
    finish_clipboard_output(&mut child, None, stop, status, Vec::new()).map(|_| ())
}

#[cfg(all(feature = "test-hooks", debug_assertions))]
#[doc(hidden)]
pub fn run_command_for_test(
    command: &mut Command,
    stop: &CancellationToken,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let output = run_clipboard_command(command, stop, timeout)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err("clipboard command failed".into())
    }
}

#[cfg(any(target_os = "linux", all(feature = "test-hooks", debug_assertions)))]
fn read_clipboard_commands(
    stop: &CancellationToken,
    commands: impl IntoIterator<Item = Command>,
) -> Result<Option<ImageAttachment>, String> {
    let mut any_tool_ran = false;
    for mut command in commands {
        let output = match run_clipboard_command(&mut command, stop, CLIPBOARD_TIMEOUT) {
            Ok(output) => {
                any_tool_ran = true;
                output
            }
            Err(error) if error == "could not start clipboard command" => continue,
            Err(error) => return Err(error),
        };
        if output.status.success() && !output.stdout.is_empty() {
            return ImageAttachment::from_bytes(output.stdout, Some("clipboard.png".into()))
                .map(Some)
                .map_err(|error| error.to_string());
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

#[cfg(all(feature = "test-hooks", debug_assertions))]
#[doc(hidden)]
pub fn read_commands_for_test(
    stop: &CancellationToken,
    commands: Vec<Command>,
) -> Result<Option<ImageAttachment>, String> {
    read_clipboard_commands(stop, commands)
}

#[cfg(target_os = "macos")]
fn read_macos_clipboard_image(stop: &CancellationToken) -> Result<Option<ImageAttachment>, String> {
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
    let mut command = Command::new("osascript");
    command.arg("-e").arg(script);
    let output = match run_clipboard_command(&mut command, stop, CLIPBOARD_TIMEOUT) {
        Ok(output) => output,
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(error);
        }
    };
    if !output.status.success() {
        let _ = std::fs::remove_file(&temp_path);
        return Err("could not read image from macOS clipboard".into());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim() != "ok" || !temp_path.exists() {
        let _ = std::fs::remove_file(&temp_path);
        return Ok(None);
    }
    if stop.is_cancelled() {
        let _ = std::fs::remove_file(&temp_path);
        return Err("clipboard read cancelled".into());
    }
    let loaded = load_image_file(&temp_path);
    let _ = std::fs::remove_file(&temp_path);
    loaded.map(Some)
}

#[cfg(target_os = "linux")]
fn read_linux_clipboard_image(stop: &CancellationToken) -> Result<Option<ImageAttachment>, String> {
    let mut wayland = Command::new("wl-paste");
    wayland.args(["--no-newline", "--type", "image/png"]);
    let mut x11 = Command::new("xclip");
    x11.args(["-selection", "clipboard", "-t", "image/png", "-o"]);
    read_clipboard_commands(stop, [wayland, x11])
}
