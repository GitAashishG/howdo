use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use url::Url;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Local,
    Openai,
    AzureOpenai,
    Anthropic,
    Other,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Local => "Local LLM",
            Self::Openai => "OpenAI",
            Self::AzureOpenai => "Azure OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Other => "Other (OpenAI-compatible)",
        }
    }

    fn key_env(self) -> Option<&'static str> {
        match self {
            Self::Openai => Some("OPENAI_API_KEY"),
            Self::AzureOpenai => Some("AZURE_OPENAI_API_KEY"),
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            Self::Local | Self::Other => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenLimit {
    #[default]
    Auto,
    MaxTokens,
    MaxCompletionTokens,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemRole {
    #[default]
    Auto,
    System,
    Developer,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RequestOptions {
    pub token_limit: TokenLimit,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    pub system_role: SystemRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            token_limit: TokenLimit::Auto,
            max_tokens: 1024,
            temperature: None,
            system_role: SystemRole::Auto,
            reasoning_effort: None,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub provider: Provider,
    pub base_url: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    // Accepted for legacy configs. Azure's full URL already includes its API version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(default)]
    pub request_options: RequestOptions,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub allow_insecure_http: bool,
}

fn default_timeout() -> u64 {
    30
}

impl Config {
    pub fn new(provider: Provider, base_url: String, model: String) -> Self {
        Self {
            provider,
            base_url,
            model,
            api_key: None,
            api_key_env: None,
            api_version: None,
            request_options: RequestOptions::default(),
            timeout_seconds: default_timeout(),
            allow_insecure_http: false,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let url = self.url()?;
        if !matches!(self.provider, Provider::Local | Provider::AzureOpenai)
            && (self.model.trim().is_empty() || self.model == "default")
        {
            return Err("This provider requires an explicit model name.".into());
        }
        if self.provider == Provider::AzureOpenai
            && self.model.trim().is_empty()
            && !url.path().contains("/deployments/")
        {
            return Err(
                "Azure endpoints without a deployment in the URL require an explicit model name."
                    .into(),
            );
        }
        if self.model.len() > 256 || self.model.chars().any(|c| c.is_control()) {
            return Err("Invalid model name.".into());
        }
        if !(1..=300).contains(&self.timeout_seconds) {
            return Err("timeout_seconds must be between 1 and 300.".into());
        }
        let options = &self.request_options;
        if options.max_tokens == 0 || options.max_tokens > 32768 {
            return Err("request_options.max_tokens must be between 1 and 32768.".into());
        }
        if options
            .temperature
            .is_some_and(|v| !v.is_finite() || !(0.0..=2.0).contains(&v))
        {
            return Err("temperature must be between 0 and 2, or null to omit it.".into());
        }
        if options
            .reasoning_effort
            .as_deref()
            .is_some_and(|v| !["none", "minimal", "low", "medium", "high", "xhigh"].contains(&v))
        {
            return Err("Unsupported reasoning_effort value.".into());
        }
        if self.provider == Provider::Anthropic
            && (options.token_limit == TokenLimit::MaxCompletionTokens
                || options.system_role == SystemRole::Developer
                || options.reasoning_effort.is_some()
                || options.temperature.is_some_and(|v| v > 1.0))
        {
            return Err("Anthropic does not support these OpenAI request options (temperature must also be <= 1).".into());
        }
        if let Some(name) = &self.api_key_env {
            if name.is_empty()
                || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                || name.as_bytes()[0].is_ascii_digit()
            {
                return Err("api_key_env must name an environment variable.".into());
            }
        }
        if let Some(key) = &self.api_key {
            validate_key(key)?;
        }
        Ok(())
    }

    pub fn url(&self) -> Result<Url> {
        if self
            .base_url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err("Endpoint URL must not contain whitespace or control characters.".into());
        }
        let url =
            Url::parse(&self.base_url).map_err(|_| "Endpoint must be an absolute HTTP(S) URL.")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err("Endpoint must be an absolute HTTP(S) URL.".into());
        }
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err("Endpoint URLs must not contain credentials or fragments.".into());
        }
        if self.provider != Provider::AzureOpenai && url.query().is_some() {
            return Err("Only Azure full endpoint URLs may include query parameters.".into());
        }
        if url.query_pairs().any(|(name, _)| {
            ["api-key", "api_key", "key", "token", "access_token"]
                .contains(&name.to_lowercase().as_str())
        }) {
            return Err(
                "Put credentials in api_key or api_key_env, not URL query parameters.".into(),
            );
        }
        if self.provider == Provider::AzureOpenai
            && !url
                .path()
                .trim_end_matches('/')
                .ends_with("/chat/completions")
        {
            return Err("Azure requires the full /chat/completions URL, including its api-version query if needed.".into());
        }
        let loopback = match url.host() {
            Some(url::Host::Domain("localhost")) => true,
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            _ => false,
        };
        if url.scheme() == "http" && !loopback && !self.allow_insecure_http {
            return Err("Non-loopback endpoints require HTTPS. For a trusted LAN only, explicitly set allow_insecure_http to true.".into());
        }
        Ok(url)
    }

    pub fn endpoint(&self) -> Result<String> {
        let url = self.url()?;
        Ok(match self.provider {
            Provider::AzureOpenai => url.to_string(),
            Provider::Anthropic => format!("{}/v1/messages", url.as_str().trim_end_matches('/')),
            _ => format!("{}/chat/completions", url.as_str().trim_end_matches('/')),
        })
    }

    pub fn key(&self) -> Result<Option<String>> {
        self.key_with(|name| env::var(name).ok())
    }

    fn key_with(&self, lookup: impl Fn(&str) -> Option<String>) -> Result<Option<String>> {
        let key = if let Some(name) = &self.api_key_env {
            Some(
                lookup(name)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| format!("Set the credential environment variable {name}."))?,
            )
        } else {
            self.provider
                .key_env()
                .and_then(lookup)
                .filter(|v| !v.is_empty())
                .or_else(|| self.api_key.clone())
        };
        if let Some(key) = &key {
            validate_key(key)?;
        } else if self.provider.key_env().is_some() {
            return Err(format!(
                "{} requires an API key in its provider environment variable or configuration.",
                self.provider.name()
            )
            .into());
        }
        Ok(key)
    }
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 16384 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("API keys must be nonempty printable ASCII without whitespace.".into());
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Store {
    pub schema_version: u32,
    pub default_profile: String,
    pub profiles: BTreeMap<String, Config>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            schema_version: 1,
            default_profile: "default".into(),
            profiles: BTreeMap::new(),
        }
    }
}

impl Store {
    pub fn profile(&self, explicit: Option<&str>) -> Result<(&str, &Config)> {
        let name = explicit.unwrap_or(&self.default_profile);
        validate_profile_name(name)?;
        self.profiles
            .get_key_value(name)
            .map(|(name, config)| (name.as_str(), config))
            .ok_or_else(|| {
                format!("Profile {name:?} does not exist. Run howdo /config --profile {name}.")
                    .into()
            })
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err("Unsupported configuration schema_version; update howdo or restore a compatible config.".into());
        }
        if !self.profiles.contains_key(&self.default_profile) {
            return Err("default_profile must name an existing profile.".into());
        }
        for (name, config) in &self.profiles {
            validate_profile_name(name)?;
            config
                .validate()
                .map_err(|e| format!("Invalid profile {name:?}: {e}"))?;
        }
        Ok(())
    }
}

pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || [".", ".."].contains(&name)
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err(
            "Profile names must be 1-64 ASCII letters, digits, dots, underscores, or hyphens."
                .into(),
        );
    }
    Ok(())
}

pub fn path() -> PathBuf {
    let root = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        PathBuf::from(env::var_os("USERPROFILE").unwrap_or_else(|| ".".into()))
                            .join("AppData/Roaming")
                    })
            } else {
                PathBuf::from(env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".config")
            }
        });
    root.join("howdo/config.json")
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("Refusing symbolic link at {}.", path.display()).into())
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn load(path: &Path) -> Result<Option<Store>> {
    reject_symlink(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "Cannot read {}: {e}. The file was not changed.",
                path.display()
            )
            .into())
        }
    };
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err("Config exceeds the 1 MiB limit.".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "Invalid configuration at {}: {e}. The file was not changed.",
            path.display()
        )
    })?;
    let store: Store = if value.get("profiles").is_some() {
        serde_json::from_value(value)
    } else {
        // Legacy single-provider configs remain readable; migration only occurs on explicit save.
        serde_json::from_value::<Config>(value).map(|config| Store {
            profiles: BTreeMap::from([("default".into(), config)]),
            ..Store::default()
        })
    }
    .map_err(|e| {
        format!(
            "Invalid configuration at {}: {e}. The file was not changed.",
            path.display()
        )
    })?;
    store.validate()?;
    Ok(Some(store))
}

/// Lock, reload, and atomically replace to avoid partial files and lost profile updates.
pub fn update(path: &Path, change: impl FnOnce(&mut Store) -> Result<()>) -> Result<()> {
    let parent = path
        .parent()
        .ok_or("Config path has no parent directory.")?;
    reject_symlink(parent)?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let lock_path = parent.join(".config.lock");
    reject_symlink(&lock_path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(lock_path)?;
    lock.lock()?;
    let mut store = load(path)?.unwrap_or_default();
    change(&mut store)?;
    store.validate()?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer_pretty(&mut temporary, &store)?;
    temporary.write_all(b"\n")?;
    if temporary.as_file().metadata()?.len() > MAX_CONFIG_BYTES {
        return Err(
            "Updated config would exceed the 1 MiB limit; the existing file was not changed."
                .into(),
        );
    }
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| format!("Cannot save config: {}", e.error))?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> Config {
        Config::new(
            Provider::Local,
            "http://127.0.0.1:1234/v1".into(),
            "default".into(),
        )
    }

    #[test]
    fn credentials_are_provider_scoped() {
        let lookup = |name: &str| (name == "OPENAI_API_KEY").then(|| "openai-secret".into());
        assert_eq!(local().key_with(lookup).unwrap(), None);
        let mut other = Config::new(
            Provider::Other,
            "https://example.com/v1".into(),
            "model".into(),
        );
        other.api_key = Some("other-secret".into());
        assert_eq!(
            other.key_with(lookup).unwrap().as_deref(),
            Some("other-secret")
        );
        other.api_key_env = Some("CUSTOM_KEY".into());
        assert!(other.key_with(lookup).is_err());
        let mut openai = Config::new(
            Provider::Openai,
            "https://api.openai.com/v1".into(),
            "model".into(),
        );
        openai.api_key = Some("saved".into());
        assert_eq!(
            openai.key_with(lookup).unwrap().as_deref(),
            Some("openai-secret")
        );
        assert_eq!(
            openai.key_with(|_| Some(String::new())).unwrap().as_deref(),
            Some("saved")
        );
    }

    #[test]
    fn validates_endpoints_and_options() {
        let mut config = local();
        for url in [
            "ftp://example.com",
            "http://example.com/v1",
            "https://user:key@example.com",
            "https://example.com/#fragment",
            "https://example.com/?key=secret",
            "not-a-url",
            "http://localhost/\n",
        ] {
            config.base_url = url.into();
            assert!(config.validate().is_err(), "{url}");
        }
        config.base_url = "http://[::1]:1234/v1".into();
        config.validate().unwrap();
        config.timeout_seconds = 0;
        assert!(config.validate().is_err());
        config.timeout_seconds = 30;
        config.request_options.temperature = Some(3.0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn azure_v1_requires_a_model_but_deployment_urls_do_not() {
        let mut config = Config::new(
            Provider::AzureOpenai,
            "https://example.com/openai/v1/chat/completions".into(),
            String::new(),
        );
        assert!(config.validate().is_err());
        config.model = "model".into();
        config.validate().unwrap();
        config.model.clear();
        config.base_url = "https://example.com/openai/deployments/model/chat/completions?api-version=2024-12-01-preview".into();
        config.validate().unwrap();
        config.base_url.push_str("&api-key=secret");
        assert!(config.validate().is_err());
    }

    #[test]
    fn legacy_config_is_read_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let bytes =
            br#"{"provider":"local","base_url":"http://localhost:1234/v1","model":"default"}"#;
        fs::write(&path, bytes).unwrap();
        let store = load(&path).unwrap().unwrap();
        assert!(store.profile(None).is_ok());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn corrupt_config_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"broken").unwrap();
        assert!(load(&path).is_err());
        assert!(update(&path, |_| Ok(())).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken");
    }

    #[test]
    fn saves_profiles_atomically_and_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("howdo/config.json");
        update(&path, |store| {
            store.profiles.insert("default".into(), local());
            Ok(())
        })
        .unwrap();
        update(&path, |store| {
            store.profiles.insert("work".into(), local());
            store.default_profile = "work".into();
            Ok(())
        })
        .unwrap();
        let store = load(&path).unwrap().unwrap();
        assert_eq!(store.profiles.len(), 2);
        assert_eq!(store.default_profile, "work");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn concurrent_profile_updates_are_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("howdo/config.json");
        update(&path, |store| {
            store.profiles.insert("default".into(), local());
            Ok(())
        })
        .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|index| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    update(&path, |store| {
                        store.profiles.insert(format!("profile-{index}"), local());
                        Ok(())
                    })
                    .unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(load(&path).unwrap().unwrap().profiles.len(), 9);
    }

    #[test]
    fn failed_transaction_preserves_previous_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("howdo/config.json");
        update(&path, |store| {
            store.profiles.insert("default".into(), local());
            Ok(())
        })
        .unwrap();
        let original = fs::read(&path).unwrap();
        assert!(update(&path, |store| {
            store.profiles.clear();
            Ok(())
        })
        .is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn refuses_to_write_a_config_that_its_reader_cannot_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("howdo/config.json");
        update(&path, |store| {
            store.profiles.insert("default".into(), local());
            Ok(())
        })
        .unwrap();
        let original = fs::read(&path).unwrap();
        assert!(update(&path, |store| {
            for index in 0..80 {
                let mut config = local();
                config.api_key = Some("x".repeat(16384));
                store.profiles.insert(format!("profile-{index}"), config);
            }
            Ok(())
        })
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(load(&path).is_ok());
    }

    #[test]
    fn validates_profile_names() {
        for name in ["", "../work", "bad name", "..", "é", "a\n"] {
            assert!(validate_profile_name(name).is_err());
        }
        validate_profile_name("work-1.dev").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_config_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let target = dir.path().join("target.json");
        fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(update(&path, |_| Ok(())).is_err());
        assert_eq!(fs::read(target).unwrap(), b"keep");
    }
}
