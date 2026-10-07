use crate::error::Result;
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
            "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" => (Kind::Posix, basename.to_owned()),
            _ => return Err("Unsupported shell. Choose sh, bash, zsh, fish, dash, ksh, powershell, pwsh, or cmd.".into()),
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
            Kind::PowerShell => vec![
                "-NoLogo".into(), "-NoProfile".into(), "-Command".into(),
                // PowerShell otherwise reports success after some native-command failures.
                format!("$ErrorActionPreference = 'Stop'; $global:LASTEXITCODE = 0\n{command}\n$__howdoSuccess = $?; if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}; if (-not $__howdoSuccess) {{ exit 1 }}"),
            ],
        }
    }

    pub fn run(&self, command: &str) -> Result<i32> {
        let status = Command::new(&self.executable)
            .args(self.arguments(command))
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
