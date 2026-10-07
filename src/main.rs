mod cli;
mod clipboard;
mod config;
mod error;
mod http;
mod provider;
mod safety;
mod shell;
mod ui;
mod update;
mod wizard;

use cli::{Action, Cli, Output};
use error::{Error, Result};
use std::io::{self, Write};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(Error::Cancelled) => {
            let _ = writeln!(io::stderr(), "Cancelled.");
            130
        }
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(error) => {
            let _ = writeln!(io::stderr(), "howdo: {}", ui::safe_text(&error.to_string()));
            1
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() {
        writeln!(io::stderr(), "howdo v{VERSION}\n{}", cli::HELP)?;
        return Ok(2);
    }
    let cli = Cli::parse(args)?;
    match &cli.action {
        Action::Help => writeln!(io::stdout(), "howdo v{VERSION}\n{}", cli::HELP)?,
        Action::Version => writeln!(io::stdout(), "howdo {VERSION}")?,
        Action::Update => update::run(cli.yes)?,
        Action::Config => {
            let path = config::path();
            let store = config::load(&path)?;
            let profile = cli
                .profile
                .clone()
                .or_else(|| std::env::var("HOWDO_PROFILE").ok())
                .unwrap_or_else(|| {
                    store.map_or_else(|| "default".into(), |store| store.default_profile)
                });
            wizard::configure(&path, &profile)?;
        }
        Action::Profiles(selected) => {
            let path = config::path();
            if let Some(name) = selected {
                config::validate_profile_name(name)?;
                config::update(&path, |store| {
                    store.profile(Some(name))?;
                    store.default_profile = name.clone();
                    Ok(())
                })?;
                writeln!(io::stderr(), "Default profile: {name}")?;
            } else {
                let store =
                    config::load(&path)?.ok_or("No configuration found. Run howdo /config.")?;
                let mut stdout = io::stdout().lock();
                for (name, config) in store.profiles {
                    writeln!(
                        stdout,
                        "{} {name}: {}",
                        if name == store.default_profile {
                            "*"
                        } else {
                            " "
                        },
                        config.provider.name()
                    )?;
                }
            }
        }
        Action::Query(query) => return query_command(query, &cli),
    }
    Ok(0)
}

fn query_command(query: &str, cli: &Cli) -> Result<i32> {
    if query.len() > 64 * 1024 {
        return Err("Query exceeds the 64 KiB limit.".into());
    }
    let profile = cli
        .profile
        .clone()
        .or_else(|| std::env::var("HOWDO_PROFILE").ok());
    if let Some(profile) = &profile {
        config::validate_profile_name(profile)?;
    }
    let path = config::path();
    let store = match config::load(&path)? {
        Some(store) => store,
        None if ui::interactive() => {
            wizard::configure(&path, profile.as_deref().unwrap_or("default"))?;
            config::load(&path)?.ok_or("Configuration was not saved.")?
        }
        None => return Err("No configuration found. Run howdo /config in a terminal or create the documented JSON config.".into()),
    };
    let (profile_name, config) = store.profile(profile.as_deref())?;
    config.key()?;
    let shell = shell::Shell::detect(cli.shell.as_deref())?;
    let generation_config = config.clone();
    let generation_shell = shell.clone();
    let generation_query = query.to_owned();
    let mut command = ui::progress("Generating command", move || {
        provider::generate(&generation_query, &generation_config, &generation_shell)
    })?;
    let explanation = if cli.explain {
        let config = config.clone();
        let shell = shell.clone();
        let command = command.clone();
        Some(ui::progress("Explaining command", move || {
            provider::explain(&command, &config, &shell)
        })?)
    } else {
        None
    };
    if cli.output != Output::Json {
        if let Some(explanation) = &explanation {
            ui::explanation(explanation)?;
        }
    }
    match cli.output {
        Output::Print => {
            writeln!(io::stdout(), "{command}")?;
            return Ok(0);
        }
        Output::Json => {
            let output = serde_json::json!({
                "command": command,
                "shell": shell.executable.to_string_lossy(),
                "profile": profile_name,
                "risks": safety::risks(&command),
                "explanation": explanation.as_deref().map(ui::safe_text),
            });
            writeln!(io::stdout(), "{output}")?;
            return Ok(0);
        }
        Output::Copy => {
            clipboard::copy(&command)?;
            writeln!(
                io::stderr(),
                "Command copied to clipboard; nothing was executed."
            )?;
            return Ok(0);
        }
        Output::Interactive if !ui::interactive() && !cli.yes => {
            writeln!(io::stdout(), "{command}")?;
            return Ok(0);
        }
        Output::Interactive => {}
    }
    loop {
        let risks = safety::risks(&command);
        writeln!(io::stderr(), "\n  > {command}\n")?;
        if !risks.is_empty() {
            writeln!(
                io::stderr(),
                "Warning: {}. These checks are heuristics, not a safety guarantee.",
                risks.join("; ")
            )?;
        }
        if cli.yes {
            if !risks.is_empty() && !cli.allow_risky {
                return Err("Risky command was not executed. Review interactively, or explicitly supply --yes --allow-risky.".into());
            }
            break;
        }
        match ui::confirm_command(!risks.is_empty())? {
            ui::Decision::Run => break,
            ui::Decision::Cancel => {
                writeln!(io::stderr(), "Cancelled; nothing was executed.")?;
                return Ok(0);
            }
            ui::Decision::Edit => match ui::edit(&command)? {
                Some(edited) if !edited.trim().is_empty() => {
                    command = edited.trim_matches(' ').to_owned();
                    safety::validate_command(&command)?;
                    // Re-display and re-assess the final text; editing never implicitly executes.
                    if explanation.is_some() {
                        writeln!(
                            io::stderr(),
                            "The previous explanation does not apply to this edited command."
                        )?;
                    }
                }
                _ => return Ok(0),
            },
        }
    }
    let code = shell.run(&command)?;
    if code != 0 {
        writeln!(io::stderr(), "Command exited with status {code}.")?;
    }
    Ok(code)
}
