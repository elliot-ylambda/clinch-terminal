import { useMemo, useState, type ReactNode } from "react";

import { Icon, type IconName } from "./icons";

// One Settings surface with one nav. Today Clinch has two (a sidebar-less "Clinch Settings" page
// and the inherited terminal settings); this merges them. Rows are data so search can index them.

type Control =
  | { kind: "switch"; on: boolean }
  | { kind: "select"; value: string }
  | { kind: "segmented"; options: string[]; value: string }
  | { kind: "slider"; value: number }
  | { kind: "number"; value: string; unit?: string }
  | { kind: "button"; label: string; primary?: boolean }
  | { kind: "status"; text: string; tone: "ok" | "warn" };

interface Row {
  label: string;
  help?: string;
  control: Control;
}
interface Group {
  title: string;
  rows?: Row[];
  custom?: "themes" | "remote" | "keybindings" | "buttons" | "dircolors" | "regex" | "subshells" | "about" | "computer";
}
interface Page {
  id: string;
  name: string;
  icon: IconName;
  lede?: string;
  groups: Group[];
}

const sw = (on: boolean): Control => ({ kind: "switch", on });
const sel = (value: string): Control => ({ kind: "select", value });

export const PAGES: Page[] = [
  {
    id: "general",
    name: "General",
    icon: "gear",
    lede: "How Clinch starts, updates, and talks to other tools.",
    groups: [
      {
        title: "Projects",
        rows: [
          { label: "Create new tabs in Git worktrees", help: "Each new agent tab gets its own worktree so sessions never collide.", control: sw(true) },
          { label: "Startup shell", control: sel("zsh (default)") },
          { label: "New tabs open in", control: sel("Project root") },
        ],
      },
      {
        title: "Clinch CLI",
        rows: [
          { label: "Allow local control", help: "Lets agents and scripts on this Mac inspect and organize Clinch through the clinch CLI.", control: sw(true) },
          { label: "Use the Clinch CLI in other terminals", control: { kind: "status", text: "Installed", tone: "ok" } },
        ],
      },
      {
        title: "Updates",
        rows: [
          { label: "Check for updates automatically", control: sw(true) },
          { label: "Clinch v0.2026.10.28.1708", help: "A newer version is available: v0.2026.10.31.0912.", control: { kind: "button", label: "Update and restart", primary: true } },
        ],
      },
    ],
  },
  {
    id: "agents",
    name: "Agents",
    icon: "claude",
    lede: "Claude Code and Codex sessions, the coordinator, and the footer buttons.",
    groups: [
      {
        title: "Sessions",
        rows: [
          { label: "Capture Claude Code and Codex sessions", help: "Restores every agent conversation after Clinch restarts or updates.", control: sw(true) },
          { label: "Show status on tabs", help: "Working, needs you, done, and rate limited.", control: sw(true) },
          { label: "Continue automatically after a rate limit", help: "Sends “continue” when the limit resets. You can still turn it off per session.", control: sw(false) },
        ],
      },
      {
        title: "Coordinator",
        rows: [
          { label: "Pin a coordinator in every project", help: "A session at the top of each project that watches the others and asks before it ships or closes anything.", control: sw(true) },
          { label: "Coordinator runs", control: { kind: "segmented", options: ["Claude Code", "Codex"], value: "Claude Code" } },
        ],
      },
      { title: "Footer buttons", custom: "buttons" },
      {
        title: "Usage",
        rows: [{ label: "Show live plan limits in the title bar", help: "Reads your Claude and Codex plan usage. Nothing leaves this Mac.", control: sw(true) }],
      },
    ],
  },
  {
    id: "remote",
    name: "Remote Control",
    icon: "phone",
    lede: "Use your Claude and Codex sessions from your phone over Tailscale.",
    groups: [{ title: "Setup", custom: "remote" }],
  },
  {
    id: "appearance",
    name: "Appearance",
    icon: "palette",
    groups: [
      { title: "Theme", custom: "themes" },
      {
        title: "Text",
        rows: [
          { label: "Interface font", control: sel("Inter") },
          { label: "Terminal font", control: sel("JetBrains Mono") },
          { label: "Font size", control: { kind: "number", value: "13", unit: "pt" } },
          { label: "Line height", control: { kind: "number", value: "1.7" } },
          { label: "Font weight", control: sel("Regular") },
          { label: "Ligatures", control: sw(false) },
          { label: "Thin strokes", control: sel("On Retina displays") },
          { label: "Minimum contrast", control: sel("Only named colors") },
        ],
      },
      {
        title: "Window",
        rows: [
          { label: "Opacity", control: { kind: "slider", value: 100 } },
          { label: "Blur", control: { kind: "slider", value: 0 } },
          { label: "Zoom", control: sel("100%") },
          { label: "Open new windows at a fixed size", control: sw(false) },
          { label: "App icon", control: sel("Clinch") },
          { label: "Show Clinch in the Dock", control: sw(true) },
        ],
      },
      {
        title: "Cursor",
        rows: [
          { label: "Cursor shape", control: { kind: "segmented", options: ["Bar", "Block", "Underline"], value: "Block" } },
          { label: "Blink", control: sw(true) },
        ],
      },
      {
        title: "Input",
        rows: [
          { label: "Prompt", control: { kind: "segmented", options: ["Clinch", "Shell (PS1)"], value: "Clinch" } },
          { label: "Input position", control: sel("Pinned to the bottom") },
        ],
      },
      {
        title: "Sidebar",
        rows: [
          { label: "Row density", control: { kind: "segmented", options: ["Comfortable", "Compact"], value: "Comfortable" } },
          { label: "Show the branch", control: { kind: "segmented", options: ["Selected tab", "Every tab", "Never"], value: "Selected tab" } },
          { label: "Show tab details when you pause on a tab", help: "Branch, folder, pull request, and the last prompt.", control: sw(true) },
          { label: "Show pull request and diff badges on rows", control: sw(false) },
        ],
      },
      {
        title: "Title bar",
        rows: [
          { label: "File explorer button", control: sw(true) },
          { label: "Skills button", control: sw(true) },
          { label: "Reopen conversation button", control: sw(true) },
          { label: "Plan usage", control: { kind: "segmented", options: ["Bars", "Percent only", "Hidden"], value: "Bars" } },
        ],
      },
      { title: "Tab colors by folder", custom: "dircolors" },
    ],
  },
  {
    id: "terminal",
    name: "Terminal",
    icon: "terminal",
    lede: "Formerly Features. Behavior of shells, blocks, and input.",
    groups: [
      {
        title: "Session",
        rows: [
          { label: "Restore windows, tabs, and panes on startup", control: sw(true) },
          { label: "Confirm before closing a tab with a running process", control: sw(true) },
          { label: "Reopen closed tabs", help: "Keep closed tabs for 60 seconds so ⌘⇧T can bring them back.", control: sw(true) },
        ],
      },
      {
        title: "Blocks",
        rows: [
          { label: "Group commands and output into blocks", control: sw(true) },
          { label: "Maximum rows per block", control: { kind: "number", value: "100,000" } },
          { label: "Show block dividers", control: sw(false) },
        ],
      },
      {
        title: "Input and editing",
        rows: [
          { label: "Vim keybindings", control: sw(false) },
          { label: "Autocomplete quotes and brackets", control: sw(true) },
          { label: "Copy on select", control: sw(true) },
          { label: "Scroll speed", control: { kind: "number", value: "3", unit: "×" } },
        ],
      },
      {
        title: "Notifications",
        rows: [
          { label: "Desktop notifications from Clinch", help: "When an agent needs you, finishes, or a long command ends.", control: sw(true) },
          { label: "Show notifications for", control: sel("Needs you and finished") },
          { label: "Keep in-app notices for", control: { kind: "number", value: "8", unit: "s" } },
        ],
      },
      {
        title: "System",
        rows: [
          { label: "Global hotkey", help: "Show or hide Clinch from anywhere.", control: { kind: "button", label: "⌃ `" } },
          { label: "Open links in", control: sel("Default browser") },
          { label: "Open files in", control: sel("Cursor") },
        ],
      },
    ],
  },
  { id: "keys", name: "Keyboard shortcuts", icon: "keyboard", groups: [{ title: "Shortcuts", custom: "keybindings" }] },
  {
    id: "ssh",
    name: "Subshells and SSH",
    icon: "layers",
    lede: "Formerly Warpify. Bring blocks and the Clinch input into subshells and SSH sessions.",
    groups: [
      { title: "Subshells", custom: "subshells" },
      {
        title: "SSH",
        rows: [
          { label: "Enhance SSH sessions", control: sw(true) },
          { label: "Reuse an existing SSH ControlMaster", control: sw(false) },
        ],
      },
    ],
  },
  {
    id: "code",
    name: "Code",
    icon: "code",
    groups: [
      {
        title: "Editor and review",
        rows: [
          { label: "Open code in", control: sel("Clinch editor") },
          { label: "Open the review panel when an agent edits files", control: sw(true) },
          { label: "Show hidden files in the explorer", control: sw(false) },
          { label: "Format on save", control: sw(true) },
        ],
      },
      {
        title: "Language servers",
        rows: [
          { label: "rust-analyzer", help: "clinch-terminal · running", control: { kind: "button", label: "Restart" } },
          { label: "typescript-language-server", help: "magister-marketing · not installed", control: { kind: "button", label: "Install" } },
        ],
      },
    ],
  },
  {
    id: "privacy",
    name: "Privacy",
    icon: "lock",
    groups: [
      {
        title: "Secret redaction",
        rows: [
          { label: "Hide secrets in terminal output", help: "API keys, tokens, and passwords are blurred until you hover.", control: sw(true) },
          { label: "Redaction style", control: sel("Blur") },
        ],
      },
      { title: "Custom patterns", custom: "regex" },
    ],
  },
  { id: "computer", name: "Computer use", icon: "eye", groups: [{ title: "Permissions", custom: "computer" }] },
  { id: "about", name: "About", icon: "info", groups: [{ title: "", custom: "about" }] },
];

export function Settings({
  page,
  onPage,
  onClose,
  remoteState,
  onRemoteState,
  theme,
  onTheme,
}: {
  page: string;
  onPage: (id: string) => void;
  onClose: () => void;
  remoteState: RemoteState;
  onRemoteState: (s: RemoteState) => void;
  theme: "ink" | "paper";
  onTheme: (t: "ink" | "paper") => void;
}) {
  const [query, setQuery] = useState("");
  const q = query.trim().toLowerCase();
  const matches = useMemo(() => {
    if (!q) return null;
    const m = new Map<string, number>();
    for (const p of PAGES) {
      const n = p.groups
        .flatMap((g) => [g.title, ...(g.rows ?? []).flatMap((r) => [r.label, r.help ?? ""])])
        .filter((s) => s.toLowerCase().includes(q)).length;
      if (n > 0 || p.name.toLowerCase().includes(q)) m.set(p.id, n);
    }
    return m;
  }, [q]);
  const current = PAGES.find((p) => p.id === page) ?? PAGES[0];

  return (
    <div className="settings" aria-label="Settings" style={{ position: "relative" }}>
      <button className="icon-btn settings-close" aria-label="Close settings" title="Close  esc" onClick={onClose}>
        <Icon name="close" size={14} />
      </button>
      <nav className="settings-nav" aria-label="Settings pages">
        <label className="field">
          <Icon name="search" size={14} />
          <input placeholder="Search settings" aria-label="Search settings" value={query} onChange={(e) => setQuery(e.target.value)} />
        </label>
        {PAGES.filter((p) => !matches || matches.has(p.id)).map((p) => (
          <button key={p.id} className="nav-item" aria-current={p.id === current.id ? "page" : undefined} onClick={() => onPage(p.id)}>
            <Icon name={p.icon} size={15} className="icon" />
            <span style={{ flex: 1 }}>{p.name}</span>
            {matches && (matches.get(p.id) ?? 0) > 0 && <span className="kbd">{matches.get(p.id)}</span>}
          </button>
        ))}
        {matches && matches.size === 0 && (
          <div className="menu-label" style={{ fontWeight: 400, lineHeight: 1.5 }}>
            No settings match “{query}”.
          </div>
        )}
      </nav>
      <div className="settings-page">
        <div className="settings-inner">
          <header>
            <h1>{current.name}</h1>
            {current.lede && <p className="lede">{current.lede}</p>}
          </header>
          {current.groups.map((g) => (
            <section className="group" key={g.title || "x"}>
              {g.title && <h2>{g.title}</h2>}
              {g.custom ? (
                <Custom kind={g.custom} remoteState={remoteState} onRemoteState={onRemoteState} theme={theme} onTheme={onTheme} />
              ) : (
                <div className="group-card">
                  {g.rows!.map((r) => (
                    <SettingRow key={r.label} row={r} highlight={!!q && (r.label + (r.help ?? "")).toLowerCase().includes(q)} />
                  ))}
                </div>
              )}
            </section>
          ))}
          <div className="settings-footer">
            <Icon name="file" size={15} />
            <span style={{ flex: 1 }}>
              Every setting is also in <code>~/.clinch/settings.toml</code>
            </span>
            <button className="btn ghost">Open settings file</button>
          </div>
        </div>
      </div>
    </div>
  );
}

function SettingRow({ row, highlight }: { row: Row; highlight: boolean }) {
  const [on, setOn] = useState(row.control.kind === "switch" ? row.control.on : false);
  const [seg, setSeg] = useState(row.control.kind === "segmented" ? row.control.value : "");
  const [val, setVal] = useState(row.control.kind === "slider" ? row.control.value : 0);
  const c = row.control;
  return (
    <div className="setting" style={highlight ? { boxShadow: "inset 0 0 0 1px var(--accent)", borderRadius: "var(--radius-lg)" } : undefined}>
      <div className="text">
        <span className="label">{row.label}</span>
        {row.help && <span className="help">{row.help}</span>}
      </div>
      {c.kind === "switch" && <button className="switch" role="switch" aria-checked={on} aria-label={row.label} onClick={() => setOn(!on)} />}
      {c.kind === "select" && (
        <button className="select" aria-haspopup="listbox" aria-label={row.label}>
          {c.value}
          <Icon name="chevron" size={13} strokeWidth={1.8} />
        </button>
      )}
      {c.kind === "segmented" && (
        <div className="segmented" role="group" aria-label={row.label}>
          {c.options.map((o) => (
            <button key={o} aria-pressed={seg === o} onClick={() => setSeg(o)}>
              {o}
            </button>
          ))}
        </div>
      )}
      {c.kind === "slider" && (
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <input className="slider" type="range" min={0} max={100} value={val} aria-label={row.label} onChange={(e) => setVal(+e.target.value)} />
          <span className="kbd" style={{ width: 34, textAlign: "right" }}>
            {val}%
          </span>
        </div>
      )}
      {c.kind === "number" && (
        <label className="field" style={{ width: 120 }}>
          <input defaultValue={c.value} aria-label={row.label} style={{ textAlign: "right" }} />
          {c.unit && <span className="kbd">{c.unit}</span>}
        </label>
      )}
      {c.kind === "button" && <button className={`btn${c.primary ? " primary" : ""}`}>{c.label}</button>}
      {c.kind === "status" && (
        <span className="kbd" style={{ color: c.tone === "ok" ? "var(--status-working)" : "var(--status-needs)", display: "flex", gap: 6, alignItems: "center" }}>
          <Icon name={c.tone === "ok" ? "check" : "warning"} size={13} />
          {c.text}
        </span>
      )}
    </div>
  );
}

export type RemoteState = "off" | "tailscale" | "pairing" | "approve" | "paired" | "error";

function Custom({
  kind,
  remoteState,
  onRemoteState,
  theme,
  onTheme,
}: {
  kind: NonNullable<Group["custom"]>;
  remoteState: RemoteState;
  onRemoteState: (s: RemoteState) => void;
  theme: "ink" | "paper";
  onTheme: (t: "ink" | "paper") => void;
}) {
  switch (kind) {
    case "themes":
      return <ThemeGrid theme={theme} onTheme={onTheme} />;
    case "remote":
      return <RemoteSetup state={remoteState} onState={onRemoteState} />;
    case "keybindings":
      return <Keybindings />;
    case "buttons":
      return <ButtonsEditor />;
    case "dircolors":
      return (
        <div className="group-card">
          {[
            ["~/projects/magister-marketing", "var(--section-blue)"],
            ["~/projects/clinch-terminal", "var(--section-lime)"],
          ].map(([dir, color]) => (
            <div className="setting" key={dir}>
              <span className="section-dot" style={{ background: color, width: 10, height: 10 }} />
              <div className="text">
                <span className="label" style={{ fontFamily: "var(--font-mono)", fontSize: "var(--text-sm)" }}>
                  {dir}
                </span>
              </div>
              <div style={{ display: "flex", gap: 6 }}>
                {["lime", "coral", "blue", "amber", "violet", "teal"].map((c) => (
                  <button
                    key={c}
                    aria-label={c}
                    style={{
                      width: 16,
                      height: 16,
                      borderRadius: "50%",
                      border: 0,
                      background: `var(--section-${c})`,
                      boxShadow: color.includes(c) ? "0 0 0 2px var(--surface-chrome), 0 0 0 3.5px var(--text-1)" : "none",
                    }}
                  />
                ))}
              </div>
              <button className="icon-btn sm" aria-label={`Remove ${dir}`}>
                <Icon name="trash" size={13} />
              </button>
            </div>
          ))}
          <div className="setting">
            <button className="btn ghost">
              <Icon name="plus" size={13} />
              Add folder
            </button>
          </div>
        </div>
      );
    case "regex":
    case "subshells":
      return (
        <div className="group-card">
          {(kind === "regex" ? ["sk-ant-[A-Za-z0-9_-]{20,}", "shpat_[a-f0-9]{32}"] : ["nix-shell", "poetry shell"]).map((r) => (
            <div className="setting" key={r}>
              <div className="text">
                <span className="label" style={{ fontFamily: "var(--font-mono)", fontSize: "var(--text-sm)" }}>
                  {r}
                </span>
              </div>
              <button className="icon-btn sm" aria-label={`Remove ${r}`}>
                <Icon name="trash" size={13} />
              </button>
            </div>
          ))}
          <div className="setting">
            <label className="field" style={{ flex: 1 }}>
              <input placeholder={kind === "regex" ? "Add a pattern, like ghp_[A-Za-z0-9]{36}" : "Add a command that starts a subshell"} />
            </label>
            <button className="btn">Add</button>
          </div>
        </div>
      );
    case "computer":
      return (
        <div className="group-card">
          <div className="setting">
            <div className="text">
              <span className="label">Screen Recording</span>
              <span className="help">Lets agents see the screen when you ask them to use your computer.</span>
            </div>
            <span className="kbd" style={{ color: "var(--status-working)", display: "flex", gap: 6 }}>
              <Icon name="check" size={13} /> Allowed
            </span>
          </div>
          <div className="setting">
            <div className="text">
              <span className="label">Accessibility</span>
              <span className="help">Lets agents click and type in other apps.</span>
            </div>
            <button className="btn">Open System Settings</button>
          </div>
        </div>
      );
    case "about":
      return (
        <div className="group-card" style={{ alignItems: "center", padding: "40px 24px", gap: 10, textAlign: "center" }}>
          <div
            style={{
              width: 64,
              height: 64,
              borderRadius: 16,
              background: "radial-gradient(circle at 35% 30%, #d8ff8a, #6f9a1f 70%)",
              boxShadow: "0 10px 30px rgb(197 240 107 / 0.25)",
            }}
            aria-hidden="true"
          />
          <div style={{ fontSize: "var(--text-xl)", fontWeight: 600 }}>Clinch</div>
          <div className="kbd" style={{ display: "flex", gap: 6, alignItems: "center" }}>
            v0.2026.10.28.1708 · Apple Silicon
            <button className="icon-btn sm" aria-label="Copy version">
              <Icon name="copy" size={12} />
            </button>
          </div>
          <div className="help" style={{ color: "var(--text-3)", fontSize: "var(--text-sm)", lineHeight: 1.6 }}>
            Based on the Warp terminal. Licensed under AGPL-3.0 and MIT.
            <br />
            <a href="#" style={{ color: "var(--text-2)" }}>
              Open-source notices
            </a>
          </div>
        </div>
      );
  }
}

const THEMES = [
  { id: "ink", name: "Ink", bg: "#0d0e11", fg: "#d3d6dd", a: "#c5f06b", b: "#6fa8ff", c: "#f08080" },
  { id: "paper", name: "Paper", bg: "#ffffff", fg: "#2a2c31", a: "#4f7d0f", b: "#2a64c9", c: "#c23b3b" },
  { id: "dusk", name: "Dusk", bg: "#14121c", fg: "#ddd6e6", a: "#f2a7bb", b: "#9db8f5", c: "#f3c27a" },
  { id: "mocha", name: "Catppuccin Mocha", bg: "#1e1e2e", fg: "#cdd6f4", a: "#a6e3a1", b: "#89b4fa", c: "#f38ba8" },
  { id: "rose", name: "Rosé Pine", bg: "#191724", fg: "#e0def4", a: "#9ccfd8", b: "#c4a7e7", c: "#eb6f92" },
  { id: "tokyo", name: "Tokyo Night", bg: "#1a1b26", fg: "#c0caf5", a: "#9ece6a", b: "#7aa2f7", c: "#f7768e" },
] as const;

function ThemeGrid({ theme, onTheme }: { theme: "ink" | "paper"; onTheme: (t: "ink" | "paper") => void }) {
  const [sync, setSync] = useState(false);
  return (
    <>
      <div className="group-card">
        <div className="setting">
          <div className="text">
            <span className="label">Match macOS light and dark mode</span>
            <span className="help">Uses Paper in light mode and Ink in dark mode.</span>
          </div>
          <button className="switch" role="switch" aria-checked={sync} aria-label="Match macOS" onClick={() => setSync(!sync)} />
        </div>
      </div>
      <div className="theme-grid" role="radiogroup" aria-label="Theme">
        {THEMES.map((t) => (
          <button
            key={t.id}
            className="theme-card"
            role="radio"
            aria-checked={t.id === theme}
            onClick={() => (t.id === "ink" || t.id === "paper") && onTheme(t.id)}
          >
            <span className="theme-swatch" style={{ background: t.bg, color: t.fg }}>
              <span>
                <span style={{ color: t.a }}>❯</span> git status
              </span>
              <span style={{ display: "flex", gap: 4 }}>
                {[t.a, t.b, t.c].map((c) => (
                  <span key={c} style={{ width: 22, height: 6, borderRadius: 3, background: c }} />
                ))}
              </span>
              <span style={{ opacity: 0.6 }}>main · 3 files changed</span>
            </span>
            {t.name}
          </button>
        ))}
        <button className="theme-card">
          <span
            className="theme-swatch"
            style={{ background: "var(--surface-field)", alignItems: "center", justifyContent: "center", color: "var(--text-2)", fontFamily: "var(--font-ui)", fontSize: 12 }}
          >
            <Icon name="plus" size={18} />
            New theme
          </span>
          Create your own
        </button>
      </div>
    </>
  );
}

function Step({ n, done, title, children }: { n: number; done: boolean; title: string; children: ReactNode }) {
  return (
    <div className="setting" style={{ alignItems: "flex-start" }}>
      <span
        className="badge"
        style={{
          width: 24,
          height: 24,
          borderRadius: 12,
          background: done ? "var(--status-working-soft)" : "var(--surface-field)",
          color: done ? "var(--status-working)" : "var(--text-2)",
          flex: "none",
        }}
      >
        {done ? <Icon name="check" size={13} strokeWidth={2.2} /> : n}
      </span>
      <div className="text" style={{ gap: 10 }}>
        <span className="label">{title}</span>
        {children}
      </div>
    </div>
  );
}

function RemoteSetup({ state, onState }: { state: RemoteState; onState: (s: RemoteState) => void }) {
  const on = state !== "off";
  const tailscale = ["pairing", "approve", "paired", "error"].includes(state);
  return (
    <div className="group-card">
      {state === "error" && (
        <div className="banner warn" style={{ margin: "var(--space-6) var(--space-6) 0" }} role="alert">
          <Icon name="warning" size={15} />
          <span className="grow">Tailscale stopped responding. Remote Control is paused until it’s back.</span>
          <button className="btn" style={{ height: 26 }} onClick={() => onState("paired")}>
            Retry
          </button>
        </div>
      )}
      <Step n={1} done={on} title="Turn on Remote Control">
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button className="switch" role="switch" aria-checked={on} aria-label="Remote Control" onClick={() => onState(on ? "off" : "tailscale")} />
          <span className="help">Only devices you pair can connect, and only through your tailnet.</span>
        </div>
      </Step>
      <Step n={2} done={tailscale} title="Tailscale on this Mac">
        {tailscale ? (
          <span className="help">Connected as elliots-macbook.tail9c2e.ts.net</span>
        ) : (
          <div style={{ display: "flex", gap: 8 }}>
            <button className="btn" disabled={!on} onClick={() => onState("pairing")}>
              Open Tailscale
            </button>
            <button className="btn ghost" disabled={!on}>
              <Icon name="copy" size={13} />
              Copy link
            </button>
          </div>
        )}
      </Step>
      <Step n={3} done={state === "paired"} title="Pair a phone">
        {state === "pairing" && (
          <div className="pair-row">
            <QR />
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <span className="help">Scan with your phone’s camera. The code expires in 4:52.</span>
              <div style={{ display: "flex", gap: 8 }}>
                <button className="btn" onClick={() => onState("approve")}>
                  Simulate scan
                </button>
                <button className="btn ghost" onClick={() => onState("tailscale")}>
                  Cancel
                </button>
              </div>
            </div>
          </div>
        )}
        {state === "approve" && (
          <div className="device" style={{ padding: "var(--space-5)", borderRadius: "var(--radius-md)", background: "var(--surface-field)" }}>
            <Icon name="phone" size={18} />
            <div className="text" style={{ flex: 1 }}>
              <span className="label">Elliot’s iPhone wants to connect</span>
              <span className="help">Safari · iOS 26.1 · 100.84.12.7</span>
            </div>
            <button className="btn ghost" onClick={() => onState("pairing")}>
              Reject
            </button>
            <button className="btn primary" onClick={() => onState("paired")}>
              Approve
            </button>
          </div>
        )}
        {(state === "paired" || state === "error") && (
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            <div className="device">
              <Icon name="phone" size={16} />
              <span style={{ flex: 1 }}>Elliot’s iPhone</span>
              <span className="kbd" style={{ color: "var(--status-working)" }}>
                Connected now
              </span>
              <button className="btn ghost" style={{ height: 26 }}>
                Remove
              </button>
            </div>
            <button className="btn ghost" style={{ alignSelf: "flex-start", paddingLeft: 0 }} onClick={() => onState("pairing")}>
              <Icon name="plus" size={13} />
              Pair another phone
            </button>
          </div>
        )}
        {(state === "off" || state === "tailscale") && (
          <button className="btn" disabled={!tailscale}>
            Pair a phone
          </button>
        )}
      </Step>
    </div>
  );
}

function QR() {
  // A deterministic fake QR so the pairing state has the right visual weight.
  const cells: ReactNode[] = [];
  let seed = 7;
  for (let y = 0; y < 21; y++)
    for (let x = 0; x < 21; x++) {
      seed = (seed * 9301 + 49297) % 233280;
      const finder = (x < 7 && y < 7) || (x > 13 && y < 7) || (x < 7 && y > 13);
      const ring = finder && (x % 14 === 0 || y % 14 === 0 || x % 14 === 6 || y % 14 === 6 || (x % 14 > 1 && x % 14 < 5 && y % 14 > 1 && y % 14 < 5));
      if (finder ? ring : seed / 233280 > 0.52) cells.push(<rect key={`${x}-${y}`} x={x} y={y} width="1" height="1" />);
    }
  return (
    <svg className="qr" viewBox="0 0 21 21" role="img" aria-label="Pairing code">
      <g fill="#121318">{cells}</g>
    </svg>
  );
}

function Keybindings() {
  const [recording, setRecording] = useState<string | null>(null);
  const rows: [string, string[]][] = [
    ["Command palette", ["⌘", "P"]],
    ["Search tabs", ["⌘", "⇧", "F"]],
    ["New tab", ["⌘", "T"]],
    ["New Claude session", ["⌘", "⇧", "C"]],
    ["Next tab that needs you", ["⌘", "⌥", "A"]],
    ["Reopen agent conversation", ["⌘", "⇧", "O"]],
    ["Split pane right", ["⌘", "D"]],
    ["Find in terminal", ["⌘", "F"]],
    ["Toggle sidebar", ["⌘", "B"]],
    ["Settings", ["⌘", ","]],
  ];
  return (
    <>
      <label className="field">
        <Icon name="search" size={14} />
        <input placeholder="Search shortcuts" aria-label="Search shortcuts" />
      </label>
      <div className="group-card kb-table">
        {rows.map(([name, keys]) => (
          <div className="kb-row" key={name}>
            <span className="name">{name}</span>
            {recording === name ? (
              <>
                <span className="keycap recording">
                  <span>Press keys…</span>
                </span>
                <button className="btn ghost" style={{ height: 26 }} onClick={() => setRecording(null)}>
                  Cancel
                </button>
                <button className="btn ghost" style={{ height: 26 }}>
                  Reset
                </button>
              </>
            ) : (
              <button className="keycap" style={{ border: 0, background: "transparent", padding: 0 }} onClick={() => setRecording(name)} aria-label={`Change shortcut for ${name}`}>
                {keys.map((k) => (
                  <span key={k}>{k}</span>
                ))}
              </button>
            )}
          </div>
        ))}
      </div>
    </>
  );
}

function ButtonsEditor() {
  const items: [string, string, boolean][] = [
    ["Fork", "Fork this session into a new tab", true],
    ["Compact", "/compact", true],
    ["Continue", "continue", true],
    ["LGTM", "LGTM, ship it", true],
    ["Review PR", "Review the open PR for this branch…", true],
    ["Push to main", "Rebase on main, run the checks, push…", true],
    ["Create a plan", "Write a plan before changing code…", false],
    ["Make no mistakes", "Double-check every claim against…", false],
  ];
  return (
    <div className="group-card" style={{ padding: "var(--space-4)" }}>
      <div className="help" style={{ padding: "var(--space-3) var(--space-4) var(--space-4)", color: "var(--text-3)", fontSize: "var(--text-sm)" }}>
        The first six show in the footer. The rest live under More. Drag to reorder.
      </div>
      <div className="button-list">
        {items.map(([name, text, shown], i) => (
          <div className="button-item" key={name} style={i === 6 ? { boxShadow: "0 -1px 0 var(--hairline)", borderRadius: 0, marginTop: 4, paddingTop: 4 } : undefined}>
            <Icon name="grip" size={14} className="grip" />
            <div className="what">
              <span>{name}</span>
              <span className="sub">{text}</span>
            </div>
            <span className="kbd">{shown ? "Footer" : "More"}</span>
            <button className="icon-btn sm" aria-label={`Edit ${name}`}>
              <Icon name="pencil" size={12} />
            </button>
          </div>
        ))}
      </div>
      <button className="btn ghost" style={{ marginTop: 6 }}>
        <Icon name="plus" size={13} />
        Add button
      </button>
    </div>
  );
}
