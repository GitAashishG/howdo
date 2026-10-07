use crate::config::{self, Config, Provider};
use crate::error::Result;
use crate::{provider, shell::Shell, ui};
use std::io::{self, Write};
use std::path::Path;

pub fn configure(path: &Path, profile: &str) -> Result<()> {
    if !ui::interactive() {
        return Err("Configuration requires an interactive terminal. See the README for the JSON schema and environment variables.".into());
    }
    config::validate_profile_name(profile)?;
    let existing = config::load(path)?.and_then(|store| store.profiles.get(profile).cloned());
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "\n=== howdo configuration: {profile} ===")?;
    writeln!(stderr, "1) Local LLM (LM Studio, Ollama, etc.)\n2) OpenAI\n3) Azure OpenAI\n4) Anthropic\n5) Other (OpenAI-compatible)")?;
    drop(stderr);
    let default = existing
        .as_ref()
        .map_or("1", |config| match config.provider {
            Provider::Local => "1",
            Provider::Openai => "2",
            Provider::AzureOpenai => "3",
            Provider::Anthropic => "4",
            Provider::Other => "5",
        });
    let provider = match ui::input("Provider (1-5)", default)?.as_str() {
        "1" => Provider::Local,
        "2" => Provider::Openai,
        "3" => Provider::AzureOpenai,
        "4" => Provider::Anthropic,
        "5" => Provider::Other,
        _ => return Err("Choose a provider from 1 to 5; no configuration was saved.".into()),
    };
    let previous = existing.filter(|config| config.provider == provider);
    let (base, model) = match provider {
        Provider::Local => ("http://127.0.0.1:1234/v1", "default"),
        Provider::Openai => ("https://api.openai.com/v1", "gpt-4.1-mini"),
        Provider::AzureOpenai => ("", ""),
        Provider::Anthropic => ("https://api.anthropic.com", ""),
        Provider::Other => ("", ""),
    };
    if provider == Provider::Local {
        writeln!(io::stderr(), "LM Studio: http://127.0.0.1:1234/v1\nOllama: http://127.0.0.1:11434/v1 (enter an installed model name)")?;
    }
    if provider == Provider::AzureOpenai {
        writeln!(
            io::stderr(),
            "Paste the full chat/completions URL, including api-version if required."
        )?;
    }
    let base_url = ui::input(
        "Endpoint URL",
        previous.as_ref().map_or(base, |config| &config.base_url),
    )?;
    let model = if provider == Provider::AzureOpenai {
        ui::input(
            "Model (leave empty for a deployment URL)",
            previous.as_ref().map_or("", |config| &config.model),
        )?
    } else {
        ui::input(
            "Model",
            previous.as_ref().map_or(model, |config| &config.model),
        )?
    };
    if provider == Provider::Local
        && base_url.contains(":11434")
        && (model.is_empty() || model == "default")
    {
        return Err("Ollama requires an installed model name. Run `ollama list` to find one; no config was saved.".into());
    }
    let mut config =
        previous.unwrap_or_else(|| Config::new(provider, base_url.clone(), model.clone()));
    config.base_url = base_url;
    config.model = model;
    let default_env = match provider {
        Provider::Openai => "OPENAI_API_KEY",
        Provider::AzureOpenai => "AZURE_OPENAI_API_KEY",
        Provider::Anthropic => "ANTHROPIC_API_KEY",
        Provider::Local | Provider::Other => "none (no implicit credentials)",
    };
    writeln!(
        io::stderr(),
        "Provider credential variable: {default_env}. Environment credentials take precedence."
    )?;
    let key_env = ui::input(
        "Explicit credential environment variable (optional)",
        config.api_key_env.as_deref().unwrap_or(""),
    )?;
    config.api_key_env = (!key_env.is_empty()).then_some(key_env);
    if config.api_key_env.is_none() {
        let prompt = if config.api_key.is_some() {
            "API key (hidden; empty keeps the saved key)"
        } else {
            "API key (hidden; empty uses the provider environment variable or no auth)"
        };
        if let Some(key) = ui::secret(prompt)? {
            config.api_key = Some(key);
        }
    }
    config.validate()?;
    config.key()?;
    if ui::confirm("Test authenticated inference before saving? Makes one API call; never executes the result (y/N)")? {
        let test_config = config.clone();
        let shell = Shell::detect(None)?;
        let command = ui::progress("Testing inference", move || provider::generate("Print the text howdo-connection-ok without changing any files", &test_config, &shell))?;
        writeln!(io::stderr(), "Inference succeeded (not executed): {command}")?;
    }
    config::update(path, |store| {
        if store.profiles.is_empty() {
            store.default_profile = profile.to_owned();
        }
        store.profiles.insert(profile.to_owned(), config);
        Ok(())
    })?;
    writeln!(
        io::stderr(),
        "Saved profile {profile:?} to {}. API keys are never displayed.",
        ui::safe_text(&path.display().to_string())
    )?;
    if let Some((rc_file, alias)) = Shell::detect(None)?.alias() {
        writeln!(io::stderr(), "Optional alias for ~/{rc_file}: {alias}\nQuote queries containing shell metacharacters. Startup files were not changed.")?;
    }
    Ok(())
}
