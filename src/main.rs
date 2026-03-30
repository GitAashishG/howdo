use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

// ── OpenAI-compatible request/response ──────────────────────────────────

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

// ── Anthropic request/response ──────────────────────────────────────────

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
    temperature: f32,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

// ── Configuration ───────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    provider: String,
    base_url: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_version: Option<String>,
}

fn config_path() -> PathBuf {
    // XDG_CONFIG_HOME is respected on all platforms (also enables testable configs)
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("howdo").join("config.json");
    }
    if cfg!(windows) {
        let appdata = env::var("APPDATA").unwrap_or_else(|_| {
            let home = env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
            format!("{}\\AppData\\Roaming", home)
        });
        PathBuf::from(appdata).join("howdo").join("config.json")
    } else {
        let home = env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home)
            .join(".config")
            .join("howdo")
            .join("config.json")
    }
}

fn load_config() -> Option<Config> {
    let path = config_path();
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_config(config: &Config) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    fs::write(&path, &json).map_err(|e| format!("Failed to write config: {}", e))?;

    // Restrict file permissions to owner-only on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| format!("Failed to set config file permissions: {}", e))?;
    }
    Ok(())
}

/// Resolve the API key: env var takes precedence, then config file.
fn resolve_api_key(config: &Config) -> Option<String> {
    let env_key = match config.provider.as_str() {
        "openai" => env::var("OPENAI_API_KEY").ok(),
        "azure_openai" => env::var("AZURE_OPENAI_API_KEY").ok(),
        "anthropic" => env::var("ANTHROPIC_API_KEY").ok(),
        _ => env::var("OPENAI_API_KEY").ok(),
    };
    env_key.or_else(|| config.api_key.clone())
}

// ── Interactive setup ───────────────────────────────────────────────────

fn prompt_input(prompt: &str, default: &str) -> String {
    if default.is_empty() {
        print!("  {}: ", prompt);
    } else {
        print!("  {} [{}]: ", prompt, default);
    }
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim().to_string();
    if input.is_empty() {
        default.to_string()
    } else {
        input
    }
}

fn run_config_wizard() -> Config {
    println!("\n  \x1b[1;36m=== howdo Configuration ===\x1b[0m\n");
    println!("  Select your LLM provider:\n");
    println!("    1) Local LLM (Ollama, LM Studio, etc.)");
    println!("    2) OpenAI");
    println!("    3) Azure OpenAI");
    println!("    4) Anthropic");
    println!("    5) Other (OpenAI-compatible)");
    println!();

    let choice = prompt_input("Choice (1-5)", "1");
    println!();

    match choice.as_str() {
        "2" => {
            let api_key = prompt_input("API key", "");
            if api_key.is_empty() {
                eprintln!("  \x1b[33mWarning: API key is required for OpenAI.\x1b[0m");
            }
            let model = prompt_input("Model", "gpt-4o-mini");
            Config {
                provider: "openai".into(),
                base_url: "https://api.openai.com/v1".into(),
                model,
                api_key: if api_key.is_empty() { None } else { Some(api_key) },
                api_version: None,
            }
        }
        "3" => {
            println!("  Paste the full chat completions URL from the Azure portal.");
            println!("  \x1b[2mExample: https://myresource.cognitiveservices.azure.com/openai/deployments/gpt-4.1-mini/chat/completions?api-version=2024-12-01-preview\x1b[0m");
            println!();
            let base_url = prompt_input("Full URL", "");
            let api_key = prompt_input("API key", "");
            if api_key.is_empty() {
                eprintln!("  \x1b[33mWarning: API key is required for Azure OpenAI.\x1b[0m");
            }
            Config {
                provider: "azure_openai".into(),
                base_url,
                model: String::new(),
                api_key: if api_key.is_empty() { None } else { Some(api_key) },
                api_version: None,
            }
        }
        "4" => {
            let api_key = prompt_input("API key", "");
            if api_key.is_empty() {
                eprintln!("  \x1b[33mWarning: API key is required for Anthropic.\x1b[0m");
            }
            let base_url = prompt_input("Base URL", "https://api.anthropic.com");
            let model = prompt_input("Model", "claude-sonnet-4-20250514");
            Config {
                provider: "anthropic".into(),
                base_url,
                model,
                api_key: if api_key.is_empty() { None } else { Some(api_key) },
                api_version: None,
            }
        }
        "5" => {
            let base_url = prompt_input("Base URL (OpenAI-compatible)", "");
            let model = prompt_input("Model", "");
            let api_key = prompt_input("API key (leave empty for none)", "");
            Config {
                provider: "other".into(),
                base_url,
                model,
                api_key: if api_key.is_empty() { None } else { Some(api_key) },
                api_version: None,
            }
        }
        // Default: local LLM
        _ => {
            let base_url = prompt_input("Base URL", "http://127.0.0.1:1234/v1");
            let model = prompt_input("Model name", "default");
            Config {
                provider: "local".into(),
                base_url,
                model,
                api_key: None,
                api_version: None,
            }
        }
    }
}

fn display_config(config: &Config) {
    let provider_name = match config.provider.as_str() {
        "local" => "Local LLM",
        "openai" => "OpenAI",
        "azure_openai" => "Azure OpenAI",
        "anthropic" => "Anthropic",
        "other" => "Other (OpenAI-compatible)",
        p => p,
    };
    println!("  Provider:  {}", provider_name);
    println!("  Endpoint:  {}", config.base_url);
    println!("  Model:     {}", config.model);
    if let Some(key) = &config.api_key {
        println!("  API key:   {}", mask_key(key));
    }
    if let Some(ver) = &config.api_version {
        println!("  API ver:   {}", ver);
    }
}

fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        "****".to_string()
    } else {
        let prefix: String = chars[..3].iter().collect();
        let suffix: String = chars[chars.len() - 4..].iter().collect();
        format!("{}...{}", prefix, suffix)
    }
}

// ── OS / shell detection ────────────────────────────────────────────────

fn detect_shell() -> String {
    if cfg!(windows) {
        // Check common indicators for PowerShell vs cmd
        if env::var("PSModulePath").is_ok() {
            // Check if it's pwsh (PowerShell 7+) or Windows PowerShell
            if let Ok(exe) = env::var("SHELL") {
                if exe.contains("pwsh") {
                    return "PowerShell 7 (pwsh)".to_string();
                }
            }
            return "Windows PowerShell".to_string();
        }
        "cmd.exe".to_string()
    } else {
        env::var("SHELL")
            .unwrap_or_else(|_| String::from("/bin/bash"))
            .rsplit('/')
            .next()
            .unwrap_or("bash")
            .to_string()
    }
}

fn detect_os() -> String {
    let os = env::consts::OS;
    if os == "macos" {
        return "macOS (Darwin/BSD)".to_string();
    }
    if os == "linux" {
        if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
            for line in contents.lines() {
                if line.starts_with("PRETTY_NAME=") {
                    return format!(
                        "Linux ({})",
                        line.trim_start_matches("PRETTY_NAME=")
                            .trim_matches('"')
                    );
                }
            }
        }
        return "Linux".to_string();
    }
    if os == "windows" {
        return "Windows".to_string();
    }
    os.to_string()
}

fn build_prompt(query: &str, os: &str, shell: &str) -> String {
    let examples = if os.contains("macOS") {
        "Examples for this OS:\n\
         Q: kill process on port 3000\nA: kill -9 $(lsof -ti :3000)\n\
         Q: find large files\nA: find . -type f -size +100M\n\
         Q: copy output to clipboard\nA: echo hello | pbcopy"
    } else if os.contains("Windows") {
        "Examples for this OS:\n\
         Q: kill process on port 3000\nA: Stop-Process -Id (Get-NetTCPConnection -LocalPort 3000).OwningProcess -Force\n\
         Q: find large files\nA: Get-ChildItem -Recurse | Where-Object {$_.Length -gt 100MB}\n\
         Q: list all services\nA: Get-Service | Format-Table Name, Status"
    } else {
        "Examples for this OS:\n\
         Q: kill process on port 3000\nA: fuser -k 3000/tcp\n\
         Q: find large files\nA: find . -type f -size +100M\n\
         Q: copy output to clipboard\nA: echo hello | xclip -selection clipboard"
    };

    format!(
        "You are a command-line assistant. Convert the user's natural language request into a single shell command.\n\n\
         Rules:\n\
         - Output ONLY the command. No explanations, no markdown, no backticks.\n\
         - One line only. Chain multiple steps with ; or | if needed.\n\
         - Prefer simple, common commands over clever tricks.\n\n\
         OS: {os}\n\
         Shell: {shell}\n\
         Working directory: {cwd}\n\n\
         {examples}\n\n\
         Q: {query}\nA:",
        os = os,
        shell = shell,
        cwd = env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".into()),
        examples = examples,
        query = query,
    )
}

// ── LLM calling ─────────────────────────────────────────────────────────

fn call_llm(query: &str, config: &Config) -> Result<String, String> {
    let os = detect_os();
    let shell = detect_shell();
    let system_prompt = build_prompt(query, &os, &shell);

    match config.provider.as_str() {
        "anthropic" => call_anthropic(query, config, &system_prompt),
        _ => call_openai_compatible(query, config, &system_prompt),
    }
}

fn call_openai_compatible(
    query: &str,
    config: &Config,
    system_prompt: &str,
) -> Result<String, String> {
    let request_body = ChatRequest {
        model: config.model.clone(),
        messages: vec![
            Message {
                role: "system".into(),
                content: system_prompt.to_string(),
            },
            Message {
                role: "user".into(),
                content: query.to_string(),
            },
        ],
        temperature: 0.0,
        max_tokens: 256,
    };

    let url = if config.provider == "azure_openai" {
        // Azure: base_url is the complete URL, use as-is
        config.base_url.clone()
    } else {
        format!(
            "{}/chat/completions",
            config.base_url.trim_end_matches('/')
        )
    };

    let body_json = serde_json::to_string(&request_body).unwrap();

    let mut req = minreq::post(&url)
        .with_header("Content-Type", "application/json")
        .with_timeout(30)
        .with_body(body_json);

    if let Some(key) = resolve_api_key(config) {
        if config.provider == "azure_openai" {
            req = req.with_header("api-key", key.as_str());
        } else {
            req = req.with_header("Authorization", format!("Bearer {}", key));
        }
    }

    let resp = req
        .send()
        .map_err(|e| format!("API request failed (is the endpoint reachable?): {}", e))?;

    if resp.status_code != 200 {
        return Err(format!(
            "API returned status {}: {}",
            resp.status_code,
            resp.as_str().unwrap_or("unknown error")
        ));
    }

    let body: ChatResponse = serde_json::from_str(resp.as_str().unwrap_or(""))
        .map_err(|e| format!("Failed to parse API response: {}", e))?;

    body.choices
        .first()
        .map(|c| strip_markdown(&c.message.content))
        .filter(|c| !c.is_empty())
        .ok_or_else(|| "Model returned an empty command".into())
}

fn call_anthropic(
    query: &str,
    config: &Config,
    system_prompt: &str,
) -> Result<String, String> {
    let request_body = AnthropicRequest {
        model: config.model.clone(),
        max_tokens: 256,
        system: system_prompt.to_string(),
        messages: vec![Message {
            role: "user".into(),
            content: query.to_string(),
        }],
        temperature: 0.0,
    };

    let url = format!(
        "{}/v1/messages",
        config.base_url.trim_end_matches('/')
    );

    let body_json = serde_json::to_string(&request_body).unwrap();

    let mut req = minreq::post(&url)
        .with_header("Content-Type", "application/json")
        .with_header("anthropic-version", "2023-06-01")
        .with_timeout(30)
        .with_body(body_json);

    if let Some(key) = resolve_api_key(config) {
        req = req.with_header("x-api-key", key.as_str());
    }

    let resp = req
        .send()
        .map_err(|e| format!("API request failed (is the endpoint reachable?): {}", e))?;

    if resp.status_code != 200 {
        return Err(format!(
            "API returned status {}: {}",
            resp.status_code,
            resp.as_str().unwrap_or("unknown error")
        ));
    }

    let body: AnthropicResponse = serde_json::from_str(resp.as_str().unwrap_or(""))
        .map_err(|e| format!("Failed to parse API response: {}", e))?;

    body.content
        .first()
        .map(|c| strip_markdown(&c.text))
        .filter(|c| !c.is_empty())
        .ok_or_else(|| "Model returned an empty response".into())
}

// ── Utilities ───────────────────────────────────────────────────────────

/// Strip markdown code fences and LLM artifacts from the response
fn strip_markdown(s: &str) -> String {
    let s = s.trim();
    // Handle ```bash\n...\n``` or ```\n...\n```
    let s = if s.starts_with("```") && s.ends_with("```") {
        let inner = &s[3..s.len() - 3];
        // Skip the language tag on the first line (e.g. "bash", "sh", "zsh")
        let inner = if let Some(newline_pos) = inner.find('\n') {
            &inner[newline_pos + 1..]
        } else {
            inner
        };
        inner.trim()
    } else if s.starts_with('`') && s.ends_with('`') && !s.contains('\n') {
        // Handle single backtick wrapping: `command`
        s[1..s.len() - 1].trim()
    } else {
        s
    };

    // Remove common LLM end-of-sequence / chat tokens
    let s = s
        .replace("</s>", "")
        .replace("<|im_end|>", "")
        .replace("<|endoftext|>", "")
        .replace("<|eot_id|>", "");

    // Strip comment lines (# ...) that some models prepend
    let lines: Vec<&str> = s
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    // If multiple lines remain, join with ; (works in bash, zsh, powershell, cmd)
    if lines.len() > 1 {
        lines.join(" ; ")
    } else {
        lines.first().unwrap_or(&"").to_string()
    }
}

fn run_command(cmd: &str) {
    let status = if cfg!(windows) {
        let shell = detect_shell();
        if shell.contains("PowerShell") || shell.contains("pwsh") {
            Command::new("powershell")
                .arg("-NoProfile")
                .arg("-Command")
                .arg(cmd)
                .status()
        } else {
            Command::new("cmd")
                .arg("/C")
                .arg(cmd)
                .status()
        }
    } else {
        let shell = detect_shell();
        Command::new(&shell)
            .arg("-c")
            .arg(cmd)
            .status()
    };

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("\x1b[33mCommand exited with status: {}\x1b[0m", s);
        }
        Err(e) => {
            eprintln!("\x1b[31mFailed to execute command: {}\x1b[0m", e);
        }
    }
}

// ── Main ────────────────────────────────────────────────────────────────

fn save_and_display(config: &Config) {
    match save_config(config) {
        Ok(()) => {
            println!(
                "\n  \x1b[1;32m✓\x1b[0m Configuration saved to {}\n",
                config_path().display()
            );
            display_config(config);
            println!();
        }
        Err(e) => {
            eprintln!("\x1b[31m{}\x1b[0m", e);
            std::process::exit(1);
        }
    }
}

fn offer_shell_alias() {
    let shell = detect_shell();
    let rc_file = match shell.as_str() {
        "zsh" => "~/.zshrc",
        "bash" => "~/.bashrc",
        _ => return, // only offer for zsh/bash
    };

    println!("  \x1b[1;36mShell alias\x1b[0m (recommended):");
    println!("  Create a short alias `q` so you can type `q whats on port 8000?`");
    println!("  The `noglob` prefix prevents shell from expanding ? ! * etc.");
    println!();
    print!("  Add alias to {}? \x1b[2m(Y/n)\x1b[0m ", rc_file);
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim().to_lowercase();

    if input.is_empty() || input == "y" || input == "yes" {
        let rc_path = rc_file.replace("~", &env::var("HOME").unwrap_or_else(|_| ".".into()));
        let alias_line = "\nalias q='noglob howdo'\n";

        // Check if alias already exists
        if let Ok(contents) = fs::read_to_string(&rc_path) {
            if contents.contains("alias q=") {
                println!("  \x1b[2mAlias already exists in {}\x1b[0m\n", rc_file);
                return;
            }
        }

        match fs::OpenOptions::new().append(true).open(&rc_path) {
            Ok(mut f) => {
                use std::io::Write;
                if f.write_all(alias_line.as_bytes()).is_ok() {
                    println!("  \x1b[1;32m✓\x1b[0m Added to {}. Run `source {}` or open a new terminal.\n", rc_file, rc_file);
                } else {
                    eprintln!("  \x1b[33mFailed to write to {}. Add manually:\x1b[0m", rc_file);
                    eprintln!("  alias q='noglob howdo'\n");
                }
            }
            Err(_) => {
                eprintln!("  \x1b[33mCould not open {}. Add manually:\x1b[0m", rc_file);
                eprintln!("  alias q='noglob howdo'\n");
            }
        }
    } else {
        println!("  \x1b[2mSkipped. You can add it later:\x1b[0m");
        println!("  alias q='noglob howdo'  # add to {}\n", rc_file);
    }
}

// ── Self-update ─────────────────────────────────────────────────────────

const REPO: &str = "GitAashishG/howdo";

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

fn get_target_triple() -> &'static str {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "x86_64-apple-darwin"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "x86_64-pc-windows-msvc"
    } else {
        "unknown"
    }
}

fn self_update() {
    println!("\n  Checking for updates...");

    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    let resp = minreq::get(&url)
        .with_header("User-Agent", "howdo-updater")
        .with_timeout(10)
        .send();

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  \x1b[31mFailed to check for updates: {}\x1b[0m", e);
            return;
        }
    };

    if resp.status_code != 200 {
        eprintln!("  \x1b[31mGitHub API returned status {}\x1b[0m", resp.status_code);
        return;
    }

    let release: GithubRelease = match serde_json::from_str(resp.as_str().unwrap_or("")) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  \x1b[31mFailed to parse release info: {}\x1b[0m", e);
            return;
        }
    };

    let latest = release.tag_name.trim_start_matches('v');
    if latest == VERSION {
        println!("  \x1b[1;32m✓\x1b[0m Already on the latest version ({})\n", VERSION);
        return;
    }

    println!("  Current: v{}  →  Latest: v{}", VERSION, latest);

    let target = get_target_triple();
    let asset = release.assets.iter().find(|a| a.name.contains(target));

    let asset = match asset {
        Some(a) => a,
        None => {
            eprintln!("  \x1b[31mNo binary found for {} in release {}\x1b[0m", target, release.tag_name);
            eprintln!("  Download manually from: https://github.com/{}/releases\n", REPO);
            return;
        }
    };

    print!("  Download and install? \x1b[2m(Y/n)\x1b[0m ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim().to_lowercase();

    if !(input.is_empty() || input == "y" || input == "yes") {
        println!("  Cancelled.\n");
        return;
    }

    print!("  Downloading {}... ", asset.name);
    io::stdout().flush().unwrap();

    let download = minreq::get(&asset.browser_download_url)
        .with_header("User-Agent", "howdo-updater")
        .with_timeout(60)
        .send();

    let download = match download {
        Ok(r) if r.status_code == 200 => r,
        Ok(r) => {
            eprintln!("\x1b[31mfailed (status {})\x1b[0m", r.status_code);
            return;
        }
        Err(e) => {
            eprintln!("\x1b[31mfailed: {}\x1b[0m", e);
            return;
        }
    };

    println!("\x1b[1;32mdone\x1b[0m");

    // Find where the current binary lives
    let current_exe = match env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("  \x1b[31mCould not determine binary path: {}\x1b[0m", e);
            return;
        }
    };

    let tmp_path = current_exe.with_extension("tmp");
    let old_path = current_exe.with_extension("old");

    // Write new binary to temp file
    if let Err(e) = fs::write(&tmp_path, download.as_bytes()) {
        eprintln!("  \x1b[31mFailed to write update: {}\x1b[0m", e);
        return;
    }

    // Make executable on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o755));
    }

    // Swap: current -> old, tmp -> current
    if fs::rename(&current_exe, &old_path).is_err() {
        eprintln!("  \x1b[31mFailed to replace binary (permission denied?). Try with sudo.\x1b[0m");
        let _ = fs::remove_file(&tmp_path);
        return;
    }

    if fs::rename(&tmp_path, &current_exe).is_err() {
        // Try to restore
        let _ = fs::rename(&old_path, &current_exe);
        eprintln!("  \x1b[31mFailed to install new binary.\x1b[0m");
        return;
    }

    // Clean up old binary
    let _ = fs::remove_file(&old_path);

    println!("  \x1b[1;32m✓\x1b[0m Updated to v{}!\n", latest);
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!("howdo v{} — natural language to terminal commands", VERSION);
    println!();
    println!("Usage: howdo <what you want to do in plain english>");
    println!("       howdo /config     Configure LLM provider");
    println!("       howdo /update     Update to latest release");
    println!("       howdo --help      Show this help");
    println!("       howdo --version   Show version");
    println!();
    println!("Quick alias (recommended):");
    println!("  alias q='noglob howdo'   # add to ~/.zshrc");
    println!("  Then just: q list files sorted by size");
    println!();
    println!("Examples:");
    println!("  howdo list files in descending order of size");
    println!("  howdo find all python files modified in the last week");
    println!("  howdo compress this folder into a tar.gz");
}

fn test_connection(config: &Config) {
    print!("  Testing connection... ");
    io::stdout().flush().unwrap();

    let url = match config.provider.as_str() {
        // Azure: base_url is already the complete URL
        "azure_openai" => config.base_url.clone(),
        "anthropic" => format!(
            "{}/v1/messages",
            config.base_url.trim_end_matches('/')
        ),
        _ => format!(
            "{}/chat/completions",
            config.base_url.trim_end_matches('/')
        ),
    };

    // Just try to reach the endpoint (a GET or empty POST — we only care about connectivity)
    let result = minreq::get(&url)
        .with_timeout(5)
        .send();

    match result {
        Ok(resp) if resp.status_code == 405 || resp.status_code == 200 || resp.status_code == 401 || resp.status_code == 422 => {
            println!("\x1b[1;32mreachable\x1b[0m");
        }
        Ok(resp) => {
            println!("\x1b[1;32mreachable\x1b[0m (status {})", resp.status_code);
        }
        Err(e) => {
            println!("\x1b[1;31mfailed\x1b[0m");
            eprintln!("  \x1b[33mCould not reach endpoint: {}\x1b[0m", e);
            eprintln!("  \x1b[33mConfig saved, but check your URL/network.\x1b[0m");
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    // Handle flags
    if let Some(first) = args.first() {
        match first.as_str() {
            "--version" | "-V" => {
                println!("howdo {}", VERSION);
                return;
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            "/config" => {
                let config = run_config_wizard();
                save_and_display(&config);
                test_connection(&config);
                offer_shell_alias();
                return;
            }
            "/update" => {
                self_update();
                return;
            }
            _ => {}
        }
    }

    if args.is_empty() {
        print_help();
        std::process::exit(1);
    }

    // Load config or run first-time setup
    let config = match load_config() {
        Some(c) => c,
        None => {
            eprintln!("  \x1b[1;33mNo configuration found. Let's set up howdo.\x1b[0m");
            let config = run_config_wizard();
            save_and_display(&config);
            test_connection(&config);
            offer_shell_alias();
            config
        }
    };

    let query = args.join(" ");

    // Call LLM
    let command = match call_llm(&query, &config) {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("\x1b[31m{}\x1b[0m", e);
            std::process::exit(1);
        }
    };

    // Display the command
    println!("\n  \x1b[1;36m❯\x1b[0m \x1b[1m{}\x1b[0m\n", command);

    // Warn on potentially dangerous commands
    let lower = command.to_lowercase();
    if lower.contains("rm -rf")
        || lower.contains("mkfs")
        || lower.contains("dd if=")
        || lower.contains("> /dev/")
        || lower.contains(":(){ :|:& };:")
        || (lower.contains("chmod") && lower.contains("-R") && lower.contains("777"))
        || lower.contains("format c:")
    {
        eprintln!("  \x1b[1;31m⚠ This command looks destructive. Double-check before running.\x1b[0m\n");
    }

    // Ask to run (single keypress)
    print!("  Run? \x1b[2m(y/e/n)\x1b[0m ");
    io::stdout().flush().unwrap();

    let action = read_single_key();

    match action {
        'y' | '\n' | '\r' => {
            println!("\n");
            run_command(&command);
        }
        'e' => {
            println!("\n");
            if let Some(edited) = edit_command(&command) {
                let edited = edited.trim().to_string();
                if !edited.is_empty() {
                    println!();
                    run_command(&edited);
                }
            }
        }
        _ => {
            println!("\n");
        }
    }
}

fn read_single_key() -> char {
    use crossterm::terminal;

    // Fall back to line-based input if stdin is not a terminal (e.g. piped in tests)
    if !crossterm::tty::IsTty::is_tty(&io::stdin()) {
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap_or(0);
        return match input.trim().to_lowercase().as_str() {
            "y" | "" => 'y',
            "e" => 'e',
            _ => 'n',
        };
    }

    use crossterm::event::{self, Event, KeyCode, KeyEvent};
    use std::time::Duration;

    terminal::enable_raw_mode().unwrap();

    // Drain any buffered input (e.g. leftover Enter from launching the command)
    while event::poll(Duration::from_millis(50)).unwrap_or(false) {
        let _ = event::read();
    }

    let result = loop {
        if let Ok(Event::Key(KeyEvent { code, .. })) = event::read() {
            break match code {
                KeyCode::Char(c) => c,
                KeyCode::Enter => '\n',
                KeyCode::Esc => 'n',
                _ => continue,
            };
        }
    };
    terminal::disable_raw_mode().unwrap();
    result
}

fn edit_command(cmd: &str) -> Option<String> {
    let mut rl = rustyline::DefaultEditor::new().ok()?;
    match rl.readline_with_initial("  ❯ ", (cmd, "")) {
        Ok(line) => Some(line),
        Err(_) => None,
    }
}
