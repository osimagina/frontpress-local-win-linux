import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, ask } from "@tauri-apps/plugin-dialog";
import { api, PhpCatalog, SetupProgress } from "../api";
import { Modal } from "./Modal";

const CUSTOM = "__custom__";

export function SettingsModal({ onClose }: { onClose: () => void }) {
  const [catalog, setCatalog] = useState<PhpCatalog | null>(null);
  const [globalVersion, setGlobalVersion] = useState("");
  const [busyMinor, setBusyMinor] = useState<string | null>(null);
  const [progress, setProgress] = useState<SetupProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [editors, setEditors] = useState<string[]>([]);
  const [editor, setEditor] = useState("");
  const [customEditor, setCustomEditor] = useState("");
  const [picker, setPicker] = useState(""); // current <select> value

  const [sitesDir, setSitesDir] = useState("");
  const [siteCount, setSiteCount] = useState(0);
  const [movingSites, setMovingSites] = useState(false);

  const [tab, setTab] = useState<"php" | "editor" | "location">("php");

  const load = async () => {
    try {
      const [cat, status, eds] = await Promise.all([
        api.availablePhp(),
        api.appStatus(),
        api.listEditors(),
      ]);
      setCatalog(cat);
      setGlobalVersion(status.globalPhpVersion);
      setSitesDir(status.sitesDir);
      setSiteCount(status.sites.length);
      setEditors(eds);
      setEditor(status.editor);
      // If the saved editor isn't a detected one, treat it as custom.
      setPicker(status.editor && !eds.includes(status.editor) ? CUSTOM : status.editor);
      if (status.editor && !eds.includes(status.editor)) setCustomEditor(status.editor);
    } catch (e) {
      setError(String(e));
    }
  };

  const changeSitesDir = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Choose sites folder" });
    if (typeof dir !== "string" || dir === sitesDir) return;
    const ok =
      siteCount === 0 ||
      (await ask(
        `Move ${siteCount} site${siteCount === 1 ? "" : "s"} to:\n${dir}\n\nAll site files move there and sites stop during the move.`,
        { title: "Change sites folder", kind: "warning" }
      ));
    if (!ok) return;
    setMovingSites(true);
    setError(null);
    setProgress(null);
    try {
      const status = await api.setSitesDir(dir);
      setSitesDir(status.sitesDir);
    } catch (e) {
      setError(String(e));
    } finally {
      setMovingSites(false);
      setProgress(null);
    }
  };

  useEffect(() => {
    load();
    const un = listen<SetupProgress>("setup-progress", (e) => setProgress(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  const saveEditor = async (val: string) => {
    try {
      await api.setEditor(val);
      setEditor(val);
    } catch (e) {
      setError(String(e));
    }
  };

  const onPick = (val: string) => {
    setPicker(val);
    if (val === CUSTOM) return; // wait for the custom input + Save
    saveEditor(val);
  };

  const install = async (minor: string) => {
    setBusyMinor(minor);
    setError(null);
    setProgress(null);
    try {
      await api.installPhp(minor);
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyMinor(null);
      setProgress(null);
    }
  };

  const makeDefault = async (minor: string) => {
    setBusyMinor(minor);
    setError(null);
    try {
      const v = await api.setGlobalPhp(minor);
      setGlobalVersion(v);
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyMinor(null);
      setProgress(null);
    }
  };

  return (
    <Modal title="Settings" onClose={onClose}>
      <div className="tabs">
        <button
          className={`tab ${tab === "php" ? "active" : ""}`}
          onClick={() => setTab("php")}
        >
          PHP Manager
        </button>
        <button
          className={`tab ${tab === "editor" ? "active" : ""}`}
          onClick={() => setTab("editor")}
        >
          Editor
        </button>
        <button
          className={`tab ${tab === "location" ? "active" : ""}`}
          onClick={() => setTab("location")}
        >
          Location
        </button>
      </div>

      {error && <div className="banner error">{error}</div>}

      {/* ── Location ───────────────────────────────────── */}
      <div className="field" hidden={tab !== "location"}>
        <span>Sites location</span>
        <div className="row-inline">
          <input readOnly value={sitesDir} title={sitesDir} />
          <button
            className="btn tiny"
            disabled={movingSites}
            onClick={changeSitesDir}
          >
            {movingSites ? "Moving…" : "Change…"}
          </button>
        </div>
        {movingSites && progress ? (
          <small className="muted">{progress.message}</small>
        ) : (
          <small className="muted">
            Where site folders live. Point this at a Drive/Dropbox folder to
            sync your sites across machines. Changing it moves every site.
          </small>
        )}
        <div className="row-inline">
          <button
            className="btn tiny"
            disabled={movingSites}
            onClick={async () => {
              const s = await api.rescanSites();
              setSiteCount(s.sites.length);
            }}
          >
            Scan folder for sites
          </button>
          <small className="muted">
            Imports any FrontPress sites found in this folder (e.g. synced from
            another machine).
          </small>
        </div>
      </div>

      {/* ── Editor ─────────────────────────────────────── */}
      <div className="field" hidden={tab !== "editor"}>
        <span>Editor</span>
        <select value={picker} onChange={(e) => onPick(e.target.value)}>
          <option value="">None</option>
          {editors.map((e) => (
            <option key={e} value={e}>
              {e}
            </option>
          ))}
          <option value={CUSTOM}>Other…</option>
        </select>
        {picker === CUSTOM && (
          <div className="row-inline">
            <input
              placeholder={
                "App name (e.g. Visual Studio Code) or command (e.g. code)"
              }
              value={customEditor}
              onChange={(e) => setCustomEditor(e.target.value)}
            />
            <button
              className="btn tiny primary"
              disabled={!customEditor.trim()}
              onClick={() => saveEditor(customEditor.trim())}
            >
              Save
            </button>
          </div>
        )}
        <small className="muted">
          Used by “Open in editor” on each site.
          {editor ? ` Currently: ${editor}.` : ""}
          {editors.length === 0
            ? " No editors detected — type a command (e.g. code, subl)."
            : ""}
        </small>
      </div>

      {/* ── PHP runtimes ───────────────────────────────── */}
      <div className="field" hidden={tab !== "php"}>
        <span>PHP runtimes</span>
        <small className="muted">
          Static PHP builds for {catalog?.arch ?? "your machine"}. Minimum:{" "}
          PHP {catalog?.minPhp ?? "8.1"}. The default is used for new sites
          unless you pick a per-site version.
        </small>
        <div className="php-table">
          {!catalog && <div className="muted">Loading available versions…</div>}
          {catalog?.options.map((o) => {
            const isDefault = globalVersion === o.latest;
            const busy = busyMinor === o.minor;
            return (
              <div key={o.minor} className="php-row">
                <div className="php-ver">
                  <strong>PHP {o.minor}</strong>
                  <span className="muted">latest {o.latest}</span>
                  {isDefault && <span className="pill accent">default</span>}
                  {o.installed && !isDefault && <span className="pill">installed</span>}
                </div>
                <div className="php-row-actions">
                  {busy && progress ? (
                    <span className="muted">
                      {progress.message}
                      {progress.pct != null ? ` ${Math.round(progress.pct)}%` : ""}
                    </span>
                  ) : o.installed ? (
                    <button
                      className="btn tiny"
                      disabled={isDefault || busy}
                      onClick={() => makeDefault(o.minor)}
                    >
                      {isDefault ? "Default" : "Make default"}
                    </button>
                  ) : (
                    <button
                      className="btn tiny primary"
                      disabled={busy}
                      onClick={() => install(o.minor)}
                    >
                      Download
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </Modal>
  );
}
