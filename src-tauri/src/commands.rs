//! Tauri command surface: everything the React UI calls. Each command returns
//! `Result<_, String>` so errors surface as readable strings in the frontend.

use crate::server::ServerManager;
use crate::store::{now_secs, Settings, Site, Store};
use crate::{frontpress, keychain, paths, php, siteops, util};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

pub struct AppState {
    pub store: Mutex<Store>,
    pub servers: ServerManager,
}

impl AppState {
    pub fn load() -> Self {
        AppState {
            store: Mutex::new(Store::load().unwrap_or_default()),
            servers: ServerManager::new(),
        }
    }
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Resolve the configured sites parent folder (or the default), creating it.
fn resolve_sites_parent(sites_dir: &str) -> CmdResult<PathBuf> {
    if sites_dir.trim().is_empty() {
        paths::default_sites_parent().map_err(err)
    } else {
        let p = PathBuf::from(sites_dir.trim());
        std::fs::create_dir_all(&p).map_err(err)?;
        Ok(p)
    }
}

/// Write the persistent identity file for a site (best-effort) so another
/// machine can discover it when the folder is synced.
fn write_site_identity(site: &Site) {
    let _ = siteops::write_site_meta(
        &PathBuf::from(&site.path),
        &siteops::BackupMeta {
            name: site.name.clone(),
            php_version: site.php_version.clone(),
            frontpress_version: site.frontpress_version.clone(),
            admin_user: site.admin_user.clone(),
        },
    );
}

/// Scan the configured sites folder and register any FrontPress site that
/// isn't already in the list — this is what makes a synced (Dropbox/Drive)
/// folder show up on another machine. Returns how many were added.
pub(crate) fn discover_sites(app_state: &AppState) -> CmdResult<usize> {
    let dir = {
        let store = app_state.store.lock().map_err(err)?;
        resolve_sites_parent(&store.settings.sites_dir)?
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(0),
    };
    let mut store = app_state.store.lock().map_err(err)?;
    let known: std::collections::HashSet<String> =
        store.sites.iter().map(|s| s.path.clone()).collect();
    let mut added = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dname.starts_with('.') {
            continue;
        }
        // A FrontPress install has router.php at its root.
        if !path.join("router.php").is_file() {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        if known.contains(&path_str) {
            continue;
        }
        let slug = util::slugify(&dname);
        if !store.slug_available(&slug) {
            continue;
        }
        let meta = siteops::read_site_meta(&path).unwrap_or_default();
        let php_version = if !meta.php_version.is_empty() {
            meta.php_version.clone()
        } else {
            store.settings.global_php_version.clone()
        };
        let _ = frontpress::write_login_helper(&path);
        let site = Site {
            id: util::random_id(),
            name: if meta.name.is_empty() { dname } else { meta.name },
            slug,
            path: path_str,
            port: store.next_free_port(),
            php_version,
            frontpress_version: if meta.frontpress_version.is_empty() {
                "?".to_string()
            } else {
                meta.frontpress_version
            },
            admin_user: if meta.admin_user.is_empty() {
                crate::store::DEFAULT_ADMIN_USER.to_string()
            } else {
                meta.admin_user
            },
            created_at: now_secs(),
        };
        store.sites.push(site);
        added += 1;
    }
    if added > 0 {
        store.save().map_err(err)?;
    }
    Ok(added)
}

/// Re-scan the sites folder for new (e.g. synced) sites. Returns fresh status.
#[tauri::command]
pub fn rescan_sites(state: State<'_, AppState>) -> CmdResult<AppStatus> {
    discover_sites(state.inner())?;
    app_status_inner(&state)
}

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteView {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub path: String,
    pub port: u16,
    pub php_version: String,
    pub frontpress_version: String,
    pub admin_user: String,
    pub running: bool,
    pub url: String,
}

fn view_of(site: &Site, servers: &ServerManager) -> SiteView {
    SiteView {
        id: site.id.clone(),
        name: site.name.clone(),
        slug: site.slug.clone(),
        path: site.path.clone(),
        port: site.port,
        php_version: site.php_version.clone(),
        frontpress_version: site.frontpress_version.clone(),
        admin_user: site.admin_user.clone(),
        running: servers.is_running(&site.id),
        url: format!("http://localhost:{}/", site.port),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub sites: Vec<SiteView>,
    pub global_php_version: String,
    pub min_php: String,
    pub arch: String,
    pub installed_php: Vec<String>,
    pub editor: String,
    pub sites_dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhpOption {
    pub minor: String,
    pub latest: String,
    pub installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhpCatalog {
    pub arch: String,
    pub min_php: String,
    pub installed: Vec<String>,
    pub options: Vec<PhpOption>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSiteArgs {
    pub name: String,
    /// "global" | "custom"
    pub php_mode: String,
    /// e.g. "8.3" — required when php_mode == "custom"
    pub php_minor: Option<String>,
}

#[derive(Clone, Serialize)]
struct Progress {
    phase: String,
    message: String,
    pct: Option<f64>,
}

fn emit_progress(app: &AppHandle, phase: &str, message: &str, done: u64, total: Option<u64>) {
    let pct = total.and_then(|t| {
        if t > 0 {
            Some((done as f64 / t as f64) * 100.0)
        } else {
            None
        }
    });
    let _ = app.emit(
        "setup-progress",
        Progress {
            phase: phase.to_string(),
            message: message.to_string(),
            pct,
        },
    );
}

// ── Status / catalogue ──────────────────────────────────────────────────────

#[tauri::command]
pub fn app_status(state: State<'_, AppState>) -> CmdResult<AppStatus> {
    app_status_inner(&state)
}

fn app_status_inner(state: &State<'_, AppState>) -> CmdResult<AppStatus> {
    let store = state.store.lock().map_err(err)?;
    let sites = store
        .sites
        .iter()
        .map(|s| view_of(s, &state.servers))
        .collect();
    Ok(AppStatus {
        sites,
        global_php_version: store.settings.global_php_version.clone(),
        min_php: store.settings.min_php.clone(),
        arch: php::ARCH.to_string(),
        installed_php: php::installed_versions().unwrap_or_default(),
        editor: store.settings.editor.clone(),
        sites_dir: resolve_sites_parent(&store.settings.sites_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| store.settings.sites_dir.clone()),
    })
}

/// Change the global sites folder and **move** every site there. New location
/// can be a Drive/Dropbox folder. Safe: copies all sites first, then switches
/// paths and removes the originals; on any failure nothing is changed.
#[tauri::command]
pub async fn set_sites_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    new_dir: String,
) -> CmdResult<AppStatus> {
    let new_parent = PathBuf::from(new_dir.trim());
    if new_dir.trim().is_empty() {
        return Err("Please choose a folder.".into());
    }
    std::fs::create_dir_all(&new_parent).map_err(err)?;

    // Snapshot current state.
    let (current_parent, sites) = {
        let store = state.store.lock().map_err(err)?;
        let cur = resolve_sites_parent(&store.settings.sites_dir)?;
        (cur, store.sites.clone())
    };
    if new_parent == current_parent {
        return app_status_inner(&state);
    }

    // Stop everything (paths are about to change).
    state.servers.stop_all();

    // Phase 1: copy each site to the new folder. Collect (id, old, new).
    let mut moves: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    for (i, site) in sites.iter().enumerate() {
        let old = PathBuf::from(&site.path);
        let dst = new_parent.join(&site.slug);
        emit_progress(
            &app,
            "config",
            &format!("Moving {} ({}/{})", site.name, i + 1, sites.len()),
            0,
            None,
        );
        if !old.exists() {
            continue; // nothing on disk to move
        }
        if dst.exists() {
            cleanup_partial(&moves);
            return Err(format!(
                "“{}” already exists in the new folder — move it aside first.",
                site.slug
            ));
        }
        let o = old.clone();
        let d = dst.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || siteops::copy_dir_all(&o, &d))
            .await
            .map_err(err)?
        {
            let _ = std::fs::remove_dir_all(&dst);
            cleanup_partial(&moves);
            return Err(err(e));
        }
        moves.push((site.id.clone(), old, dst));
    }

    // Phase 2: commit — update paths, set the new folder, remove originals.
    {
        let mut store = state.store.lock().map_err(err)?;
        for (id, _old, new) in &moves {
            if let Some(s) = store.sites.iter_mut().find(|s| &s.id == id) {
                s.path = new.to_string_lossy().to_string();
            }
        }
        store.settings.sites_dir = new_parent.to_string_lossy().to_string();
        store.save().map_err(err)?;
    }
    for (_id, old, _new) in &moves {
        let _ = std::fs::remove_dir_all(old);
    }

    // Pick up any sites already present in the new folder (e.g. a synced
    // Dropbox folder another machine populated).
    let _ = discover_sites(state.inner());

    emit_progress(&app, "done", "Sites moved", 100, Some(100));
    app_status_inner(&state)
}

/// Remove copies created so far during a failed relocate (rollback helper).
fn cleanup_partial(moves: &[(String, PathBuf, PathBuf)]) {
    for (_id, _old, new) in moves {
        let _ = std::fs::remove_dir_all(new);
    }
}

/// Editors detected on this machine (the favorite-editor picker offers these).
/// Windows + Linux only: probe well-known editor commands on PATH.
/// Anything else can still be typed manually via "Other…" in Settings.
#[tauri::command]
pub fn list_editors() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            "Visual Studio Code",
            "Cursor",
            "Sublime Text",
            "PhpStorm",
            "WebStorm",
            "Notepad++",
            "VSCodium",
            "Zed",
        ];
        candidates
            .iter()
            .filter(|name| windows_editor_available(name))
            .map(|s| s.to_string())
            .collect()
    }
    #[cfg(target_os = "linux")]
    {
        // (display name, binary on PATH)
        let candidates = [
            ("Visual Studio Code", "code"),
            ("Cursor", "cursor"),
            ("VSCodium", "codium"),
            ("Sublime Text", "subl"),
            ("PhpStorm", "phpstorm"),
            ("WebStorm", "webstorm"),
            ("Zed", "zed"),
            ("GNOME Text Editor", "gnome-text-editor"),
            ("Kate", "kate"),
            ("Gedit", "gedit"),
        ];
        candidates
            .iter()
            .filter(|(_, bin)| command_exists(bin))
            .map(|(name, _)| name.to_string())
            .collect()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Vec::new()
    }
}

/// True when `bin` resolves via PATH (`which` on unix, `where` on Windows).
fn command_exists(bin: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("where")
            .arg(bin)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("which")
            .arg(bin)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[cfg(target_os = "windows")]
fn windows_editor_available(display: &str) -> bool {
    // Map display names to their CLI / exe probes.
    let bins: &[&str] = match display {
        "Visual Studio Code" => &["code", "code.cmd"],
        "Cursor" => &["cursor", "cursor.cmd"],
        "Sublime Text" => &["subl", "sublime_text"],
        "PhpStorm" => &["phpstorm", "phpstorm64"],
        "WebStorm" => &["webstorm", "webstorm64"],
        "Notepad++" => &["notepad++", "notepad++.exe"],
        "VSCodium" => &["codium", "codium.cmd"],
        "Zed" => &["zed"],
        _ => &[],
    };
    if bins.iter().any(|b| command_exists(b)) {
        return true;
    }
    // Fall back to default install locations under %ProgramFiles%.
    let pf = std::env::var("ProgramFiles").unwrap_or_default();
    let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_default();
    let paths: &[String] = &match display {
        "Visual Studio Code" => vec![format!("{pf}\\Microsoft VS Code\\Code.exe")],
        "Cursor" => vec![format!(
            "{}\\Cursor\\Cursor.exe",
            std::env::var("LOCALAPPDATA").unwrap_or_default()
        )],
        "Sublime Text" => vec![format!("{pf}\\Sublime Text\\sublime_text.exe")],
        "PhpStorm" => vec![format!("{pf}\\JetBrains\\PhpStorm\\bin\\phpstorm64.exe")],
        "Notepad++" => vec![
            format!("{pf}\\Notepad++\\notepad++.exe"),
            format!("{pf86}\\Notepad++\\notepad++.exe"),
        ],
        _ => vec![],
    };
    paths.iter().any(|p| std::path::Path::new(p).is_file())
}

/// Resolve a display name (as returned by `list_editors`) to the actual
/// command to spawn. Free-typed values pass through untouched.
fn editor_command(editor: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        match editor {
            "Visual Studio Code" => "code".into(),
            "Cursor" => "cursor".into(),
            "Sublime Text" => "subl".into(),
            "PhpStorm" => "phpstorm".into(),
            "WebStorm" => "webstorm".into(),
            "Notepad++" => "notepad++".into(),
            "VSCodium" => "codium".into(),
            "Zed" => "zed".into(),
            other => other.to_string(),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        match editor {
            "Visual Studio Code" => "code".into(),
            "Cursor" => "cursor".into(),
            "VSCodium" => "codium".into(),
            "Sublime Text" => "subl".into(),
            "PhpStorm" => "phpstorm".into(),
            "WebStorm" => "webstorm".into(),
            "Zed" => "zed".into(),
            "GNOME Text Editor" => "gnome-text-editor".into(),
            "Kate" => "kate".into(),
            "Gedit" => "gedit".into(),
            other => other.to_string(),
        }
    }
}

/// Set the favorite editor (app name / command).
#[tauri::command]
pub fn set_editor(state: State<'_, AppState>, editor: String) -> CmdResult<()> {
    let mut store = state.store.lock().map_err(err)?;
    store.settings.editor = editor.trim().to_string();
    store.save().map_err(err)?;
    Ok(())
}

/// Open a site's folder in the configured editor.
#[tauri::command]
pub fn open_in_editor(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let (editor, path) = {
        let store = state.store.lock().map_err(err)?;
        let editor = store.settings.editor.clone();
        let path = store.site(&id).cloned().ok_or("Unknown site")?.path;
        (editor, path)
    };
    if editor.trim().is_empty() {
        return Err("No editor chosen yet — pick one in Settings.".into());
    }
    // Open the editable `site/` folder (content, themes, config, uploads) —
    // not the whole framework. Fall back to the webroot if it isn't seeded yet.
    let webroot = PathBuf::from(&path);
    let target = webroot.join("site");
    let target = if target.is_dir() { target } else { webroot };
    let target = target.to_string_lossy().to_string();

    let cmd = editor_command(&editor);
    // VS Code-likes get a new window so we don't hijack an existing one.
    let new_window = matches!(
        cmd.as_str(),
        "code" | "code.cmd" | "cursor" | "cursor.cmd" | "codium" | "codium.cmd" | "code-insiders"
    );
    let mut c = std::process::Command::new(&cmd);
    if new_window {
        c.arg("--new-window");
    }
    c.arg(&target).spawn().map_err(|e| {
        format!("Couldn't launch “{editor}” ({cmd}): {e}. Check the command in Settings → Editor.")
    })?;
    Ok(())
}

#[tauri::command]
pub async fn available_php(state: State<'_, AppState>) -> CmdResult<PhpCatalog> {
    let min_php = state
        .store
        .lock()
        .map_err(err)?
        .settings
        .min_php
        .clone();

    let remote = php::remote_versions().await.map_err(err)?;
    let installed = php::installed_versions().unwrap_or_default();

    // One option per supported minor (>= min_php), newest patch.
    let mut seen = Vec::<String>::new();
    let mut options = Vec::new();
    for v in &remote {
        let minor = util::minor_of(v);
        if seen.contains(&minor) || !util::version_at_least(&minor, &min_php) {
            continue;
        }
        if let Some(latest) = php::latest_patch(&remote, &minor) {
            options.push(PhpOption {
                installed: installed.contains(&latest),
                minor: minor.clone(),
                latest,
            });
            seen.push(minor);
        }
    }
    Ok(PhpCatalog {
        arch: php::ARCH.to_string(),
        min_php,
        installed,
        options,
    })
}

/// Download a PHP runtime for a given minor (resolving the latest patch).
/// Returns the resolved full version.
#[tauri::command]
pub async fn install_php(app: AppHandle, minor: String) -> CmdResult<String> {
    let remote = php::remote_versions().await.map_err(err)?;
    let version = php::latest_patch(&remote, &minor)
        .ok_or_else(|| format!("No PHP {minor} build available"))?;
    let app2 = app.clone();
    let v2 = version.clone();
    php::ensure_installed(&version, move |done, total| {
        emit_progress(&app2, "php", &format!("Downloading PHP {v2}"), done, total);
    })
    .await
    .map_err(err)?;
    Ok(version)
}

/// Set the default PHP for new sites (resolves + installs the latest patch).
#[tauri::command]
pub async fn set_global_php(
    app: AppHandle,
    state: State<'_, AppState>,
    minor: String,
) -> CmdResult<String> {
    let version = install_php(app, minor).await?;
    let mut store = state.store.lock().map_err(err)?;
    store.settings.global_php_version = version.clone();
    store.save().map_err(err)?;
    Ok(version)
}

// ── Site lifecycle ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn create_site(
    app: AppHandle,
    state: State<'_, AppState>,
    args: CreateSiteArgs,
) -> CmdResult<SiteView> {
    let name = args.name.trim().to_string();
    if name.is_empty() {
        return Err("Please enter a site name.".into());
    }
    let slug = util::slugify(&name);

    // Snapshot what we need without holding the lock across awaits.
    let (port, min_php, global_php, sites_dir) = {
        let store = state.store.lock().map_err(err)?;
        if !store.slug_available(&slug) {
            return Err(format!("A site named “{slug}” already exists."));
        }
        (
            store.next_free_port(),
            store.settings.min_php.clone(),
            store.settings.global_php_version.clone(),
            store.settings.sites_dir.clone(),
        )
    };

    // Resolve the PHP version to run.
    let php_version = match args.php_mode.as_str() {
        "custom" => {
            let minor = args
                .php_minor
                .clone()
                .ok_or("Please choose a PHP version.")?;
            if !util::version_at_least(&minor, &min_php) {
                return Err(format!(
                    "PHP {minor} is below FrontPress's minimum ({min_php})."
                ));
            }
            let remote = php::remote_versions().await.map_err(err)?;
            php::latest_patch(&remote, &minor)
                .ok_or_else(|| format!("No PHP {minor} build available"))?
        }
        _ => {
            if !global_php.is_empty() {
                global_php
            } else {
                let remote = php::remote_versions().await.map_err(err)?;
                php::latest_patch(&remote, crate::store::PREFERRED_PHP_MINOR)
                    .ok_or("No default PHP build available")?
            }
        }
    };
    if !util::version_at_least(&php_version, &min_php) {
        return Err(format!(
            "Selected PHP {php_version} is below the minimum {min_php}."
        ));
    }

    // 1. PHP runtime.
    let app_php = app.clone();
    let vlabel = php_version.clone();
    php::ensure_installed(&php_version, move |done, total| {
        emit_progress(&app_php, "php", &format!("Preparing PHP {vlabel}"), done, total);
    })
    .await
    .map_err(err)?;

    // 2. Site directory.
    let site_dir = resolve_sites_parent(&sites_dir)?.join(&slug);
    if site_dir.exists() {
        return Err(format!("Directory already exists: {}", site_dir.display()));
    }

    // 3. FrontPress release.
    emit_progress(&app, "frontpress", "Finding latest FrontPress Studio…", 0, None);
    let release = frontpress::latest_release().await.map_err(err)?;
    let fp_version = release.version.clone();
    let app_fp = app.clone();
    let label_version = fp_version.clone();
    frontpress::install_into(&release.zip_url, &site_dir, move |done, total| {
        emit_progress(
            &app_fp,
            "frontpress",
            &format!("Downloading FrontPress Studio {label_version}"),
            done,
            total,
        );
    })
    .await
    .map_err(|e| {
        let _ = std::fs::remove_dir_all(&site_dir);
        err(e)
    })?;

    // 4. Login bridge only. We deliberately DON'T generate config.php —
    //    FrontPress runs on its shipped sample.config.php defaults
    //    (fpsadmin / fpspass, dev env) and promotes sample → config.php
    //    itself on the first credential change.
    emit_progress(&app, "config", "Configuring site…", 0, None);
    let admin_user = crate::store::DEFAULT_ADMIN_USER.to_string();
    frontpress::write_login_helper(&site_dir).map_err(err)?;
    let id = util::random_id();

    // 5. Persist.
    let site = Site {
        id: id.clone(),
        name,
        slug,
        path: site_dir.to_string_lossy().to_string(),
        port,
        php_version,
        frontpress_version: fp_version,
        admin_user,
        created_at: now_secs(),
    };
    write_site_identity(&site);
    {
        let mut store = state.store.lock().map_err(err)?;
        // If global PHP was unset, adopt what we just resolved.
        if store.settings.global_php_version.is_empty() {
            store.settings.global_php_version = site.php_version.clone();
        }
        store.sites.push(site.clone());
        store.save().map_err(err)?;
    }

    // Bring the new site online right away (best-effort — a failed start
    // still leaves the site created; the user can start it from the toggle).
    emit_progress(&app, "config", "Starting site…", 0, None);
    let _ = start_internal(&state, &site);

    emit_progress(&app, "done", "Site ready", 100, Some(100));
    Ok(view_of(&site, &state.servers))
}

#[tauri::command]
pub fn start_site(state: State<'_, AppState>, id: String) -> CmdResult<SiteView> {
    let site = {
        let store = state.store.lock().map_err(err)?;
        store.site(&id).cloned().ok_or("Unknown site")?
    };
    start_internal(&state, &site)?;
    Ok(view_of(&site, &state.servers))
}

#[tauri::command]
pub fn stop_site(state: State<'_, AppState>, id: String) -> CmdResult<SiteView> {
    state.servers.stop(&id).map_err(err)?;
    let store = state.store.lock().map_err(err)?;
    let site = store.site(&id).cloned().ok_or("Unknown site")?;
    Ok(view_of(&site, &state.servers))
}

/// Stop every running site at once (the header "Stop all" button).
#[tauri::command]
pub fn stop_all_sites(state: State<'_, AppState>) -> CmdResult<()> {
    state.servers.stop_all();
    Ok(())
}

/// Duplicate a site into a new folder with its own port + fresh credentials,
/// then bring it online.
#[tauri::command]
pub async fn duplicate_site(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> CmdResult<SiteView> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Please enter a name for the copy.".into());
    }
    let slug = util::slugify(&name);
    let src = {
        let store = state.store.lock().map_err(err)?;
        store.site(&id).cloned().ok_or("Unknown site")?
    };
    let (port, sites_dir) = {
        let store = state.store.lock().map_err(err)?;
        if !store.slug_available(&slug) {
            return Err(format!("A site named “{slug}” already exists."));
        }
        (store.next_free_port(), store.settings.sites_dir.clone())
    };

    let src_dir = PathBuf::from(&src.path);
    let dst_dir = resolve_sites_parent(&sites_dir)?.join(&slug);
    if dst_dir.exists() {
        return Err(format!("Directory already exists: {}", dst_dir.display()));
    }

    emit_progress(&app, "config", "Copying site files…", 0, None);
    let s2 = src_dir.clone();
    let d2 = dst_dir.clone();
    tokio::task::spawn_blocking(move || siteops::copy_dir_all(&s2, &d2))
        .await
        .map_err(err)?
        .map_err(|e| {
            let _ = std::fs::remove_dir_all(&dst_dir);
            err(e)
        })?;

    // Reset the copy to FrontPress's shipped defaults: drop any config.php so
    // it falls back to sample.config.php (fpsadmin / fpspass). Keep the bridge.
    let admin_user = crate::store::DEFAULT_ADMIN_USER.to_string();
    let _ = std::fs::remove_file(dst_dir.join("config.php"));
    frontpress::write_login_helper(&dst_dir).map_err(err)?;
    let new_id = util::random_id();

    // The source's PHP should already be installed; make sure.
    php::ensure_installed(&src.php_version, |_, _| {})
        .await
        .map_err(err)?;

    let site = Site {
        id: new_id,
        name,
        slug,
        path: dst_dir.to_string_lossy().to_string(),
        port,
        php_version: src.php_version.clone(),
        frontpress_version: src.frontpress_version.clone(),
        admin_user,
        created_at: now_secs(),
    };
    write_site_identity(&site);
    {
        let mut store = state.store.lock().map_err(err)?;
        store.sites.push(site.clone());
        store.save().map_err(err)?;
    }
    emit_progress(&app, "config", "Starting site…", 0, None);
    let _ = start_internal(&state, &site);
    emit_progress(&app, "done", "Site ready", 100, Some(100));
    Ok(view_of(&site, &state.servers))
}

/// Import an existing FrontPress site folder (e.g. one synced via Dropbox)
/// in place — registers it without moving it, then starts it.
#[tauri::command]
pub async fn import_site(
    state: State<'_, AppState>,
    folder: String,
) -> CmdResult<SiteView> {
    let path = PathBuf::from(folder.trim());
    if folder.trim().is_empty() {
        return Err("Please choose a folder.".into());
    }
    if !path.join("router.php").is_file() {
        return Err("That folder isn't a FrontPress site (no router.php).".into());
    }
    let path_str = path.to_string_lossy().to_string();
    let dname = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("site")
        .to_string();

    let site = {
        let mut store = state.store.lock().map_err(err)?;
        if store.sites.iter().any(|s| s.path == path_str) {
            return Err("That folder is already added.".into());
        }
        let base = util::slugify(&dname);
        let mut slug = base.clone();
        let mut n = 2;
        while !store.slug_available(&slug) {
            slug = format!("{base}-{n}");
            n += 1;
        }
        let meta = siteops::read_site_meta(&path).unwrap_or_default();
        let php_version = if !meta.php_version.is_empty() {
            meta.php_version.clone()
        } else {
            store.settings.global_php_version.clone()
        };
        let site = Site {
            id: util::random_id(),
            name: if meta.name.is_empty() { dname } else { meta.name },
            slug,
            path: path_str,
            port: store.next_free_port(),
            php_version,
            frontpress_version: if meta.frontpress_version.is_empty() {
                "?".to_string()
            } else {
                meta.frontpress_version
            },
            admin_user: if meta.admin_user.is_empty() {
                crate::store::DEFAULT_ADMIN_USER.to_string()
            } else {
                meta.admin_user
            },
            created_at: now_secs(),
        };
        let _ = frontpress::write_login_helper(&path);
        write_site_identity(&site);
        store.sites.push(site.clone());
        store.save().map_err(err)?;
        site
    };

    if !site.php_version.is_empty() {
        php::ensure_installed(&site.php_version, |_, _| {})
            .await
            .map_err(err)?;
    }
    let _ = start_internal(&state, &site);
    Ok(view_of(&site, &state.servers))
}

/// Zip the site's `site/` folder (content, themes, config, uploads) to `dest`.
/// The framework itself isn't backed up — it's re-downloadable.
#[tauri::command]
pub async fn backup_site(
    state: State<'_, AppState>,
    id: String,
    dest: String,
) -> CmdResult<()> {
    let site = {
        let store = state.store.lock().map_err(err)?;
        store.site(&id).cloned().ok_or("Unknown site")?
    };
    let site_dir = PathBuf::from(&site.path).join("site");
    if !site_dir.is_dir() {
        return Err(
            "Open this site once (Preview or Login) so its content folder exists, then back it up."
                .into(),
        );
    }
    let meta = siteops::BackupMeta {
        name: site.name.clone(),
        php_version: site.php_version.clone(),
        frontpress_version: site.frontpress_version.clone(),
        admin_user: site.admin_user.clone(),
    };
    let dest_path = PathBuf::from(dest);
    tokio::task::spawn_blocking(move || {
        // Drop a metadata sidecar in, zip (minus the one-shot login token and
        // the regenerable cache), then remove it.
        siteops::write_meta(&site_dir, &meta)?;
        let res = siteops::zip_dir(&site_dir, &dest_path, &[".fp-local-login", "cache"]);
        let _ = std::fs::remove_file(site_dir.join(siteops::META_FILE));
        res
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    Ok(())
}

/// Restore a specific site's content IN PLACE from one of its backup zips:
/// stop it, swap only its `site/` folder for the backup contents, then start
/// it again. The framework, config.php and login bridge are left untouched —
/// you're rolling this site's content back to a snapshot.
#[tauri::command]
pub async fn restore_into_site(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    zip_path: String,
) -> CmdResult<SiteView> {
    let site = {
        let store = state.store.lock().map_err(err)?;
        store.site(&id).cloned().ok_or("Unknown site")?
    };
    let zp = PathBuf::from(&zip_path);
    if zp.extension().and_then(|e| e.to_str()) != Some("zip") {
        return Err("Please drop a .zip backup file.".into());
    }

    // Unpack the backup to a temp dir that's a sibling of the actual site
    // (same filesystem, so the swap below is an atomic rename).
    let parent = PathBuf::from(&site.path)
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or("Bad site path")?;
    let tmp_dir = parent.join(format!(".restore-{}", util::random_id()));
    let zp2 = zp.clone();
    let tmp2 = tmp_dir.clone();
    emit_progress(&app, "config", "Reading backup…", 0, None);
    tokio::task::spawn_blocking(move || siteops::unzip(&zp2, &tmp2))
        .await
        .map_err(err)?
        .map_err(|e| {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            err(e)
        })?;
    if let Err(e) = siteops::looks_like_site_folder(&tmp_dir) {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(err(e));
    }
    let _ = siteops::take_meta(&tmp_dir); // discard the sidecar from the restored content

    // Stop the site, then swap ONLY the site/ folder (current → aside, backup
    // → live), leaving the framework in place.
    state.servers.stop(&id).map_err(err)?;
    let site_sub = PathBuf::from(&site.path).join("site");
    let bak_dir = parent.join(format!(".bak-{}", util::random_id()));
    emit_progress(&app, "config", "Restoring content…", 0, None);
    if site_sub.exists() {
        std::fs::rename(&site_sub, &bak_dir).map_err(|e| {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            err(e)
        })?;
    }
    if let Err(e) = std::fs::rename(&tmp_dir, &site_sub) {
        let _ = std::fs::rename(&bak_dir, &site_sub); // roll back
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(err(e));
    }
    let _ = std::fs::remove_dir_all(&bak_dir);

    php::ensure_installed(&site.php_version, |_, _| {})
        .await
        .map_err(err)?;
    emit_progress(&app, "config", "Starting site…", 0, None);
    let _ = start_internal(&state, &site);
    emit_progress(&app, "done", "Site restored", 100, Some(100));
    Ok(view_of(&site, &state.servers))
}

fn start_internal(state: &State<'_, AppState>, site: &Site) -> CmdResult<()> {
    let php_bin = paths::php_binary(&site.php_version).map_err(err)?;
    if !php_bin.is_file() {
        return Err(format!(
            "PHP {} isn't installed. Re-install it from PHP settings.",
            site.php_version
        ));
    }
    let webroot = PathBuf::from(&site.path);
    let log = paths::app_data_dir()
        .map_err(err)?
        .join("logs")
        .join(format!("{}.log", site.id));
    state
        .servers
        .start(&site.id, &php_bin, site.port, &webroot, &log)
        .map_err(err)
}

#[tauri::command]
pub fn delete_site(
    state: State<'_, AppState>,
    id: String,
    delete_files: bool,
) -> CmdResult<()> {
    state.servers.stop(&id).map_err(err)?;
    let removed = {
        let mut store = state.store.lock().map_err(err)?;
        let removed = store.remove_site(&id);
        store.save().map_err(err)?;
        removed
    };
    let _ = keychain::delete_password(&id);
    if let Some(site) = removed {
        if delete_files {
            let _ = std::fs::remove_dir_all(&site.path);
        }
    }
    Ok(())
}

// ── Browser actions ─────────────────────────────────────────────────────────

#[tauri::command]
pub fn open_preview(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let site = {
        let store = state.store.lock().map_err(err)?;
        store.site(&id).cloned().ok_or("Unknown site")?
    };
    start_internal(&state, &site)?;
    open_url(&format!("http://localhost:{}/", site.port))
}

#[tauri::command]
pub fn auto_login(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let site = {
        let store = state.store.lock().map_err(err)?;
        store.site(&id).cloned().ok_or("Unknown site")?
    };
    start_internal(&state, &site)?;

    // Issue a single-use token the bridge will consume.
    let token = util::random_token();
    let token_path = frontpress::login_token_path(&PathBuf::from(&site.path));
    if let Some(parent) = token_path.parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    std::fs::write(&token_path, &token).map_err(err)?;

    open_url(&format!(
        "http://localhost:{}/fp-local-login.php?token={}",
        site.port, token
    ))
}

#[tauri::command]
pub fn reveal_in_finder(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let path = {
        let store = state.store.lock().map_err(err)?;
        store.site(&id).cloned().ok_or("Unknown site")?.path
    };
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn()
        .map_err(err)?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map_err(err)?;
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    return Err("Revealing files is only supported on Windows and Linux in this build.".into());
    Ok(())
}

fn open_url(url: &str) -> CmdResult<()> {
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn()
        .map_err(err)?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map_err(err)?;
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    return Err("Opening URLs is only supported on Windows and Linux in this build.".into());
    Ok(())
}

/// Re-read the on-disk settings into a struct the UI can show.
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(state.store.lock().map_err(err)?.settings.clone())
}

/// Test-only: true when launched with FP_SELFTEST_UPDATE set, so the update
/// banner can auto-apply during an automated end-to-end updater test.
#[tauri::command]
pub fn selftest_update() -> bool {
    std::env::var("FP_SELFTEST_UPDATE").is_ok()
}
