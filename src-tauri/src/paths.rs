//! Filesystem locations FrontPress Local owns (Windows + Linux only).
//!
//! - App data:   %APPDATA%/FrontPress Local/ (Windows)
//!               ~/.local/share/FrontPress Local/ (Linux)
//!     - sites.json          single source of truth for sites + settings
//!     - php/<version>/php(.exe)  downloaded PHP runtimes
//! - Sites:      ~/FrontPress Sites/<name>/   the extracted FrontPress installs

use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// Platform app-data dir joined with "FrontPress Local" (created if missing).
pub fn app_data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().ok_or_else(|| anyhow!("no platform data dir"))?;
    let dir = base.join("FrontPress Local");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// App-data `sites.json`.
pub fn store_file() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("sites.json"))
}

/// Directory holding all downloaded PHP runtimes.
pub fn php_root() -> Result<PathBuf> {
    let dir = app_data_dir()?.join("php");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Filename of the PHP executable on this platform.
pub fn php_bin_name() -> &'static str {
    if cfg!(windows) {
        "php.exe"
    } else {
        "php"
    }
}

/// Path to the PHP binary for a fully-qualified version (e.g. "8.3.9").
pub fn php_binary(version: &str) -> Result<PathBuf> {
    Ok(php_root()?.join(version).join(php_bin_name()))
}

/// Default parent directory for new sites: `~/FrontPress Sites`.
pub fn default_sites_parent() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home dir"))?;
    let dir = home.join("FrontPress Sites");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
