# howdo

Natural language → a terminal command you can review, edit, copy, or run.

```text
$ howdo list files sorted by size

  > ls -lS

Run? (y/e/N; Enter cancels) y
```

A small Rust CLI, not an autonomous agent. One request generates a command; an optional second request explains it. No conversation history, file-content uploads, automatic execution retries, or background agent loops.

## Install

Prebuilt binaries support macOS (arm64/x86_64), Linux (x86_64, glibc 2.35+), and Windows (x86_64). Installers and self-update **require a matching SHA-256 checksum**; older releases without `SHA256SUMS` must be installed manually or built from source.

```sh
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/GitAashishG/howdo/main/install.sh | sh
```

```powershell
# Windows
irm https://raw.githubusercontent.com/GitAashishG/howdo/main/install.ps1 | iex
```

Prefer downloading and reviewing an installer before executing it. Unix installs to `/usr/local/bin` (using sudo only if needed); Windows installs to `%LOCALAPPDATA%\Programs\howdo` and adds it to the user PATH. Set `HOWDO_INSTALL_DIR` to an absolute path for a different destination. Unix users must add a custom directory to PATH themselves.

For a portable Windows install without PATH changes, set `HOWDO_NO_PATH_UPDATE=1` before running the installer.

Build from source with Rust **1.89+** and a C/C++ build toolchain (required by TLS dependencies):

```sh
git clone https://github.com/GitAashishG/howdo.git
cd howdo
cargo build --release --locked
# Add target/release to PATH, or copy the binary into a directory already on PATH.
```

Release assets include GitHub build-provenance attestations. With the GitHub CLI, you can independently verify provenance:

```sh
gh attestation verify ./howdo-aarch64-apple-darwin --repo GitAashishG/howdo
```

Checksums detect corruption; they do not independently authenticate a publisher if the release account is compromised.

## Configure

```sh
howdo /config
```

The wizard supports local LLMs, OpenAI, Azure OpenAI, Anthropic, and other OpenAI-compatible servers. Key input is hidden. Its optional connection test makes a real authenticated inference request, potentially incurring a small API charge, but **never executes the returned command**. Failed validation or inference does not overwrite the existing configuration. Shell startup files are never changed automatically.

- LM Studio: `http://127.0.0.1:1234/v1`; `default` can omit the model field.
- Ollama: `http://127.0.0.1:11434/v1`; specify an installed model from `ollama list`.
- Azure: provide the complete `/chat/completions` URL, including `api-version` if required. The model can be empty for deployment URLs.
- Anthropic: use `https://api.anthropic.com` and an explicit available model name.

Config location:
- Unix: `~/.config/howdo/config.json`
- Windows: `%APPDATA%\howdo\config.json`
- All platforms: `$XDG_CONFIG_HOME/howdo/config.json` when set

### Credentials

Provider environment variables take precedence over stored keys:

| Provider | Implicit credential variable |
|---|---|
| OpenAI | `OPENAI_API_KEY` |
| Azure OpenAI | `AZURE_OPENAI_API_KEY` |
| Anthropic | `ANTHROPIC_API_KEY` |
| Local / Other | **None** |

Local and custom endpoints never receive an unrelated global OpenAI key. For custom credentials, explicitly set `api_key_env` in a profile. That variable must be present and nonempty; missing explicit credentials fail rather than silently falling back. Keys can alternatively be stored in `api_key`.

Unix config files are written atomically with `0600` permissions from creation; the app's config directory is `0700`. Windows relies on the user directory's inherited access controls. Malformed/unreadable configs produce an error, not an automatic replacement wizard. Back up and repair a broken file manually. Symbolic links at the config file are refused.

Remote endpoints require HTTPS by default. A trusted LAN can explicitly opt into unencrypted HTTP using `allow_insecure_http: true`; credentials and queries will then travel without transport encryption. Provider redirects are disabled so credentials cannot silently move to another endpoint.

### Profiles

```sh
howdo /config --profile work
howdo /profiles
howdo /profiles use work
howdo --profile local list files
HOWDO_PROFILE=work howdo --print list files
```

Selection order: `--profile`, `HOWDO_PROFILE`, saved `default_profile`. Profile names use ASCII letters, digits, dots, underscores, or hyphens. Existing single-provider configs remain readable as the `default` profile and migrate only on an explicit configuration save or profile switch.

Example multi-profile config (set `OPENAI_API_KEY` outside the file):

```json
{
  "schema_version": 1,
  "default_profile": "local",
  "profiles": {
    "local": {
      "provider": "local",
      "base_url": "http://127.0.0.1:1234/v1",
      "model": "default"
    },
    "work": {
      "provider": "openai",
      "base_url": "https://api.openai.com/v1",
      "model": "gpt-4.1-mini"
    },
    "proxy": {
      "provider": "other",
      "base_url": "https://my-proxy.example/v1",
      "model": "my-model",
      "api_key_env": "MY_PROXY_API_KEY"
    }
  }
}
```

### Model-specific request options

Temperature is omitted by default because some reasoning models reject it. OpenAI/Azure default to `max_completion_tokens`; local/custom servers default to `max_tokens`. Known OpenAI reasoning model families (`o1`, `o3`, `o4`, `gpt-5`) use the `developer` instruction role. Model capabilities vary; explicitly override options as needed inside a profile:

```json
"request_options": {
  "token_limit": "max_completion_tokens",
  "max_tokens": 2048,
  "system_role": "developer",
  "reasoning_effort": "none"
},
"timeout_seconds": 30
```

- `token_limit`: `auto`, `max_tokens`, or `max_completion_tokens`
- `system_role`: `auto`, `system`, or `developer`
- `temperature`: optional numeric value (0–2 for OpenAI-compatible providers; 0–1 for Anthropic)
- `reasoning_effort`: optional `none`, `minimal`, `low`, `medium`, `high`, or `xhigh`; support depends on the model
- `max_tokens`: 1–32768; `timeout_seconds`: 1–300

Anthropic uses its native system field and content blocks; OpenAI-only options are rejected. Truncated, refused, tool-use, empty, and malformed responses are never offered for execution. Raising the token limit may be necessary for reasoning models because their budget can include reasoning tokens.

## Usage

Put options **before** the query. Once the query starts, later arguments—including flags—are treated as query text. Use `--` for a query starting with a dash or reserved option.

```sh
howdo find python files modified in the last week
howdo --print show disk usage sorted by size
howdo --json list files
howdo --copy compress this folder into a tar.gz
howdo --explain find large files
howdo --profile work --shell /bin/zsh list files
```

- Interactive input requires typing **y** and submitting it with Enter. An empty answer, EOF, and unrecognized answers cancel. Keys typed while a request is pending cannot approve the future command.
- **e** opens an editor, then redisplays and rechecks the edited command before asking again. Editing never executes implicitly.
- Obvious risky commands require typing **RUN**, not merely `y`.
- With noninteractive stdin, the default is **print-only**, even when `y` is piped in.
- `--print`, `--json`, and `--copy` never execute anything. JSON includes command, shell, profile, detected risks, and optional explanation.
- `--explain` makes one extra API request. Its explanation is model-generated, can be wrong, and never changes the command. For print mode it goes to stderr; for JSON it goes into the result.
- Requests show elapsed time in an interactive terminal; **Esc/Ctrl+C** cancels. Cancellation stops the CLI, but the provider may still finish and bill an already-submitted request.
- The executed command's exit status is propagated. Diagnostics and review prompts go to stderr; print/JSON results go to stdout.

Explicit automation is available, but opts into running arbitrary model-generated code:

```sh
howdo --yes print the current date
# For detected risky operations, both overrides are required:
howdo --yes --allow-risky <query>
```

### Shell selection

`--shell` overrides `HOWDO_SHELL`, then Unix `$SHELL` or Windows `%COMSPEC%`. Fallbacks are `/bin/sh` on Unix and `cmd.exe` on Windows. PowerShell's inherited `PSModulePath` is not used to guess the calling shell.

On Windows, explicitly select PowerShell if desired:

```powershell
howdo --shell pwsh list files
$env:HOWDO_SHELL = 'pwsh'
```

Full executable paths are preserved. Supported names: `sh`, `bash`, `zsh`, `fish`, `dash`, `ash`, `ksh`, `mksh`, `powershell`, `pwsh`, and `cmd`. PowerShell launches without profiles; cmd launches with AutoRun disabled. Commands execute in a child shell: `cd`, aliases, and variable assignments do not change the parent shell.

Short aliases:

```sh
# Zsh: suppress glob expansion (not all shell interpretation)
alias q='noglob howdo'

# Bash: noglob is not a Bash command; quote metacharacters in queries
alias q='howdo'
q 'what is listening on port 8000?'
```

For Linux clipboard copying, install `wl-clipboard` on Wayland, or `xclip`/`xsel` on X11. macOS uses `pbcopy`; Windows uses PowerShell `Set-Clipboard`.

## Safety and privacy

**This is not a sandbox.** An accepted command runs with your account's permissions and can delete files, transmit data, or modify the system. Risk checks are conservative string/token heuristics, not a complete shell parser. They can miss dangerous commands and flag harmless ones. Always review the actual command.

The app rejects multiline commands, terminal controls, invisible direction-changing formatting, malformed fences, and reasoning artifacts instead of rewriting their meaning. Well-formed single-line fenced responses can be unwrapped. It does not rewrite quoted strings, concatenate commands, or strip arbitrary comments/tokens.

The selected provider receives your query, OS, shell name, and working-directory path. Explanation mode additionally sends the generated command. File contents and environment variables are not added to the prompt; configured credentials are sent as authentication headers. A subsequently executed command can access any data your account can access.

Update with `howdo /update` (or explicitly `/update --yes`). Updates verify exact asset names and SHA-256 before replacing the binary and never downgrade. Failed updates return nonzero. On Windows, a uniquely named backup directory can remain until a running old executable exits.

## Development

The development test tools require Python 3.11+. They use only the standard library; Ruff is an optional development lint/format tool pinned in `tests/requirements-dev.txt`.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
python3 tests/test_cli.py target/release/howdo
python3 tests/test_installers.py
cargo audit --deny warnings             # install cargo-audit separately
sh tests/bench.sh --runs 50
sh tests/bench.sh --build                # optional clean build in an isolated directory
```

On Windows, pass `target/release/howdo.exe` to the integration suite. Tests use ephemeral local ports, temporary configs, fake credentials, clipboard stubs, and installer download stubs. They never call a real LLM or install into system directories. Unix tests also exercise real terminal confirmation, editing, and cancellation. Benchmarks do not run `cargo clean` or disturb normal build artifacts.

CI runs formatting, strict linting, Rust tests, release builds, and integration tests on macOS/Linux/Windows, plus dependency, shell, workflow, and minimum-Rust checks.

## License

MIT
