use crate::error::{Error, Result};
use crate::safety::deceptive_character;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

pub fn safe_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || deceptive_character(c) {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

pub fn explanation(text: &str) -> Result<()> {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "\nExplanation (model-generated; may be wrong):")?;
    for line in text.lines() {
        writeln!(stderr, "  {}", safe_text(line))?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Run,
    Edit,
    Cancel,
}

pub fn decision(reader: &mut impl BufRead, risky: bool) -> Result<Decision> {
    let mut input = String::new();
    if reader.read_line(&mut input)? == 0 {
        return Ok(Decision::Cancel);
    }
    Ok(match input.trim() {
        "RUN" if risky => Decision::Run,
        "y" | "Y" | "yes" | "YES" if !risky => Decision::Run,
        "e" | "E" => Decision::Edit,
        _ => Decision::Cancel,
    })
}

pub fn confirm_command(risky: bool) -> Result<Decision> {
    if !interactive() {
        return Ok(Decision::Cancel);
    }
    let mut stderr = io::stderr().lock();
    if risky {
        write!(
            stderr,
            "Type RUN to execute, e to edit, or Enter to cancel: "
        )?;
    } else {
        write!(stderr, "Run? (y/e/N; Enter cancels) ")?;
    }
    stderr.flush()?;
    decision(&mut io::stdin().lock(), risky)
}

pub fn confirm(prompt: &str) -> Result<bool> {
    if !interactive() {
        return Err(
            "This action requires a terminal and explicit confirmation (or --yes where supported)."
                .into(),
        );
    }
    let input = input(prompt, "")?;
    Ok(matches!(input.to_lowercase().as_str(), "y" | "yes"))
}

pub fn input(prompt: &str, default: &str) -> Result<String> {
    let mut stderr = io::stderr().lock();
    if default.is_empty() {
        write!(stderr, "{prompt}: ")?;
    } else {
        write!(stderr, "{prompt} [{}]: ", safe_text(default))?;
    }
    stderr.flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Err(Error::Cancelled);
    }
    let value = line.trim();
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value.to_owned()
    })
}

pub fn secret(prompt: &str) -> Result<Option<String>> {
    if !interactive() {
        return Err(
            "Credential entry requires a terminal; configure environment variables instead.".into(),
        );
    }
    // Disable echo before exposing the prompt. Otherwise a fast paste can race
    // the password reader's own terminal setup and briefly reveal the key.
    let raw = RawModeGuard::new()?;
    let mut stderr = io::stderr().lock();
    write!(stderr, "{prompt}: ")?;
    stderr.flush()?;
    let secret = rpassword::read_password().map_err(|error| match error.kind() {
        io::ErrorKind::Interrupted | io::ErrorKind::UnexpectedEof => Error::Cancelled,
        _ => Error::Io(error),
    })?;
    drop(raw);
    writeln!(stderr)?;
    Ok((!secret.is_empty()).then_some(secret))
}

pub fn edit(command: &str) -> Result<Option<String>> {
    let config = rustyline::Config::builder()
        .behavior(rustyline::config::Behavior::PreferTerm)
        .check_cursor_position(false)
        .build();
    let mut editor = rustyline::DefaultEditor::with_config(config)
        .map_err(|e| format!("Cannot open command editor: {e}"))?;
    match editor.readline_with_initial("  > ", (command, "")) {
        Ok(command) => Ok(Some(command)),
        Err(
            rustyline::error::ReadlineError::Interrupted | rustyline::error::ReadlineError::Eof,
        ) => Ok(None),
        Err(e) => Err(format!("Command editor failed: {e}").into()),
    }
}

struct RawModeGuard {
    restore_raw: bool,
}

impl RawModeGuard {
    fn new() -> Result<Self> {
        let restore_raw = !terminal::is_raw_mode_enabled()?;
        if restore_raw {
            terminal::enable_raw_mode()?;
        }
        Ok(Self { restore_raw })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.restore_raw {
            let _ = terminal::disable_raw_mode();
        }
    }
}

struct ProgressLine;

impl Drop for ProgressLine {
    fn drop(&mut self) {
        let _ = write!(io::stderr(), "\r\x1b[2K");
        let _ = io::stderr().flush();
    }
}

/// The worker only performs network reads. Cancellation never leaves disk writes in flight.
pub fn progress<T: Send + 'static>(
    label: &str,
    task: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    if !interactive() || std::env::var("TERM").is_ok_and(|term| term == "dumb") {
        return task();
    }
    let _raw = RawModeGuard::new()?;
    let _line = ProgressLine;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(task());
    });
    let started = Instant::now();
    loop {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Request worker stopped unexpectedly.".into())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        while event::poll(Duration::ZERO)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && (key.code == KeyCode::Esc
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)))
                {
                    return Err(Error::Cancelled);
                }
            }
        }
        write!(
            io::stderr(),
            "\r{label}... {:.1}s (Esc/Ctrl+C cancels)",
            started.elapsed().as_secs_f32()
        )?;
        io::stderr().flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn confirmation_fails_closed() {
        for input in ["", "\n", "n\n", "maybe\n"] {
            assert_eq!(
                decision(&mut Cursor::new(input), false).unwrap(),
                Decision::Cancel
            );
        }
        assert_eq!(
            decision(&mut Cursor::new("y\n"), false).unwrap(),
            Decision::Run
        );
        assert_eq!(
            decision(&mut Cursor::new("e\n"), false).unwrap(),
            Decision::Edit
        );
    }

    #[test]
    fn risky_confirmation_requires_exact_acknowledgement() {
        assert_eq!(
            decision(&mut Cursor::new("y\n"), true).unwrap(),
            Decision::Cancel
        );
        assert_eq!(
            decision(&mut Cursor::new("run\n"), true).unwrap(),
            Decision::Cancel
        );
        assert_eq!(
            decision(&mut Cursor::new("RUN\n"), true).unwrap(),
            Decision::Run
        );
    }

    #[test]
    fn terminal_controls_are_escaped() {
        assert_eq!(safe_text("a\x1b[2J\nb"), "a\\u{1b}[2J\\nb");
        assert_eq!(safe_text("café"), "café");
        assert!(!safe_text("\u{202e}").contains('\u{202e}'));
    }

    #[test]
    fn read_errors_do_not_approve_execution() {
        struct Broken;
        impl io::Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::Other.into())
            }
        }
        impl BufRead for Broken {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                Err(io::ErrorKind::Other.into())
            }
            fn consume(&mut self, _: usize) {}
        }
        assert!(decision(&mut Broken, false).is_err());
    }
}
