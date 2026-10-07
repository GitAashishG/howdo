use crate::error::Result;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn copy(command: &str) -> Result<()> {
    let candidates: Vec<(&str, Vec<&str>)> = if cfg!(target_os = "macos") {
        vec![("pbcopy", vec![])]
    } else if cfg!(windows) {
        let script = "[Console]::InputEncoding = [System.Text.UTF8Encoding]::new(); Set-Clipboard -Value ([Console]::In.ReadToEnd())";
        vec![
            (
                "pwsh",
                vec!["-NoProfile", "-NonInteractive", "-Command", script],
            ),
            (
                "powershell",
                vec!["-NoProfile", "-NonInteractive", "-Command", script],
            ),
        ]
    } else {
        let mut candidates = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            candidates.push(("wl-copy", vec![]));
        }
        candidates.extend([
            ("xclip", vec!["-selection", "clipboard"]),
            ("xsel", vec!["--clipboard", "--input"]),
        ]);
        candidates
    };
    for (program, arguments) in candidates {
        let mut child = match Command::new(program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Cannot launch clipboard tool {program}: {e}").into()),
        };
        let write_result = child
            .stdin
            .take()
            .ok_or("Clipboard tool has no stdin.")?
            .write_all(command.as_bytes());
        if let Err(error) = write_result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Cannot write to clipboard tool: {error}").into());
        }
        // Clipboard owners may fork and retain their descriptors. Waiting for a
        // piped stderr to close would hang until that clipboard ownership ends.
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > Duration::from_secs(5) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    "Clipboard tool timed out. Check your desktop session and clipboard access."
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        if !status.success() {
            return Err(format!("Clipboard tool {program} failed with status {status}.").into());
        }
        return Ok(());
    }
    Err(
        "No clipboard tool is available. Use --print, or install wl-clipboard/xclip/xsel on Linux."
            .into(),
    )
}
