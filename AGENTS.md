# Agent directives — frontpress-local-win-linux

Windows + Linux fork of FrontPress Local (upstream is macOS-only).
Target matrix: Windows 10/11 x64 (NSIS) + Linux x64 (AppImage, deb).

## Language rules (mandatory)

- Write ALL commit messages in ENGLISH, even when the conversation with the
  operator is in Spanish. Imperative mood, concise subject line (~50 chars),
  bullet-point body explaining what/why.
- Write all CODE, code comments, and doc comments in ENGLISH.
- Docs (`docs/`, `README.md`) and user-facing UI strings stay in ENGLISH
  (upstream convention).
- Chat with the operator may be in Spanish.

## Workflow rules

- Never push to `upstream` (krstivoja/frontpress-local, macOS original).
  `origin` is this fork; feature work lands on `origin/main`.
- Version lives in THREE files, keep them in sync:
  `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`
  (`src-tauri/Cargo.lock` updates via cargo).
- Tags (`v*`) trigger the Release workflow (Windows + Linux artifacts +
  `latest.json`). Only tag from a green `cargo test` + `vite build`.
- Updater signing: releases require `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)`
  repo secrets and matching `plugins.updater` endpoint/pubkey in
  `tauri.conf.json` pointing at THIS fork's releases.
- Verify with real runs whenever possible: `cargo test` (backend),
  `npm run build` (frontend), `npm run tauri dev` (smoke test).
  Linux system deps for the backend: `pkg-config libdbus-1-dev
  libwebkit2gtk-4.1-dev build-essential libssl-dev libgtk-3-dev
  libayatana-appindicator3-dev librsvg2-dev`.

## Platform notes

- Rust `#[cfg]` gating: `windows` → `win::` (windows.php.net NTS),
  non-Windows → `unix::` (static-php.dev linux tarballs). macOS is
  intentionally unsupported in this fork.
- Windows quirks on record: Defender quarantines `router.php` on site
  creation (needs exclusions + post-install entrypoint verification);
  `php -S` `$_SERVER` values differ from Linux (see `//assets` issue);
  symlinks need Developer Mode (framework mirrors `assets/` instead).
