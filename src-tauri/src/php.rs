//! PHP runtime management. Downloads a portable PHP per fully-qualified
//! version and resolves the latest patch for a given minor.
//!
//! - Linux: prebuilt static binaries from static-php.dev (single `php` file).
//! - Windows: official builds from windows.php.net (php.exe + DLLs + ext/),
//!   with a generated php.ini that enables the extensions FrontPress needs.
//!
//! macOS is intentionally NOT supported in this fork (Windows + Linux only).

use crate::{net, paths, util};
use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Architecture token shown in the UI / used in static-php.dev filenames.
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const ARCH: &str = "aarch64";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const ARCH: &str = "x86_64";
#[cfg(target_os = "windows")]
pub const ARCH: &str = "x64";
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub const ARCH: &str = "x86_64";

/// Highest patch release for a given "major.minor".
pub fn latest_patch(versions: &[String], minor: &str) -> Option<String> {
    versions
        .iter()
        .filter(|v| util::minor_of(v) == minor)
        .max_by(|a, b| util::parse_version(a).cmp(&util::parse_version(b)))
        .cloned()
}

/// Versions already downloaded (the PHP binary exists under php/<version>/).
pub fn installed_versions() -> Result<Vec<String>> {
    let root = paths::php_root()?;
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            if e.path().join(paths::php_bin_name()).is_file() {
                if let Some(name) = e.file_name().to_str() {
                    out.push(name.to_string());
                }
            }
        }
    }
    out.sort_by(|a, b| util::parse_version(b).cmp(&util::parse_version(a)));
    Ok(out)
}

/// Fetch the remote catalogue of installable PHP versions, newest first.
pub async fn remote_versions() -> Result<Vec<String>> {
    #[cfg(target_os = "windows")]
    {
        win::remote_versions().await
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix::remote_versions().await
    }
}

/// Ensure the given fully-qualified PHP version is installed locally,
/// downloading + extracting it if needed. Returns the path to the binary.
pub async fn ensure_installed<F>(version: &str, progress: F) -> Result<PathBuf>
where
    F: Fn(u64, Option<u64>),
{
    let bin = paths::php_binary(version)?;
    if bin.is_file() {
        return Ok(bin);
    }
    let dir = paths::php_root()?.join(version);
    std::fs::create_dir_all(&dir)?;

    #[cfg(target_os = "windows")]
    {
        win::install(version, &dir, &progress).await?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix::install(version, &dir, &progress).await?;
    }
    Ok(bin)
}

/// Stream a download to `dest`, reporting progress.
async fn download_to<F>(url: &str, dest: &Path, progress: &F) -> Result<()>
where
    F: Fn(u64, Option<u64>),
{
    let resp = net::client()?
        .get(url)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("download {url}"))?;
    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = std::fs::File::create(dest)?;
    let mut done = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush()?;
    Ok(())
}

// ── Linux: static-php.dev ────────────────────────────────────────────────────
/// Single-file static `php` CLI builds: php-{version}-cli-linux-{ARCH}.tar.gz
#[cfg(not(target_os = "windows"))]
mod unix {
    use super::*;

    const BASE: &str = "https://dl.static-php.dev/static-php-cli/common";

    fn download_url(version: &str) -> String {
        format!("{BASE}/php-{version}-cli-linux-{ARCH}.tar.gz")
    }

    pub async fn remote_versions() -> Result<Vec<String>> {
        let body = net::client()?
            .get(format!("{BASE}/"))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let versions = parse_listing(&body, ARCH);
        if versions.is_empty() {
            return Err(anyhow!("no PHP builds found in static-php listing"));
        }
        Ok(versions)
    }

    pub async fn install<F>(version: &str, dir: &Path, progress: &F) -> Result<()>
    where
        F: Fn(u64, Option<u64>),
    {
        let tmp = dir.join("download.tar.gz");
        download_to(&download_url(version), &tmp, progress)
            .await
            .with_context(|| format!("PHP {version} not available for {ARCH}"))?;

        let bin = paths::php_binary(version)?;
        let tmp2 = tmp.clone();
        let bin2 = bin.clone();
        tokio::task::spawn_blocking(move || extract_php(&tmp2, &bin2))
            .await
            .context("join extract task")??;

        let _ = std::fs::remove_file(&tmp);
        Ok(())
    }

    fn extract_php(tarball: &Path, dest: &Path) -> Result<()> {
        use flate2::read::GzDecoder;
        let f = std::fs::File::open(tarball)?;
        let mut archive = tar::Archive::new(GzDecoder::new(f));
        for entry in archive.entries()? {
            let mut entry = entry?;
            let is_php = entry.path()?.file_name().map(|n| n == "php").unwrap_or(false);
            if is_php {
                entry.unpack(dest)?;
                set_executable(dest)?;
                return Ok(());
            }
        }
        Err(anyhow!("no `php` binary inside tarball"))
    }

    fn set_executable(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }

    /// Parse a static-php directory listing into the available full versions.
    /// `arch` is e.g. "x86_64" / "aarch64", matched against
    /// `-cli-linux-{arch}.tar.gz` entries.
    pub fn parse_listing(body: &str, arch: &str) -> Vec<String> {
        let suffix = format!("-cli-linux-{arch}.tar.gz");
        let mut out = Vec::new();
        for chunk in body.split("php-").skip(1) {
            if let Some(idx) = chunk.find(&suffix) {
                let ver = &chunk[..idx];
                if !ver.is_empty()
                    && ver.contains('.')
                    && ver.chars().all(|c| c.is_ascii_digit() || c == '.')
                {
                    out.push(ver.to_string());
                }
            }
        }
        out.sort_by(|a, b| util::parse_version(b).cmp(&util::parse_version(a)));
        out.dedup();
        out
    }
}

// ── Windows: windows.php.net ─────────────────────────────────────────────────
#[cfg(target_os = "windows")]
mod win {
    use super::*;

    const RELEASES_JSON: &str = "https://downloads.php.net/~windows/releases/releases.json";
    const DOWNLOAD_BASE: &str = "https://windows.php.net/downloads/releases";

    /// (full version, zip filename) for each branch's NTS x64 build.
    async fn releases() -> Result<Vec<(String, String)>> {
        let json: serde_json::Value = net::client()?
            .get(RELEASES_JSON)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut out = Vec::new();
        if let Some(obj) = json.as_object() {
            for (_branch, b) in obj {
                let ver = match b.get("version").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => continue,
                };
                if let Some(bo) = b.as_object() {
                    for (k, v) in bo {
                        let kl = k.to_lowercase();
                        if kl.contains("nts") && kl.contains("x64") {
                            if let Some(path) = v
                                .get("zip")
                                .and_then(|z| z.get("path"))
                                .and_then(|p| p.as_str())
                            {
                                out.push((ver, path.to_string()));
                                break;
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    pub async fn remote_versions() -> Result<Vec<String>> {
        let mut v: Vec<String> = releases().await?.into_iter().map(|(ver, _)| ver).collect();
        if v.is_empty() {
            return Err(anyhow!("no Windows PHP builds found"));
        }
        v.sort_by(|a, b| util::parse_version(b).cmp(&util::parse_version(a)));
        v.dedup();
        Ok(v)
    }

    pub async fn install<F>(version: &str, dir: &Path, progress: &F) -> Result<()>
    where
        F: Fn(u64, Option<u64>),
    {
        let file = releases()
            .await?
            .into_iter()
            .find(|(ver, _)| ver == version)
            .map(|(_, f)| f)
            .ok_or_else(|| anyhow!("PHP {version} not available for Windows"))?;
        let url = format!("{DOWNLOAD_BASE}/{file}");

        let tmp = dir.join("download.zip");
        download_to(&url, &tmp, progress).await?;

        let dir2 = dir.to_path_buf();
        let tmp2 = tmp.clone();
        tokio::task::spawn_blocking(move || crate::siteops::unzip(&tmp2, &dir2))
            .await
            .context("join unzip task")??;
        let _ = std::fs::remove_file(&tmp);

        write_php_ini(dir)?;
        Ok(())
    }

    /// php.exe loads php.ini from its own directory; enable the extensions
    /// FrontPress needs and point extension_dir at the absolute ext/ folder.
    fn write_php_ini(dir: &Path) -> Result<()> {
        let ext = dir.join("ext").to_string_lossy().replace('\\', "/");
        let mut ini = format!("extension_dir=\"{ext}\"\n");
        for e in [
            "mbstring", "openssl", "curl", "gd", "fileinfo", "sqlite3",
            "pdo_sqlite", "exif", "zip", "intl",
        ] {
            ini.push_str(&format!("extension={e}\n"));
        }
        std::fs::write(dir.join("php.ini"), ini)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_patch_picks_highest() {
        let v = vec![
            "8.3.9".to_string(),
            "8.3.7".to_string(),
            "8.3.10".to_string(),
            "8.1.34".to_string(),
        ];
        assert_eq!(latest_patch(&v, "8.3"), Some("8.3.10".to_string()));
        assert_eq!(latest_patch(&v, "8.1"), Some("8.1.34".to_string()));
        assert_eq!(latest_patch(&v, "8.9"), None);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn parse_listing_extracts_versions() {
        let html = r#"
            <a href="/static-php-cli/common/php-8.1.34-cli-linux-aarch64.tar.gz">x</a>
            <a href="/static-php-cli/common/php-8.3.9-cli-linux-aarch64.tar.gz">x</a>
            <a href="/static-php-cli/common/php-8.2.21-cli-linux-x86_64.tar.gz">x</a>
            <a href="/static-php-cli/common/php-8.4.1-cli-linux-aarch64.tar.gz">x</a>
        "#;
        let v = unix::parse_listing(html, "aarch64");
        assert_eq!(v.first().map(String::as_str), Some("8.4.1"));
        assert!(v.contains(&"8.1.34".to_string()));
        assert!(!v.contains(&"8.2.21".to_string()));
    }
}
