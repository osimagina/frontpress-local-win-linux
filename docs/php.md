# Managing PHP

FrontPress Local bundles PHP **for you** — there's no Homebrew or system PHP to set up. It downloads small, self‑contained **static PHP** builds and runs each site with PHP's built‑in dev server (`php -S`).

Open **Settings → PHP Manager** to manage runtimes.

<img width="2184" height="1664" alt="Screenshot 2026-06-05 at 11 42 33" src="https://github.com/user-attachments/assets/ef50dea6-8078-479c-9d38-e5d22e2a3804" />


## How it works

- On Linux, runtimes come from **[static‑php.dev](https://dl.static-php.dev/)** — a single `php` binary per version, with the extensions FrontPress needs already compiled in (mbstring, gd, curl, sqlite3, openssl, dom, fileinfo, zip, …).
- On Windows, runtimes come from **[windows.php.net](https://windows.php.net/)** (NTS x64 zips) with a generated `php.ini` enabling the same extensions.
- Each version is stored under the app-data dir (`~/.local/share/FrontPress Local/php/<version>/` on Linux, `%APPDATA%/FrontPress Local/php/<version>/` on Windows).
- **Minimum supported:** PHP **8.1** (FrontPress Studio's requirement).

## The PHP Manager tab

You'll see a row per PHP minor (8.1, 8.2, 8.3, 8.4, …):

- **Download** — fetches the latest patch of that minor.
- **Make default** — sets it as the **global default** used for new sites. The current default is tagged.
- Installed versions are marked **installed**.

## Global default vs. per‑site

- The **global default** is what a new site uses unless you choose otherwise.
- When **creating a site**, you can pick **"Per‑site"** and choose a specific minor for just that site.
- A site remembers its exact version (e.g. `8.3.31`). If that version isn't installed on a machine yet, it's **downloaded automatically the first time the site starts** — handy when [syncing sites across machines](syncing.md).

## Architecture

The PHP Manager shows your machine's architecture (`x86_64`, `aarch64` on Linux, `x64` on Windows) and only offers builds for it.

---

Next: **[Editor →](editor.md)**
