# Installation

FrontPress Local is a native app for **Windows and Linux** (Tauri + Rust).

## 1. Download

Open the **[Releases page](https://github.com/krstivoja/frontpress-local/releases/latest)** and download the asset for your OS:

- **Windows 10/11 x64:** `FrontPress-Local_<version>_x64-setup.exe` (NSIS installer, per-user). Run it and follow the wizard. Requires **WebView2** (preinstalled on Windows 11; on Windows 10 install [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) first if prompted).
- **Linux x64 (Ubuntu/Debian):** `FrontPress-Local_<version>_amd64.AppImage` (make executable and run) or `frontpress-local_<version>_amd64.deb` (`sudo dpkg -i …`). Requires **WebKitGTK** (`libwebkit2gtk-4.1`) — on Ubuntu: `sudo apt install libwebkit2gtk-4.1-0`.

> Prefer the terminal on Linux? `chmod +x FrontPress-Local_*.AppImage && ./FrontPress-Local_*.AppImage`

## 2. Updates

The app **checks for updates on launch** and via the menu bar (**Help → Check for Updates…**). When a new version is available you'll see an amber **"Update available"** bar at the top of the window — click **Install & restart**.

## Where things are stored

- **Your sites:** `~/FrontPress Sites/` by default (configurable — see [Sites location & sync](syncing.md)).
- **App data:** `%APPDATA%/FrontPress Local/` on Windows, `~/.local/share/FrontPress Local/` on Linux — the downloaded PHP runtimes, the site list (`sites.json`), and per‑site server logs.

---

Next: **[Creating & managing websites →](websites.md)**
