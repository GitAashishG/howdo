use crate::error::Result;
use base64::Engine;
use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Posix,
    PowerShell,
    Cmd,
}

#[derive(Clone, Debug)]
pub struct Shell {
    pub executable: PathBuf,
    pub name: String,
    pub kind: Kind,
}

impl Shell {
    pub fn detect(explicit: Option<&str>) -> Result<Self> {
        let executable = explicit
            .map(str::to_owned)
            .or_else(|| env::var("HOWDO_SHELL").ok().filter(|s| !s.is_empty()))
            .or_else(|| {
                if cfg!(windows) {
                    // Inherited PSModulePath does not identify the calling shell.
                    // Use a deterministic cmd default unless the user explicitly selects PowerShell.
                    env::var("COMSPEC").ok()
                } else {
                    env::var("SHELL").ok().filter(|s| !s.is_empty())
                }
            })
            .unwrap_or_else(|| if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into());
        Self::from_executable(&executable)
    }

    pub fn from_executable(executable: &str) -> Result<Self> {
        if executable.is_empty() || executable.chars().any(|c| c.is_control()) {
            return Err(
                "Shell must be an executable name or path, not a command with arguments.".into(),
            );
        }
        let basename = executable
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(executable)
            .to_lowercase();
        let basename = basename.strip_suffix(".exe").unwrap_or(&basename);
        let (kind, name) = match basename {
            "pwsh" => (Kind::PowerShell, "PowerShell 7 (pwsh)".into()),
            "powershell" => (Kind::PowerShell, "Windows PowerShell".into()),
            "cmd" => (Kind::Cmd, "cmd.exe".into()),
            "sh" | "bash" | "zsh" | "fish" | "dash" | "ash" | "ksh" | "mksh" => (Kind::Posix, basename.to_owned()),
            _ => return Err("Unsupported shell. Choose sh, bash, zsh, fish, dash, ash, ksh, mksh, powershell, pwsh, or cmd.".into()),
        };
        Ok(Self {
            executable: executable.into(),
            name,
            kind,
        })
    }

    pub fn arguments(&self, command: &str) -> Vec<String> {
        match self.kind {
            Kind::Posix => vec!["-c".into(), command.into()],
            Kind::Cmd => vec!["/D".into(), "/S".into(), "/C".into(), command.into()],
            Kind::PowerShell => {
                // A newline prevents an inline comment from swallowing the status check.
                let script = format!("$ErrorActionPreference = 'Stop'; $global:LASTEXITCODE = 0\n{command}\n$__howdoSuccess = $?; if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}; if (-not $__howdoSuccess) {{ exit 1 }}");
                // EncodedCommand uses UTF-16LE on all platforms and avoids the Windows
                // command-line parser changing quotes, backslashes, or Unicode in the code.
                let bytes: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
                vec![
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-OutputFormat".into(),
                    "Text".into(),
                    "-EncodedCommand".into(),
                    base64::engine::general_purpose::STANDARD.encode(bytes),
                ]
            }
        }
    }

    pub fn run(&self, command: &str) -> Result<i32> {
        let mut process = Command::new(&self.executable);
        #[cfg(windows)]
        if self.kind == Kind::Cmd {
            use std::os::windows::process::CommandExt;
            // cmd does not understand the C-runtime escaping used by Command::arg.
            // /S removes just the outer quotes, leaving reviewed command text intact.
            process
                .args(["/D", "/S", "/C"])
                .raw_arg(format!("\"{command}\""));
        } else {
            process.args(self.arguments(command));
        }
        #[cfg(not(windows))]
        process.args(self.arguments(command));
        let status = process
            .status()
            .map_err(|e| format!("Failed to execute {}: {e}", self.executable.display()))?;
        Ok(exit_code(status))
    }

    pub fn alias(&self) -> Option<(&'static str, &'static str)> {
        match self.name.as_str() {
            "zsh" => Some((".zshrc", "alias q='noglob howdo'")),
            "bash" => Some((".bashrc", "alias q='howdo'")),
            _ => None,
        }
    }
}

fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        128 + status.signal().unwrap_or(1)
    }
    #[cfg(not(unix))]
    1
}

pub fn os() -> String {
    match env::consts::OS {
        "macos" => "macOS (Darwin/BSD; do not assume GNU utilities)".into(),
        "linux" => {
            let distribution =
                std::fs::read_to_string("/etc/os-release")
                    .ok()
                    .and_then(|contents| {
                        contents.lines().find_map(|line| {
                            line.strip_prefix("PRETTY_NAME=")
                                .map(|s| s.trim_matches('"').to_owned())
                        })
                    });
            distribution.map_or_else(|| "Linux".into(), |name| format!("Linux ({name})"))
        }
        "windows" => "Windows".into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_full_shell_paths() {
        let shell = Shell::from_executable("/opt/custom/bin/bash").unwrap();
        assert_eq!(shell.executable, PathBuf::from("/opt/custom/bin/bash"));
        assert_eq!(shell.arguments("echo hello"), vec!["-c", "echo hello"]);
        assert_eq!(shell.alias().unwrap().1, "alias q='howdo'");
    }

    #[test]
    fn powershell_uses_selected_executable() {
        let shell = Shell::from_executable("C:\\Program Files\\PowerShell\\7\\pwsh.exe").unwrap();
        assert_eq!(shell.kind, Kind::PowerShell);
        assert!(shell.executable.to_string_lossy().ends_with("pwsh.exe"));
        assert!(shell
            .arguments("Write-Output hello")
            .contains(&"-NoProfile".into()));
    }

    #[test]
    fn powershell_encoding_preserves_code_and_terminates_comments() {
        let shell = Shell::from_executable("pwsh").unwrap();
        let command = "Write-Output 'café \"quoted\"' # trailing comment";
        let args = shell.arguments(command);
        assert!(args.contains(&"-EncodedCommand".into()));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(args.last().unwrap())
            .unwrap();
        let utf16: Vec<_> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let script = String::from_utf16(&utf16).unwrap();
        assert!(script.contains(&format!("\n{command}\n")));
    }

    #[test]
    fn cmd_disables_autorun() {
        let shell = Shell::from_executable("cmd.exe").unwrap();
        assert_eq!(
            shell.arguments("echo hello"),
            vec!["/D", "/S", "/C", "echo hello"]
        );
    }

    #[test]
    fn rejects_shell_commands_and_unknown_shells() {
        for shell in ["", "bash -c", "not-a-shell", "bash\n"] {
            assert!(Shell::from_executable(shell).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn propagates_child_failure_and_signal() {
        let shell = Shell::from_executable("/bin/sh").unwrap();
        assert_eq!(shell.run("exit 7").unwrap(), 7);
        assert_eq!(shell.run("kill -TERM $$").unwrap(), 143);
    }
}
