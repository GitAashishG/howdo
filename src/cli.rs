use crate::error::Result;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Output {
    #[default]
    Interactive,
    Print,
    Json,
    Copy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Query(String),
    Config,
    Update,
    Profiles(Option<String>),
    Help,
    Version,
}

#[derive(Debug)]
pub struct Cli {
    pub action: Action,
    pub profile: Option<String>,
    pub shell: Option<String>,
    pub output: Output,
    pub explain: bool,
    pub yes: bool,
    pub allow_risky: bool,
}

impl Cli {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut args = args.into_iter();
        let mut cli = Self {
            action: Action::Help,
            profile: None,
            shell: None,
            output: Output::Interactive,
            explain: false,
            yes: false,
            allow_risky: false,
        };
        let mut words = Vec::new();
        let mut admin = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--" => {
                    words.extend(args);
                    break;
                }
                "--help" | "-h" => return Ok(cli.with_action(Action::Help)),
                "--version" | "-V" => return Ok(cli.with_action(Action::Version)),
                "--profile" => cli.profile = Some(value(&mut args, "--profile")?),
                "--shell" => cli.shell = Some(value(&mut args, "--shell")?),
                "--print" => cli.set_output(Output::Print)?,
                "--json" => cli.set_output(Output::Json)?,
                "--copy" => cli.set_output(Output::Copy)?,
                "--explain" => cli.explain = true,
                "--yes" | "-y" => cli.yes = true,
                "--allow-risky" => cli.allow_risky = true,
                "/config" | "/update" | "/profiles" if admin.is_none() && words.is_empty() => {
                    admin = Some(arg);
                }
                _ if arg.starts_with('-') => {
                    return Err(format!(
                        "Unknown option {arg:?}. Use -- before a query starting with '-'."
                    )
                    .into());
                }
                _ => {
                    words.push(arg);
                    if admin.is_none() {
                        // Once a query starts, its flags and punctuation belong to the user.
                        words.extend(args);
                        break;
                    }
                }
            }
        }
        if cli.allow_risky && !cli.yes {
            return Err("--allow-risky requires --yes.".into());
        }
        if cli.yes && cli.output != Output::Interactive {
            return Err("--yes cannot be combined with --print, --json, or --copy.".into());
        }
        cli.action = match admin.as_deref() {
            Some("/config") if words.is_empty() => Action::Config,
            Some("/update") if words.is_empty() => Action::Update,
            Some("/profiles") if words.is_empty() => Action::Profiles(None),
            Some("/profiles") if words.len() == 2 && words[0] == "use" => {
                Action::Profiles(Some(words[1].clone()))
            }
            Some(_) => return Err("Invalid subcommand arguments. See howdo --help.".into()),
            None if !words.is_empty() && !words.join(" ").trim().is_empty() => {
                Action::Query(words.join(" "))
            }
            None => return Err("A query is required. See howdo --help.".into()),
        };
        if !matches!(cli.action, Action::Query(_))
            && (cli.output != Output::Interactive
                || cli.explain
                || cli.shell.is_some()
                || cli.allow_risky
                || (cli.yes && !matches!(cli.action, Action::Update)))
        {
            return Err("Generation and execution options only apply to a query (--yes also supports /update).".into());
        }
        if matches!(cli.action, Action::Update | Action::Profiles(_)) && cli.profile.is_some() {
            return Err("--profile only applies to a query or /config.".into());
        }
        Ok(cli)
    }

    fn with_action(mut self, action: Action) -> Self {
        self.action = action;
        self
    }

    fn set_output(&mut self, output: Output) -> Result<()> {
        if self.output != Output::Interactive {
            return Err("Choose only one of --print, --json, and --copy.".into());
        }
        self.output = output;
        Ok(())
    }
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    match args.next() {
        Some(value) if !value.is_empty() && !value.starts_with('-') => Ok(value),
        _ => Err(format!("{flag} requires a value.").into()),
    }
}

pub const HELP: &str = "Natural language to terminal commands.

Usage: howdo [options] <query ...>
       howdo /config [--profile NAME]
       howdo /profiles [use NAME]
       howdo /update [--yes]

Options (place before the query):
  --profile NAME   Use a named provider profile (or HOWDO_PROFILE)
  --shell PATH     Shell executable (or HOWDO_SHELL)
  --print          Print only the command; never execute
  --json           Print structured JSON; never execute
  --copy           Copy the command to the clipboard; never execute
  --explain        Ask for an explanation before review (one extra API call)
  --yes, -y        Explicitly execute without interactive confirmation
  --allow-risky    Also allow risky commands with --yes
  --               Treat subsequent arguments as the query
  --help, -h       Show this help
  --version, -V    Show version

Noninteractive input defaults to print-only, even if 'y' is piped in.
Interactive execution requires an explicit y; Enter and EOF cancel.
Risk warnings are heuristics, not a sandbox or a security guarantee.

Zsh: alias q='noglob howdo'
Bash: alias q='howdo'  (quote queries containing shell metacharacters)
";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli> {
        Cli::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn query_options_remain_literal() {
        assert_eq!(
            parse(&["find", "--help"]).unwrap().action,
            Action::Query("find --help".into())
        );
        assert_eq!(
            parse(&["--", "--help"]).unwrap().action,
            Action::Query("--help".into())
        );
    }

    #[test]
    fn profiles_and_config() {
        let cli = parse(&["/config", "--profile", "work"]).unwrap();
        assert_eq!(cli.action, Action::Config);
        assert_eq!(cli.profile.as_deref(), Some("work"));
        assert_eq!(
            parse(&["/profiles", "use", "work"]).unwrap().action,
            Action::Profiles(Some("work".into()))
        );
    }

    #[test]
    fn rejects_conflicting_or_missing_options() {
        for args in [
            vec![],
            vec!["--shell"],
            vec!["--profile", "--print"],
            vec!["--yes", "--print", "test"],
            vec!["--copy", "--json", "test"],
            vec!["--allow-risky", "test"],
            vec!["--typo", "test"],
            vec!["/config", "--yes"],
            vec!["/update", "--profile", "work"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
