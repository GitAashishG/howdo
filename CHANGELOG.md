# Changelog

## 0.2.0 (unreleased)

### Safety and correctness
- Explicit confirmation; Enter/EOF cancel and piped stdin defaults to print-only.
- Provider-scoped credentials, explicit custom credential variables, and no authenticated redirects.
- Strict single-line response validation, safe fence handling, and rejection of terminal controls and incomplete responses.
- Propagated execution/update failures, consistent shell selection, and stronger final-command risk checks.
- Hidden credential entry and atomic, private, locked configuration writes.

### Features
- Print-only and JSON output, clipboard copying, optional explanations, and named profiles.
- Shell overrides and model-specific request options.
- Interactive request progress and cancellation.

### Delivery and maintenance
- Modular Rust implementation with unit, integration, terminal, and installer regression tests.
- Updated dependencies, strict linting, dependency audits, and cross-platform/minimum-Rust CI.
- Intentional tag-driven releases with consistent source versions, checksums, and provenance attestations.
- Verified installers/self-update with unique staging paths, atomic replacement, and no downgrades.
- Non-destructive benchmarks using isolated build directories.

### Migration
- Legacy single-provider configs load as the `default` profile; explicit saves migrate them.
- Use `--yes` for intentional noninteractive execution, and also `--allow-risky` for detected risky operations.
- Local/custom endpoints no longer implicitly use `OPENAI_API_KEY`; configure `api_key_env` if desired.
- Non-loopback HTTP requires the explicit `allow_insecure_http` opt-in.
- On Windows select `--shell pwsh`/`powershell` explicitly rather than relying on inherited environment heuristics.
- Automatic installation/update requires a release containing `SHA256SUMS`.
