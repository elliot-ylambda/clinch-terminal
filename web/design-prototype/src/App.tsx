import { useCallback, useEffect, useMemo, useState } from "react";

import { PROJECTS, allTabs, type Project, type Section, type SectionColor, type Status, type Tab } from "./data";
import { Icon } from "./icons";
import { ConfirmDialog, Menu, NewSectionDialog, Palette, RemotePopover, Toasts, UsagePopover, type MenuEntry, type Toast } from "./Overlays";
import { Pane } from "./Pane";
import { FilesPanel, SkillsPanel } from "./Panels";
import { Settings, type RemoteState } from "./Settings";
import { Sidebar } from "./Sidebar";
import { TitleBar } from "./TitleBar";

type Overlay =
  | null
  | { kind: "palette" | "finder" | "newSection" | "quit" }
  | { kind: "closeTab"; tab: Tab }
  | { kind: "menu"; x: number; y: number; entries: MenuEntry[] }
  | { kind: "usage" | "remote"; x: number; y: number };

type Banner = { tone: "info" | "warn"; text: string; action: string } | null;

const clone = (): Project[] => structuredClone(PROJECTS);

export function App() {
  const [projects, setProjects] = useState<Project[]>(clone);
  const [projectId, setProjectId] = useState("magister");
  const [selected, setSelected] = useState<Record<string, string>>(() =>
    Object.fromEntries(PROJECTS.map((p) => [p.id, p.id === "magister" ? allTabs(p)[10].id : allTabs(p)[1]?.id ?? p.coordinator.id])),
  );
  const [view, setView] = useState<"workspace" | "settings">("workspace");
  const [settingsPage, setSettingsPage] = useState("general");
  const [theme, setTheme] = useState<"ink" | "paper">("ink");
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [panel, setPanel] = useState<"files" | "skills" | null>(null);
  const [overlay, setOverlay] = useState<Overlay>(null);
  const [findOpen, setFindOpen] = useState(false);
  const [split, setSplit] = useState(false);
  const [autoContinue, setAutoContinue] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [banner, setBanner] = useState<Banner>(null);
  const [remote, setRemote] = useState<RemoteState>("paired");
  const [updateAvailable, setUpdateAvailable] = useState(true);
  const [doneTasks, setDoneTasks] = useState<Set<number>>(new Set());
  const [multi, setMulti] = useState<Set<string>>(new Set());

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  const project = projects.find((p) => p.id === projectId)!;
  const tabs = useMemo(() => allTabs(project), [project]);
  const tab = tabs.find((t) => t.id === selected[projectId]) ?? project.coordinator;

  const select = useCallback(
    (id: string) => {
      setSelected((s) => ({ ...s, [projectId]: id }));
      setView("workspace");
      setFindOpen(false);
      setMulti(new Set());
      // Opening a finished tab marks it read, like the real app.
      setProjects((ps) => mapTabs(ps, (t) => (t.id === id && t.status === "done" ? { ...t, status: "idle" } : t)));
    },
    [projectId],
  );

  const toast = useCallback((t: Omit<Toast, "id">) => setToasts((ts) => [...ts.slice(-2), { ...t, id: Date.now() + Math.random() }]), []);

  const nextNeeds = useCallback(() => {
    const needs = tabs.filter((t) => t.status === "needs");
    if (needs.length === 0) return;
    const i = needs.findIndex((t) => t.id === tab.id);
    select(needs[(i + 1) % needs.length].id);
  }, [tabs, tab.id, select]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      const k = e.key.toLowerCase();
      if (k === "p") setOverlay({ kind: "palette" });
      else if (k === "," ) setView("settings");
      else if (k === "f" && !e.shiftKey) setFindOpen(true);
      else if (k === "b") setSidebarOpen((v) => !v);
      else if (k === "o" && e.shiftKey) setOverlay({ kind: "finder" });
      else if (k === "d") setSplit((v) => !v);
      else if (k === "a" && e.altKey) nextNeeds();
      else if (k === "q") setOverlay({ kind: "quit" });
      else return;
      e.preventDefault();
    };
    const onEsc = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || overlay || findOpen) return;
      setMulti(new Set());
      setView("workspace");
    };
    window.addEventListener("keyup", onEsc);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onEsc);
    };
  }, [nextNeeds, overlay, findOpen]);

  const sectionTargets = (ids: string[]): MenuEntry[] => [
    ...project.sections.map((s) => ({ label: s.name, onSelect: () => ids.forEach((id) => moveTab(id, s.id, "into")) })),
    { sep: true },
    { label: "New section…", icon: "plus", onSelect: () => setOverlay({ kind: "newSection" }) },
  ];

  const tabMenu = (t: Tab, x: number, y: number) => {
    const other = t.kind === "codex" ? "Claude Code" : "Codex";
    if (multi.size > 0 && (multi.has(t.id) || t.id === tab.id)) {
      const ids = [...new Set([...multi, tab.id, t.id])];
      setOverlay({
        kind: "menu",
        x,
        y,
        entries: [
          { heading: `${ids.length} tabs selected` },
          { label: "Move to section", icon: "layers", children: sectionTargets(ids) },
          { label: "Bookmark all", icon: "bookmark" },
          { sep: true },
          { label: `Close ${ids.length} tabs`, icon: "close", danger: true, onSelect: () => setMulti(new Set()) },
        ],
      });
      return;
    }
    const entries: MenuEntry[] = t.coordinator
      ? [
          { label: `Switch coordinator to ${other}`, icon: "transfer" },
          { label: "Rename", icon: "pencil", kbd: "F2" },
          { sep: true },
          { label: "Close coordinator", icon: "close", danger: true, onSelect: () => setOverlay({ kind: "closeTab", tab: t }) },
        ]
      : [
          { label: "Rename", icon: "pencil", kbd: "F2" },
          { label: "New tab below", icon: "plus" },
          ...(t.kind !== "shell"
            ? ([
                { label: "Fork in new tab", icon: "fork" },
                { label: `Transfer to ${other}`, icon: "transfer" },
                { label: "Bookmark session", icon: "bookmark" },
              ] as MenuEntry[])
            : []),
          { sep: true },
          { label: "Move to section", icon: "layers", children: sectionTargets([t.id]) },
          {
            label: "Move to project",
            icon: "external",
            children: projects.filter((p) => p.id !== projectId).map((p) => ({ label: p.name })),
          },
          { label: "Pin to top", icon: "pin" },
          { label: "Split right", icon: "splitRight", kbd: "⌘D", onSelect: () => setSplit(true) },
          { sep: true },
          { label: "Close tab", icon: "close", kbd: "⌘W", danger: true, onSelect: () => setOverlay({ kind: "closeTab", tab: t }) },
        ];
    setOverlay({ kind: "menu", x, y, entries });
  };

  const sectionMenu = (s: Section, x: number, y: number) =>
    setOverlay({
      kind: "menu",
      x,
      y,
      entries: [
        { label: "New tab in section", icon: "plus" },
        { label: "Rename", icon: "pencil" },
        { label: "Change color", icon: "palette" },
        { label: "Pin section", icon: "pin" },
        { sep: true },
        { label: "Move up", icon: "push" },
        { label: "Move down", icon: "arrowDown" },
        { sep: true },
        { label: `Delete “${s.name}” (keeps its tabs)`, icon: "trash", danger: true },
      ],
    });

  const newMenu = (x: number, y: number) =>
    setOverlay({
      kind: "menu",
      x,
      y,
      entries: [
        { label: "New Claude Code session", icon: "claude", kbd: "⌘⇧C" },
        { label: "New Codex session", icon: "codex" },
        { label: "New terminal tab", icon: "terminal", kbd: "⌘T" },
        { sep: true },
        { label: "New section…", icon: "layers", onSelect: () => setOverlay({ kind: "newSection" }) },
        { label: "Reopen conversation…", icon: "history", kbd: "⌘⇧O", onSelect: () => setOverlay({ kind: "finder" }) },
      ],
    });

  const moveTab = (tabId: string, targetId: string, where: "before" | "after" | "into") =>
    setProjects((ps) =>
      ps.map((p) => {
        if (p.id !== projectId) return p;
        const moving = allTabs(p).find((t) => t.id === tabId);
        if (!moving || moving.coordinator) return p;
        const strip = (list: Tab[]) => list.filter((t) => t.id !== tabId);
        const insert = (list: Tab[]) => {
          const i = list.findIndex((t) => t.id === targetId);
          if (i < 0) return list;
          const out = [...list];
          out.splice(where === "before" ? i : i + 1, 0, { ...moving, child: false });
          return out;
        };
        return {
          ...p,
          ungrouped: insert(strip(p.ungrouped)),
          sections: p.sections.map((s) =>
            where === "into" && s.id === targetId
              ? { ...s, open: true, tabs: [...strip(s.tabs), { ...moving, child: false }] }
              : { ...s, tabs: insert(strip(s.tabs)) },
          ),
        };
      }),
    );

  const createSection = (name: string, color: SectionColor) =>
    setProjects((ps) =>
      ps.map((p) => (p.id === projectId ? { ...p, sections: [{ id: `s${Date.now()}`, name, color, open: true, tabs: [] }, ...p.sections] } : p)),
    );

  const setStatus = (status: Status) =>
    setProjects((ps) => mapTabs(ps, (t) => (t.id === tab.id ? { ...t, status } : t)));

  const main =
    view === "settings" ? (
      <Settings
        page={settingsPage}
        onPage={setSettingsPage}
        onClose={() => setView("workspace")}
        remoteState={remote}
        onRemoteState={setRemote}
        theme={theme}
        onTheme={setTheme}
      />
    ) : split ? (
      <>
        <Pane {...paneProps()} split="focused" />
        <Pane {...paneProps()} tab={tabs.find((t) => t.kind === "shell") ?? project.coordinator} split="unfocused" findOpen={false} banner={null} />
      </>
    ) : (
      <Pane {...paneProps()} />
    );

  function paneProps() {
    return {
      tab,
      findOpen,
      onCloseFind: () => setFindOpen(false),
      autoContinue,
      onToggleAutoContinue: () => setAutoContinue((v) => !v),
      onAction: (label: string) => toast({ tone: "info" as const, title: label, body: `Sent to “${tab.title}”.` }),
      onEditButtons: () => {
        setView("settings");
        setSettingsPage("agents");
      },
      onOpenFinder: () => setOverlay({ kind: "finder" }),
      banner,
      onDismissBanner: () => setBanner(null),
    };
  }

  return (
    <div className="stage">
      <div className="window">
        <TitleBar
          projects={projects}
          activeId={projectId}
          onProject={(id) => {
            setProjectId(id);
            setView("workspace");
          }}
          sidebarOpen={sidebarOpen}
          onToggleSidebar={() => setSidebarOpen((v) => !v)}
          panel={panel}
          onPanel={(p) => setPanel((cur) => (cur === p ? null : p))}
          updateAvailable={updateAvailable}
          onUpdate={() =>
            toast({ tone: "info", title: "Update ready", body: "Clinch v0.2026.10.31.0912 is downloaded. Your sessions come back after the restart.", actions: ["Restart now", "Later"] })
          }
          remote={remote}
          onRemote={(x, y) => setOverlay({ kind: "remote", x, y })}
          onUsage={(x, y) => setOverlay({ kind: "usage", x, y })}
          onFinder={() => setOverlay({ kind: "finder" })}
          onGear={(x, y) =>
            setOverlay({
              kind: "menu",
              x: x - 240,
              y,
              entries: [
                { label: "Settings", icon: "gear", kbd: "⌘,", onSelect: () => setView("settings") },
                {
                  label: "Keyboard shortcuts",
                  icon: "keyboard",
                  onSelect: () => {
                    setView("settings");
                    setSettingsPage("keys");
                  },
                },
                { label: "What’s new", icon: "sparkle" },
                { sep: true },
                ...(updateAvailable ? ([{ label: "Update and restart Clinch", icon: "refresh" }] as MenuEntry[]) : []),
                { label: "Report an issue", icon: "external" },
                { label: "Open logs", icon: "file" },
              ],
            })
          }
          onNewProject={() => toast({ tone: "info", title: "New project", body: "Pick a folder to open as a project." })}
        />
        <div className="window-body">
          {sidebarOpen && (
            <Sidebar
              project={project}
              selectedId={view === "settings" ? "" : tab.id}
              onSelect={select}
              multi={multi}
              onMultiToggle={(id) =>
                setMulti((m) => {
                  const n = new Set(m);
                  if (n.has(id)) n.delete(id);
                  else n.add(id);
                  return n;
                })
              }
              onToggleSection={(id) =>
                setProjects((ps) => ps.map((p) => (p.id === projectId ? { ...p, sections: p.sections.map((s) => (s.id === id ? { ...s, open: !s.open } : s)) } : p)))
              }
              onMove={moveTab}
              onRename={(id, title) => setProjects((ps) => mapTabs(ps, (t) => (t.id === id && title.trim() ? { ...t, title: title.trim() } : t)))}
              onContextMenu={tabMenu}
              onNewMenu={newMenu}
              onSectionMenu={sectionMenu}
              doneTasks={doneTasks}
              onToggleTask={(i) =>
                setDoneTasks((d) => {
                  const n = new Set(d);
                  if (n.has(i)) n.delete(i);
                  else n.add(i);
                  return n;
                })
              }
            />
          )}
          <div className="main" style={sidebarOpen ? undefined : { paddingLeft: "var(--card-inset)" }}>
            {panel === "files" && view === "workspace" && <FilesPanel onClose={() => setPanel(null)} />}
            {panel === "skills" && view === "workspace" && <SkillsPanel onClose={() => setPanel(null)} />}
            {main}
          </div>
        </div>

        {overlay?.kind === "menu" && <Menu x={overlay.x} y={overlay.y} entries={overlay.entries} onClose={() => setOverlay(null)} />}
        {overlay?.kind === "usage" && <UsagePopover x={overlay.x} y={overlay.y} onClose={() => setOverlay(null)} />}
        {overlay?.kind === "remote" && (
          <RemotePopover
            x={overlay.x}
            y={overlay.y}
            connected={remote === "paired"}
            onClose={() => setOverlay(null)}
            onOpenSettings={() => {
              setView("settings");
              setSettingsPage("remote");
            }}
          />
        )}
        {(overlay?.kind === "palette" || overlay?.kind === "finder") && (
          <Palette
            initialScope={overlay.kind === "finder" ? "conversations" : "all"}
            onClose={() => setOverlay(null)}
            onPickTab={(pid, tid) => {
              setProjectId(pid);
              setSelected((s) => ({ ...s, [pid]: tid }));
              setView("workspace");
            }}
          />
        )}
        {overlay?.kind === "newSection" && <NewSectionDialog onClose={() => setOverlay(null)} onCreate={createSection} />}
        {overlay?.kind === "closeTab" && (
          <ConfirmDialog
            title={overlay.tab.coordinator ? "Close the coordinator?" : `Close “${overlay.tab.title}”?`}
            body={
              overlay.tab.coordinator
                ? "New projects won’t get a coordinator until you turn it back on from any tab’s menu."
                : overlay.tab.status === "working"
                  ? "Claude is still working in this tab. Closing it stops the turn. The conversation stays in Reopen conversation."
                  : "The conversation stays in Reopen conversation, so you can bring it back later."
            }
            confirm={overlay.tab.coordinator ? "Close coordinator" : "Close tab"}
            danger
            checkbox={overlay.tab.coordinator ? undefined : "Don’t ask again for finished sessions"}
            onClose={() => setOverlay(null)}
          />
        )}
        {overlay?.kind === "quit" && (
          <ConfirmDialog
            title="Quit Clinch?"
            body="4 agents are working and 1 command is running. Agent conversations come back when you reopen Clinch; the running command stops."
            confirm="Quit"
            danger
            checkbox="Don’t ask again"
            onClose={() => setOverlay(null)}
          />
        )}
        <Toasts toasts={toasts} onDismiss={(id) => setToasts((ts) => ts.filter((t) => t.id !== id))} />
      </div>

      <ProtoControls
        theme={theme}
        onTheme={setTheme}
        projectId={projectId}
        onProject={(id) => {
          setProjectId(id);
          setView("workspace");
        }}
        onSelectStatus={(s) => {
          const t = tabs.find((x) => (s === "shell" ? x.kind === "shell" : s === "coordinator" ? x.coordinator : x.status === s && !x.coordinator));
          if (t) select(t.id);
        }}
        onStatus={setStatus}
        onOverlay={(k) => {
          if (k === "settings") setView("settings");
          else if (k === "find") setFindOpen(true);
          else if (k === "split") setSplit((v) => !v);
          else if (k === "files" || k === "skills") setPanel(k);
          else if (k === "sidebar") setSidebarOpen((v) => !v);
          else if (k === "tabMenu") tabMenu(tab, 420, 320);
          else if (k === "newMenu") newMenu(250, 100);
          else setOverlay({ kind: k } as Overlay);
        }}
        onToast={(k) =>
          k === "done"
            ? toast({ tone: "done", title: "Harness bench S1 finished", body: "magister-marketing · P1", actions: ["Open", "Dismiss"] })
            : k === "needs"
              ? toast({ tone: "needs", title: "Magister gateway HTTP 5xx spikes needs you", body: "Claude is asking to run fly deploy.", actions: ["Open"] })
              : toast({ tone: "info", title: "Update ready", body: "Clinch v0.2026.10.31.0912 is downloaded.", actions: ["Restart now", "Later"] })
        }
        onBanner={(b) =>
          setBanner(
            b === "toml"
              ? { tone: "warn", text: "settings.toml has an error on line 42, so recent changes aren’t applied.", action: "Open file" }
              : b === "restored"
                ? { tone: "info", text: "Restored 23 sessions after the update. 2 couldn’t resume and started fresh.", action: "Show which" }
                : b === "plugin"
                  ? { tone: "info", text: "Turn on Clinch notifications for Codex so this tab shows when Codex needs you.", action: "Turn on" }
                  : null,
          )
        }
        onRemote={setRemote}
        onUpdate={() => setUpdateAvailable((v) => !v)}
      />
    </div>
  );
}

function mapTabs(ps: Project[], f: (t: Tab) => Tab): Project[] {
  return ps.map((p) => ({
    ...p,
    coordinator: f(p.coordinator),
    ungrouped: p.ungrouped.map(f),
    sections: p.sections.map((s) => ({ ...s, tabs: s.tabs.map(f) })),
  }));
}

/* ---------- Prototype controls: a jump list to every state, not part of the design ---------- */

type OverlayKey =
  | "palette"
  | "finder"
  | "newSection"
  | "quit"
  | "settings"
  | "find"
  | "split"
  | "files"
  | "skills"
  | "sidebar"
  | "tabMenu"
  | "newMenu";

function ProtoControls(props: {
  theme: "ink" | "paper";
  onTheme: (t: "ink" | "paper") => void;
  projectId: string;
  onProject: (id: string) => void;
  onSelectStatus: (s: Status | "shell" | "coordinator") => void;
  onStatus: (s: Status) => void;
  onOverlay: (k: OverlayKey) => void;
  onToast: (k: "done" | "needs" | "update") => void;
  onBanner: (b: "toml" | "restored" | "plugin" | null) => void;
  onRemote: (s: RemoteState) => void;
  onUpdate: () => void;
}) {
  const [open, setOpen] = useState(() => new URLSearchParams(location.search).has("proto"));
  return (
    <div className="proto">
      {open && (
        <div className="proto-panel" aria-label="Prototype controls">
          <h3>Theme</h3>
          <div className="opts">
            {(["ink", "paper"] as const).map((t) => (
              <button key={t} aria-pressed={props.theme === t} onClick={() => props.onTheme(t)}>
                {t === "ink" ? "Ink (dark)" : "Paper (light)"}
              </button>
            ))}
          </div>
          <h3>Project</h3>
          <div className="opts">
            {PROJECTS.map((p) => (
              <button key={p.id} aria-pressed={props.projectId === p.id} onClick={() => props.onProject(p.id)}>
                {p.name}
              </button>
            ))}
          </div>
          <h3>Select a tab that is…</h3>
          <div className="opts">
            {(["working", "needs", "done", "limited", "idle", "coordinator", "shell"] as const).map((s) => (
              <button key={s} onClick={() => props.onSelectStatus(s)}>
                {s}
              </button>
            ))}
          </div>
          <h3>Set the selected tab to…</h3>
          <div className="opts">
            {(["working", "needs", "done", "limited", "idle"] as Status[]).map((s) => (
              <button key={s} onClick={() => props.onStatus(s)}>
                {s}
              </button>
            ))}
          </div>
          <h3>Open</h3>
          <div className="opts">
            {(
              [
                ["palette", "Palette ⌘P"],
                ["finder", "Reopen conversation"],
                ["settings", "Settings ⌘,"],
                ["tabMenu", "Tab menu"],
                ["newMenu", "+ menu"],
                ["newSection", "New section"],
                ["quit", "Quit ⌘Q"],
                ["find", "Find ⌘F"],
                ["split", "Split ⌘D"],
                ["files", "Files"],
                ["skills", "Skills"],
                ["sidebar", "Sidebar ⌘B"],
              ] as [OverlayKey, string][]
            ).map(([k, l]) => (
              <button key={k} onClick={() => props.onOverlay(k)}>
                {l}
              </button>
            ))}
          </div>
          <h3>Notifications</h3>
          <div className="opts">
            <button onClick={() => props.onToast("done")}>Agent finished</button>
            <button onClick={() => props.onToast("needs")}>Needs you</button>
            <button onClick={() => props.onToast("update")}>Update ready</button>
            <button onClick={() => props.onBanner("toml")}>settings.toml error</button>
            <button onClick={() => props.onBanner("restored")}>Restored banner</button>
            <button onClick={() => props.onBanner("plugin")}>Plugin notice</button>
            <button onClick={() => props.onBanner(null)}>No banner</button>
            <button onClick={props.onUpdate}>Toggle update pill</button>
          </div>
          <h3>Remote Control</h3>
          <div className="opts">
            {(["off", "tailscale", "pairing", "approve", "paired", "error"] as RemoteState[]).map((s) => (
              <button key={s} onClick={() => props.onRemote(s)}>
                {s}
              </button>
            ))}
          </div>
        </div>
      )}
      <button className="proto-toggle" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        <Icon name="layers" size={13} /> States
      </button>
    </div>
  );
}
