use crate::error::Result;
use crate::{http, ui};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use tempfile::NamedTempFile;

const REPO: &str = "GitAashishG/howdo";
const MAX_BINARY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        _ => Err("No prebuilt release is available for this platform; build from source.".into()),
    }
}

fn is_newer(current: &str, latest: &str) -> Result<bool> {
    let current = Version::parse(current).map_err(|_| "Invalid installed version.")?;
    let latest = Version::parse(latest.strip_prefix('v').unwrap_or(latest))
        .map_err(|_| "Release has an invalid semantic version.")?;
    Ok(latest.cmp_precedence(&current).is_gt())
}

fn asset_url(release: &Release, name: &str) -> Result<String> {
    let mut matches = release.assets.iter().filter(|asset| asset.name == name);
    let asset = matches.next().ok_or_else(|| {
        format!(
            "Release {} has no {name}. Automatic updates require binaries and SHA256SUMS.",
            release.tag_name
        )
    })?;
    if matches.next().is_some() {
        return Err("Release contains duplicate asset names.".into());
    }
    let expected = format!(
        "https://github.com/{REPO}/releases/download/{}/{name}",
        release.tag_name
    );
    if asset.browser_download_url != expected {
        return Err("Release asset URL does not match the expected GitHub repository.".into());
    }
    Ok(expected)
}

fn download(url: String, limit: usize) -> Result<Vec<u8>> {
    let response = http::send(
        minreq::get(url)
            .with_header("User-Agent", "howdo-updater")
            .with_timeout(60)
            .with_max_redirects(5),
        limit,
    )?;
    if response.status != 200 {
        return Err(format!("Update download returned HTTP {}.", response.status).into());
    }
    Ok(response.body)
}

pub fn verify_checksum(bytes: &[u8], manifest: &str, name: &str) -> Result<()> {
    let mut expected = None;
    for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
        let (digest, filename) = line
            .split_once(char::is_whitespace)
            .ok_or("Malformed checksum manifest.")?;
        let filename = filename.trim().trim_start_matches('*');
        if digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            || filename.is_empty()
        {
            return Err("Malformed checksum manifest.".into());
        }
        if filename == name && expected.replace(digest.to_ascii_lowercase()).is_some() {
            return Err("Checksum manifest contains duplicate entries.".into());
        }
    }
    let expected = expected.ok_or("Checksum manifest does not contain this binary.")?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err("Update checksum mismatch. The installed binary was not changed.".into());
    }
    Ok(())
}

pub fn install(path: &Path, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Err("Refusing an empty update binary.".into());
    }
    let parent = path
        .parent()
        .ok_or("Binary path has no parent directory.")?;
    let mut staged = NamedTempFile::new_in(parent)?;
    staged.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    staged.as_file().sync_all()?;
    #[cfg(not(windows))]
    {
        staged.persist(path).map_err(|e| {
            format!(
                "Cannot atomically replace binary: {}. Check installation permissions.",
                e.error
            )
        })?;
        fs::File::open(parent)?.sync_all()?;
    }
    #[cfg(windows)]
    {
        // Windows permits renaming a running executable, but not overwriting it in place.
        let backup_dir = tempfile::Builder::new()
            .prefix(".howdo-backup-")
            .tempdir_in(parent)?;
        let backup = backup_dir.path().join("howdo.exe");
        fs::rename(path, &backup).map_err(|e| {
            format!("Cannot move installed binary: {e}. Check installation permissions.")
        })?;
        if let Err(error) = staged.persist(path) {
            if let Err(restore) = fs::rename(&backup, path) {
                let recovery = backup_dir.keep();
                return Err(format!(
                    "Update failed: {}. Restore failed: {restore}. Recover the old binary from {}.",
                    error.error,
                    recovery.display()
                )
                .into());
            }
            return Err(format!(
                "Update failed; the previous binary was restored: {}",
                error.error
            )
            .into());
        }
        // A running backup may remain locked until process exit; unique names prevent collisions.
    }
    Ok(())
}

pub fn run(yes: bool) -> Result<()> {
    if !yes && !ui::interactive() {
        return Err(
            "Updating requires interactive confirmation or an explicit /update --yes.".into(),
        );
    }
    let target = target()?;
    let release = ui::progress("Checking releases", || {
        let response = http::send(
            minreq::get(format!(
                "https://api.github.com/repos/{REPO}/releases/latest"
            ))
            .with_header("User-Agent", "howdo-updater")
            .with_timeout(10)
            .with_follow_redirects(false),
            1024 * 1024,
        )?;
        if response.status != 200 {
            return Err(format!("GitHub release lookup returned HTTP {}.", response.status).into());
        }
        Ok(serde_json::from_slice::<Release>(&response.body)?)
    })?;
    if !is_newer(env!("CARGO_PKG_VERSION"), &release.tag_name)? {
        writeln!(
            io::stderr(),
            "Already up to date (v{}); no downgrade will be installed.",
            env!("CARGO_PKG_VERSION")
        )?;
        return Ok(());
    }
    let name = format!("howdo-{target}{}", if cfg!(windows) { ".exe" } else { "" });
    let binary_url = asset_url(&release, &name)?;
    let checksums_url = asset_url(&release, "SHA256SUMS")?;
    if !yes
        && !ui::confirm(&format!(
            "Install {} over v{}? (y/N)",
            release.tag_name,
            env!("CARGO_PKG_VERSION")
        ))?
    {
        writeln!(io::stderr(), "Update cancelled.")?;
        return Ok(());
    }
    let (binary, manifest) = ui::progress("Downloading verified release", move || {
        let binary = download(binary_url, MAX_BINARY_BYTES)?;
        let manifest = download(checksums_url, 64 * 1024)?;
        Ok((binary, manifest))
    })?;
    let manifest = std::str::from_utf8(&manifest).map_err(|_| "Checksum manifest is not UTF-8.")?;
    verify_checksum(&binary, manifest, &name)?;
    install(&std::env::current_exe()?, &binary)?;
    writeln!(
        io::stderr(),
        "Updated to {} (SHA-256 verified).",
        release.tag_name
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_without_downgrades() {
        assert!(is_newer("0.1.9", "v0.1.10").unwrap());
        assert!(!is_newer("0.2.0", "v0.1.10").unwrap());
        assert!(!is_newer("0.2.0", "v0.2.0").unwrap());
        assert!(is_newer("0.2.0-rc.1", "v0.2.0").unwrap());
        assert!(!is_newer("0.2.0", "v0.2.0-rc.1").unwrap());
        assert!(!is_newer("0.2.0+build.1", "v0.2.0+build.2").unwrap());
        assert!(is_newer("0.2.0", "not-a-version").is_err());
    }

    #[test]
    fn verifies_manifest_strictly() {
        let checksum = format!("{:x}", Sha256::digest(b"binary"));
        let line = format!("{checksum}  howdo-target\n");
        verify_checksum(b"binary", &line, "howdo-target").unwrap();
        verify_checksum(
            b"binary",
            &format!("{checksum} *howdo-target\r\n"),
            "howdo-target",
        )
        .unwrap();
        for (bytes, manifest, name) in [
            (b"corrupt".as_slice(), line.clone(), "howdo-target"),
            (b"binary".as_slice(), line.clone(), "missing"),
            (
                b"binary".as_slice(),
                format!("{line}{line}"),
                "howdo-target",
            ),
            (b"binary".as_slice(), "bad manifest".into(), "howdo-target"),
        ] {
            assert!(verify_checksum(bytes, &manifest, name).is_err());
        }
    }

    #[test]
    fn asset_selection_is_exact_and_repo_scoped() {
        let release = Release {
            tag_name: "v0.2.0".into(),
            assets: vec![Asset {
                name: "howdo-target.sha256".into(),
                browser_download_url: "https://evil.example/binary".into(),
            }],
        };
        assert!(asset_url(&release, "howdo-target").is_err());
        assert!(asset_url(&release, "howdo-target.sha256").is_err());
    }

    #[test]
    fn installation_replaces_binary_without_fixed_temp_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("howdo");
        fs::write(&path, b"old").unwrap();
        install(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(install(&path, b"").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
    }
}
