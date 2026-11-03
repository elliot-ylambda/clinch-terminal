import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { PROJECTS, allTabs, type SectionColor, type Tab } from "./data";
import { Icon, type IconName } from "./icons";

/* ---------- Menus ---------- */

export type MenuEntry =
  | { label: string; icon?: IconName; kbd?: string; danger?: boolean; onSelect?: () => void; children?: MenuEntry[] }
  | { sep: true }
  | { heading: string };

export function Menu({ x, y, entries, onClose }: { x: number; y: number; entries: MenuEntry[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  useEffect(() => {
    const el = ref.current;
    const host = el?.offsetParent as HTMLElement | null;
    if (!el || !host) return;
    const hr = host.getBoundingClientRect();
    let left = x - hr.left;
    let top = y - hr.top;
    if (left + el.offsetWidth > hr.width - 8) left = Math.max(8, x - hr.left - el.offsetWidth);
    if (top + el.offsetHeight > hr.height - 8) top = Math.max(8, hr.height - el.offsetHeight - 8);
    setPos({ left, top });
    el.querySelector<HTMLButtonElement>(".menu-item")?.focus();
  }, [x, y]);

  useEffect(() => {
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && onClose();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="menu popover"
      role="menu"
      style={pos}
      onKeyDown={(e) => {
        const items = [...(ref.current?.querySelectorAll<HTMLButtonElement>(".menu-item") ?? [])];
        const i = items.indexOf(document.activeElement as HTMLButtonElement);
        if (e.key === "ArrowDown") items[(i + 1) % items.length]?.focus();
        if (e.key === "ArrowUp") items[(i - 1 + items.length) % items.length]?.focus();
      }}
    >
      <MenuEntries entries={entries} onClose={onClose} />
    </div>
  );
}

function MenuEntries({ entries, onClose }: { entries: MenuEntry[]; onClose: () => void }) {
  const [sub, setSub] = useState<number | null>(null);
  return (
    <>
      {entries.map((e, i) =>
        "sep" in e ? (
          <div className="menu-sep" key={i} />
        ) : "heading" in e ? (
          <div className="menu-label" key={i}>
            {e.heading}
          </div>
        ) : e.children ? (
          <div className="submenu-anchor" key={i} onMouseEnter={() => setSub(i)} onMouseLeave={() => setSub(null)}>
            <button
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={sub === i}
              className="menu-item"
              style={{ width: "100%" }}
              onClick={() => setSub(i)}
              onKeyDown={(k) => k.key === "ArrowRight" && setSub(i)}
            >
              {e.icon ? <Icon name={e.icon} size={14} className="icon" /> : <span style={{ width: 14 }} />}
              {e.label}
              <Icon name="chevronRight" size={12} className="chev" strokeWidth={1.8} />
            </button>
            {sub === i && (
              <div className="menu popover" role="menu" style={{ left: "calc(100% + 2px)", top: -6 }}>
                <MenuEntries entries={e.children} onClose={onClose} />
              </div>
            )}
          </div>
        ) : (
          <button
            key={i}
            role="menuitem"
            className={`menu-item${e.danger ? " danger" : ""}`}
            onMouseEnter={() => setSub(null)}
            onClick={() => {
              e.onSelect?.();
              onClose();
            }}
          >
            {e.icon ? <Icon name={e.icon} size={14} className="icon" /> : <span style={{ width: 14 }} />}
            {e.label}
            {e.kbd && <span className="kbd">{e.kbd}</span>}
          </button>
        ),
      )}
    </>
  );
}

/* ---------- Command palette and conversation finder ---------- */

type Scope = "all" | "tabs" | "commands" | "conversations";
interface Item {
  id: string;
  group: string;
  label: string;
  sub?: string;
  icon: IconName;
  kbd?: string;
  status?: Tab["status"];
}

const COMMANDS: Item[] = [
  { id: "c1", group: "Commands", label: "New Claude Code session", icon: "claude", kbd: "⌘⇧C" },
  { id: "c2", group: "Commands", label: "New Codex session", icon: "codex" },
  { id: "c3", group: "Commands", label: "New terminal tab", icon: "terminal", kbd: "⌘T" },
  { id: "c4", group: "Commands", label: "Split pane right", icon: "splitRight", kbd: "⌘D" },
  { id: "c5", group: "Commands", label: "Next tab that needs you", icon: "needs", kbd: "⌘⌥A" },
  { id: "c6", group: "Commands", label: "Open settings", icon: "gear", kbd: "⌘," },
  { id: "c7", group: "Commands", label: "Pair a phone for Remote Control", icon: "phone" },
  { id: "c8", group: "Commands", label: "Switch theme…", icon: "palette" },
];

const CONVERSATIONS: Item[] = [
  { id: "v1", group: "Closed conversations", label: "Shopify OAuth review readiness", sub: "magister-marketing · Codex · 2 days ago", icon: "codex" },
  { id: "v2", group: "Closed conversations", label: "Remote Control garble round 5", sub: "clinch-terminal · Claude Code · Aug 6", icon: "claude" },
  { id: "v3", group: "Closed conversations", label: "Keychain prompt structural fix", sub: "clinch-terminal · Claude Code · Jul 16", icon: "claude" },
  { id: "v4", group: "Closed conversations", label: "Pricing page copy pass", sub: "magister-marketing · Claude Code · Oct 12", icon: "claude" },
];

export function Palette({
  initialScope,
  onClose,
  onPickTab,
}: {
  initialScope: Scope;
  onClose: () => void;
  onPickTab: (projectId: string, tabId: string) => void;
}) {
  const [scope, setScope] = useState<Scope>(initialScope);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const [agent, setAgent] = useState<"any" | "claude" | "codex">("any");
  const [where, setWhere] = useState<"project" | "all" | "bookmarked">("project");

  const items = useMemo(() => {
    const tabs: (Item & { project: string })[] = PROJECTS.flatMap((p) =>
      allTabs(p).map((t) => ({
        id: t.id,
        project: p.id,
        group: "Open tabs",
        label: t.title,
        sub: `${p.name} · ${t.kind === "shell" ? t.cwd : t.branch}`,
        icon: (t.status === "needs" ? "needs" : t.kind === "shell" ? "terminal" : t.kind) as IconName,
        status: t.status,
      })),
    );
    const pool: Item[] =
      scope === "tabs" ? tabs : scope === "commands" ? COMMANDS : scope === "conversations" ? CONVERSATIONS : [...tabs.slice(0, 6), ...COMMANDS, ...CONVERSATIONS.slice(0, 2)];
    const needle = q.toLowerCase();
    return pool.filter(
      (i) =>
        (!needle || i.label.toLowerCase().includes(needle) || i.sub?.toLowerCase().includes(needle)) &&
        (scope !== "conversations" || agent === "any" || i.icon === agent),
    );
  }, [scope, q, agent]);

  useEffect(() => setSel(0), [scope, q, agent]);

  const pick = (i: Item) => {
    const tab = i as Item & { project?: string };
    if (tab.project) onPickTab(tab.project, tab.id);
    onClose();
  };

  let lastGroup = "";
  return (
    <Scrim onClose={onClose}>
      <div className="palette popover" role="dialog" aria-label={scope === "conversations" ? "Reopen a conversation" : "Command palette"}>
        <div className="palette-input">
          <Icon name="search" size={17} />
          <input
            autoFocus
            placeholder={scope === "conversations" ? "Search closed conversations" : "Search tabs, commands, and conversations"}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") setSel((s) => Math.min(s + 1, items.length - 1));
              if (e.key === "ArrowUp") setSel((s) => Math.max(s - 1, 0));
              if (e.key === "Enter" && items[sel]) pick(items[sel]);
              if (e.key === "Escape") onClose();
            }}
          />
          <span className="kbd">esc</span>
        </div>
        <div className="palette-scopes" role="group" aria-label="Scope">
          {(["all", "tabs", "commands", "conversations"] as Scope[]).map((s) => (
            <button key={s} className="scope" aria-pressed={scope === s} onClick={() => setScope(s)}>
              {s === "all" ? "All" : s === "tabs" ? "Tabs" : s === "commands" ? "Commands" : "Conversations"}
            </button>
          ))}
        </div>
        {scope === "conversations" && (
          <div className="palette-scopes" role="group" aria-label="Filters" style={{ paddingTop: 2 }}>
            {(["project", "all", "bookmarked"] as const).map((w) => (
              <button key={w} className="scope" aria-pressed={where === w} onClick={() => setWhere(w)}>
                {w === "project" ? "This project" : w === "all" ? "All projects" : "Bookmarked"}
              </button>
            ))}
            <span style={{ width: 1, background: "var(--hairline)", margin: "4px 6px" }} />
            {(["any", "claude", "codex"] as const).map((a) => (
              <button key={a} className="scope" aria-pressed={agent === a} onClick={() => setAgent(a)}>
                {a === "any" ? "Any agent" : a === "claude" ? "Claude" : "Codex"}
              </button>
            ))}
          </div>
        )}
        <div className="palette-list" role="listbox">
          {items.length === 0 && <div className="palette-empty">Nothing matches “{q}”.</div>}
          {items.map((i, idx) => {
            const heading = i.group !== lastGroup ? <div className="palette-group">{i.group}</div> : null;
            lastGroup = i.group;
            return (
              <div key={i.id} style={{ display: "contents" }}>
                {heading}
                <button
                  role="option"
                  className="palette-item"
                  aria-selected={idx === sel}
                  onMouseMove={() => setSel(idx)}
                  onClick={() => pick(i)}
                >
                  <span className={`icon tab-glyph ${i.status ?? ""}`}>
                    <Icon name={i.icon} size={15} />
                  </span>
                  <span className="main-text">
                    <span>{i.label}</span>
                    {i.sub && <span className="sub">{i.sub}</span>}
                  </span>
                  {i.kbd && <span className="kbd">{i.kbd}</span>}
                </button>
              </div>
            );
          })}
        </div>
        <div className="palette-foot">
          <span>↑↓ to move</span>
          <span>↵ to open</span>
          <span>⌘↵ to open in a split</span>
        </div>
      </div>
    </Scrim>
  );
}

/* ---------- Dialogs ---------- */

export function Scrim({ children, onClose, center }: { children: ReactNode; onClose: () => void; center?: boolean }) {
  return (
    <div className={`scrim${center ? " center" : ""}`} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      {children}
    </div>
  );
}

export function ConfirmDialog({
  title,
  body,
  confirm,
  danger,
  checkbox,
  onClose,
}: {
  title: string;
  body: ReactNode;
  confirm: string;
  danger?: boolean;
  checkbox?: string;
  onClose: () => void;
}) {
  const [checked, setChecked] = useState(false);
  return (
    <Scrim onClose={onClose} center>
      <div className="dialog popover" role="alertdialog" aria-label={title} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <h2>{title}</h2>
        <p>{body}</p>
        {checkbox && (
          <label className="check-row">
            <button className="checkbox" role="checkbox" aria-checked={checked} onClick={() => setChecked(!checked)}>
              {checked && <Icon name="check" size={11} strokeWidth={2.2} />}
            </button>
            {checkbox}
          </label>
        )}
        <div className="dialog-actions">
          <button className="btn ghost" autoFocus onClick={onClose}>
            Cancel
          </button>
          <button className={`btn ${danger ? "danger" : "primary"}`} onClick={onClose}>
            {confirm}
          </button>
        </div>
      </div>
    </Scrim>
  );
}

const COLORS: SectionColor[] = ["lime", "coral", "blue", "amber", "violet", "teal", "pink", "gray"];

export function NewSectionDialog({ onClose, onCreate }: { onClose: () => void; onCreate: (name: string, color: SectionColor) => void }) {
  const [name, setName] = useState("");
  const [color, setColor] = useState<SectionColor>("violet");
  return (
    <Scrim onClose={onClose} center>
      <form
        className="dialog popover"
        role="dialog"
        aria-label="New section"
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim()) onCreate(name.trim(), color);
          onClose();
        }}
        onKeyDown={(e) => e.key === "Escape" && onClose()}
      >
        <h2>New section</h2>
        <label className="field">
          <input autoFocus placeholder="Name, like “Ready to ship”" value={name} onChange={(e) => setName(e.target.value)} aria-label="Section name" />
        </label>
        <div style={{ display: "flex", gap: 10 }} role="radiogroup" aria-label="Color">
          {COLORS.map((c) => (
            <button
              type="button"
              key={c}
              role="radio"
              aria-checked={color === c}
              aria-label={c}
              onClick={() => setColor(c)}
              style={{
                width: 22,
                height: 22,
                borderRadius: "50%",
                border: 0,
                background: `var(--section-${c})`,
                boxShadow: color === c ? "0 0 0 2px var(--surface-overlay), 0 0 0 4px var(--text-1)" : "none",
              }}
            />
          ))}
        </div>
        <div className="dialog-actions">
          <button type="button" className="btn ghost" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={!name.trim()}>
            Create section
          </button>
        </div>
      </form>
    </Scrim>
  );
}

/* ---------- Popovers anchored to the title bar ---------- */

export function UsagePopover({ x, y, onClose }: { x: number; y: number; onClose: () => void }) {
  return (
    <Anchored x={x} y={y} onClose={onClose} width={320}>
      <div className="menu-label">Plan usage</div>
      {[
        { name: "Claude · 5-hour", pct: 54, reset: "resets in 1h 50m" },
        { name: "Claude · weekly", pct: 86, reset: "resets in 5d", warn: true },
        { name: "Codex · weekly", pct: 100, reset: "resets in 5d · 3 sessions paused", max: true },
      ].map((m) => (
        <div key={m.name} style={{ padding: "8px 10px", display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ display: "flex", fontSize: "var(--text-sm)" }}>
            <span style={{ flex: 1, color: "var(--text-1)" }}>{m.name}</span>
            <span style={{ color: m.max ? "var(--status-limit)" : m.warn ? "var(--status-needs)" : "var(--text-2)" }}>{m.pct}%</span>
          </div>
          <span className="meter-bar" style={{ width: "100%", height: 6, borderRadius: 3 }}>
            <span style={{ width: `${m.pct}%`, background: m.max ? "var(--status-limit)" : m.warn ? "var(--status-needs)" : "var(--text-2)" }} />
          </span>
          <span className="kbd">{m.reset}</span>
        </div>
      ))}
      <div className="menu-sep" />
      <button className="menu-item">
        <Icon name="refresh" size={14} className="icon" />
        Refresh now
      </button>
    </Anchored>
  );
}

export function RemotePopover({ x, y, connected, onClose, onOpenSettings }: { x: number; y: number; connected: boolean; onClose: () => void; onOpenSettings: () => void }) {
  return (
    <Anchored x={x} y={y} onClose={onClose} width={300}>
      <div className="menu-label">Remote Control</div>
      {connected ? (
        <div className="device" style={{ padding: "8px 10px" }}>
          <span className="tab-glyph working">
            <Icon name="phone" size={15} />
          </span>
          <span style={{ flex: 1, display: "flex", flexDirection: "column" }}>
            <span>Elliot’s iPhone</span>
            <span className="kbd">Connected now · viewing magister-marketing</span>
          </span>
        </div>
      ) : (
        <div style={{ padding: "6px 10px 10px", color: "var(--text-2)", fontSize: "var(--text-sm)", lineHeight: 1.5 }}>
          Use your sessions from your phone. Pair once over Tailscale.
        </div>
      )}
      <div className="menu-sep" />
      <button
        className="menu-item"
        onClick={() => {
          onOpenSettings();
          onClose();
        }}
      >
        <Icon name={connected ? "gear" : "plus"} size={14} className="icon" />
        {connected ? "Remote Control settings" : "Pair a phone"}
      </button>
    </Anchored>
  );
}

function Anchored({ x, y, width, onClose, children }: { x: number; y: number; width: number; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  useEffect(() => {
    const host = ref.current?.offsetParent as HTMLElement | null;
    if (!host) return;
    const hr = host.getBoundingClientRect();
    setPos({ left: Math.max(8, x - hr.left - width), top: y - hr.top });
  }, [x, y, width]);
  useEffect(() => {
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && onClose();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc);
    };
  }, [onClose]);
  return (
    <div ref={ref} className="menu popover" style={{ width, ...(pos ?? { left: -9999, top: 0 }) }} role="dialog">
      {children}
    </div>
  );
}

/* ---------- Toasts ---------- */

export interface Toast {
  id: number;
  tone: "done" | "needs" | "info";
  title: string;
  body: string;
  actions?: string[];
}

export function Toasts({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  return (
    <div className="toasts" role="region" aria-label="Notifications">
      {toasts.map((t) => (
        <div className="toast" key={t.id} role="status">
          <span className={`tab-glyph ${t.tone === "info" ? "idle" : t.tone}`} style={{ marginTop: 1 }}>
            <Icon name={t.tone === "needs" ? "needs" : t.tone === "done" ? "check" : "info"} size={15} />
          </span>
          <span className="body">
            <strong>{t.title}</strong>
            <span>{t.body}</span>
            {t.actions && (
              <span className="actions">
                {t.actions.map((a, i) => (
                  <button key={a} className={`btn${i === 0 ? "" : " ghost"}`} style={{ height: 26 }} onClick={() => onDismiss(t.id)}>
                    {a}
                  </button>
                ))}
              </span>
            )}
          </span>
          <button className="icon-btn sm" aria-label="Dismiss" onClick={() => onDismiss(t.id)}>
            <Icon name="close" size={11} />
          </button>
        </div>
      ))}
    </div>
  );
}
