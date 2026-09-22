//! Persistent state: `sites.json` holding settings + the list of sites.
//! Loaded on demand, written atomically. Credentials live in the OS keychain,
//! never here — this file is plain JSON the user can read.

use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// FrontPress's hard minimum PHP. Bump when the framework raises it.
pub const MIN_PHP_DEFAULT: &str = "8.1";
/// Default PHP minor we steer new installs toward.
pub const PREFERRED_PHP_MINOR: &str = "8.3";

/// FrontPress Studio's shipped default credentials. We use these so the
/// operator knows the current password and can change it from the admin
/// (FrontPress requires the current password as a second factor).
pub const DEFAULT_ADMIN_USER: &str = "fpsadmin";
#[allow(dead_code)]
pub const DEFAULT_ADMIN_PASS: &str = "fpspass";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// Absolute path to the site's webroot (where router.php lives).
    pub path: String,
    pub port: u16,
    /// Fully-resolved PHP version this site runs, e.g. "8.3.9".
    pub php_version: String,
    /// FrontPress Studio version installed, e.g. "0.4.1".
    pub frontpress_version: String,
    pub admin_user: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Fully-resolved default PHP for new sites, e.g. "8.3.9". Empty until
    /// the first runtime is installed.
    #[serde(default)]
    pub global_php_version: String,
    /// Minimum PHP FrontPress accepts.
    #[serde(default = "default_min_php")]
    pub min_php: String,
    /// Favorite editor to open site folders with: the editor command
    /// (e.g. "code", "cursor", "subl"). Empty = none chosen.
    #[serde(default)]
    pub editor: String,
    /// Folder where site directories live. Empty = the default
    /// (`~/FrontPress Sites`). Point this at a Drive/Dropbox folder to keep
    /// sites in sync across machines.
    #[serde(default)]
    pub sites_dir: String,
}

fn default_min_php() -> String {
    MIN_PHP_DEFAULT.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            global_php_version: String::new(),
            min_php: default_min_php(),
            editor: String::new(),
            sites_dir: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Store {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub sites: Vec<Site>,
}

impl Store {
    /// Read `sites.json`, returning a default (empty) store if absent.
    pub fn load() -> Result<Store> {
        let path = paths::store_file()?;
        if !path.exists() {
            return Ok(Store::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        if raw.trim().is_empty() {
            return Ok(Store::default());
        }
        Ok(serde_json::from_str(&raw)?)
    }

    /// Atomically write `sites.json` (temp file + rename).
    pub fn save(&self) -> Result<()> {
        let path = paths::store_file()?;
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn site(&self, id: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.id == id)
    }

    pub fn remove_site(&mut self, id: &str) -> Option<Site> {
        if let Some(idx) = self.sites.iter().position(|s| s.id == id) {
            Some(self.sites.remove(idx))
        } else {
            None
        }
    }

    /// First free port at or above 8081 not already claimed by a site.
    pub fn next_free_port(&self) -> u16 {
        let mut port = 8081u16;
        while self.sites.iter().any(|s| s.port == port) {
            port += 1;
        }
        port
    }

    /// True when no other site already uses this slug.
    pub fn slug_available(&self, slug: &str) -> bool {
        !self.sites.iter().any(|s| s.slug == slug)
    }
}

/// Seconds since the Unix epoch (for `created_at`).
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
