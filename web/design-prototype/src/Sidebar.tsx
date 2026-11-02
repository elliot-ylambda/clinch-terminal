import { useEffect, useRef, useState } from "react";

import { LAST_PROMPT, PROVIDER_NAME, STATUS_WORD, type Project, type Section, type Tab } from "./data";
import { Icon } from "./icons";

export interface SidebarProps {
  project: Project;
  selectedId: string;
  onSelect: (id: string) => void;
  /** Tabs added with ⌘-click or ⇧-click, for bulk move and close. */
  multi: Set<string>;
  onMultiToggle: (id: string) => void;
  onToggleSection: (id: string) => void;
  onMove: (tabId: string, targetId: string, where: "before" | "after" | "into") => void;
  onRename: (tabId: string, title: string) => void;
  onContextMenu: (tab: Tab, x: number, y: number) => void;
  onNewMenu: (x: number, y: number) => void;
  onSectionMenu: (section: Section, x: number, y: number) => void;
  onToggleTask: (index: number) => void;
  doneTasks: Set<number>;
}

export function Sidebar(props: SidebarProps) {
  const { project } = props;
  const [query, setQuery] = useState("");
  const [needsOnly, setNeedsOnly] = useState(false);
  const [drag, setDrag] = useState<{ id: string; over?: string; where?: "before" | "after" | "into" } | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [footPanel, setFootPanel] = useState<"tasks" | "bookmarks" | null>(null);
  const [hover, setHover] = useState<{ tab: Tab; top: number; left: number } | null>(null);
  const hoverTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(hoverTimer.current), []);
  const onHover = (tab: Tab | null, el?: HTMLElement) => {
    window.clearTimeout(hoverTimer.current);
    if (!tab || !el) return setHover(null);
    // Details wait for a deliberate pause so passing the mouse over the list stays quiet.
    hoverTimer.current = window.setTimeout(() => {
      const r = el.getBoundingClientRect();
      setHover({ tab, top: r.top, left: r.right + 10 });
    }, 650);
  };

  const q = query.trim().toLowerCase();
  const visible = (tab: Tab) =>
    (!needsOnly || tab.status === "needs") &&
    (q === "" || tab.title.toLowerCase().includes(q) || tab.branch.toLowerCase().includes(q));
  const filtering = needsOnly || q !== "";

  const rowProps = { ...props, drag, setDrag, renaming, setRenaming, onHover };
  const coordinatorVisible = visible(project.coordinator);
  const ungrouped = project.ungrouped.filter(visible);
  const sections = project.sections
    .map((s) => ({ section: s, tabs: s.tabs.filter(visible) }))
    .filter(({ tabs }) => !filtering || tabs.length > 0);
  const nothing =
    !coordinatorVisible && ungrouped.length === 0 && sections.every(({ tabs }) => tabs.length === 0);

  return (
    <aside className="sidebar" aria-label="Tabs">
      <div className="sidebar-search">
        <label className="field">
          <Icon name="search" size={14} />
          <input
            placeholder="Search"
            aria-label="Search tabs"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && setQuery("")}
          />
          {query ? (
            <button className="icon-btn sm" aria-label="Clear search" onClick={() => setQuery("")}>
              <Icon name="close" size={12} />
            </button>
          ) : (
            <span className="kbd">⌘⇧F</span>
          )}
        </label>
        {project.waiting > 0 && (
          <button
            className="needs-filter"
            aria-pressed={needsOnly}
            aria-label={`${project.waiting} sessions need you; show only those`}
            title="Show only sessions that need you"
            onClick={() => setNeedsOnly((v) => !v)}
          >
            <Icon name="needs" size={14} />
            {project.waiting}
          </button>
        )}
        <button
          className="icon-btn"
          style={{ width: "var(--field-height)", height: "var(--field-height)", borderRadius: "var(--radius-md)" }}
          aria-label="New tab or section"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            props.onNewMenu(r.left, r.bottom + 6);
          }}
        >
          <Icon name="plus" size={15} />
        </button>
      </div>

      <div className="tab-list" role="tree" aria-label="Sessions">
        {nothing && (
          <div className="sidebar-empty">
            <strong>{filtering ? "No matching sessions" : "No sessions yet"}</strong>
            <span>
              {needsOnly
                ? "Nothing needs you right now."
                : q
                  ? `Nothing matches “${query}”.`
                  : "Start a Claude Code or Codex session, or open a terminal."}
            </span>
            {filtering ? (
              <button
                className="btn ghost"
                onClick={() => {
                  setQuery("");
                  setNeedsOnly(false);
                }}
              >
                Clear filters
              </button>
            ) : (
              <button
                className="btn"
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  props.onNewMenu(r.left, r.bottom + 6);
                }}
              >
                <Icon name="plus" size={14} />
                New session
              </button>
            )}
          </div>
        )}

        {(coordinatorVisible || ungrouped.length > 0) && (
          <div className="tab-group" role="group">
            {coordinatorVisible && <TabRow tab={project.coordinator} {...rowProps} />}
            {ungrouped.map((tab) => (
              <TabRow key={tab.id} tab={tab} {...rowProps} />
            ))}
          </div>
        )}

        {sections.map(({ section, tabs }) => {
          const open = filtering || section.open;
          const total = section.tabs.length + (section.hiddenCount ?? 0);
          const needs =
            section.tabs.filter((tb) => tb.status === "needs").length + (section.hiddenNeeds ?? 0);
          const dropTarget = drag?.over === section.id && drag.where === "into";
          return (
            <div className="tab-group" role="group" aria-label={section.name} key={section.id}>
              <div
                className={`section-header${dropTarget ? " drop-target" : ""}`}
                role="button"
                tabIndex={0}
                aria-expanded={open}
                onClick={() => props.onToggleSection(section.id)}
                onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && props.onToggleSection(section.id)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  props.onSectionMenu(section, e.clientX, e.clientY);
                }}
                onDragOver={(e) => {
                  if (!drag) return;
                  e.preventDefault();
                  setDrag({ ...drag, over: section.id, where: "into" });
                }}
                onDragLeave={() => drag && setDrag({ id: drag.id })}
                onDrop={(e) => {
                  e.preventDefault();
                  if (drag) props.onMove(drag.id, section.id, "into");
                  setDrag(null);
                }}
              >
                <span className="section-dot" style={{ background: `var(--section-${section.color})` }} />
                <span className="section-name">{section.name}</span>
                <span className="section-count">{total}</span>
                <span style={{ flex: 1 }} />
                <span className="section-count-wrap">
                  {!open && needs > 0 && (
                    <span className="section-needs" aria-label={`${needs} need you`}>
                      {needs}
                    </span>
                  )}
                </span>
                <span className="section-actions">
                  <button
                    className="icon-btn sm"
                    aria-label={`New tab in ${section.name}`}
                    onClick={(e) => e.stopPropagation()}
                  >
                    <Icon name="plus" size={12} />
                  </button>
                  <button
                    className="icon-btn sm"
                    aria-label={`${section.name} actions`}
                    onClick={(e) => {
                      e.stopPropagation();
                      const r = e.currentTarget.getBoundingClientRect();
                      props.onSectionMenu(section, r.left, r.bottom + 4);
                    }}
                  >
                    <Icon name="dots" size={12} />
                  </button>
                </span>
                <Icon name="chevron" size={12} className="section-chevron" strokeWidth={1.8} />
              </div>
              {open && (
                <>
                  {tabs.map((tab) => (
                    <TabRow key={tab.id} tab={tab} {...rowProps} />
                  ))}
                  {!filtering && (section.hiddenCount ?? 0) > 0 && (
                    <button className="show-more">Show {section.hiddenCount} more</button>
                  )}
                </>
              )}
            </div>
          );
        })}
      </div>

      <div className="sidebar-foot">
        <button
          className="foot-link"
          aria-expanded={footPanel === "tasks"}
          onClick={() => setFootPanel((p) => (p === "tasks" ? null : "tasks"))}
        >
          <Icon name="check" size={13} />
          Tasks <span className="n">{project.tasks.length - props.doneTasks.size}</span>
        </button>
        <button
          className="foot-link"
          aria-expanded={footPanel === "bookmarks"}
          onClick={() => setFootPanel((p) => (p === "bookmarks" ? null : "bookmarks"))}
        >
          <Icon name="bookmark" size={13} />
          Bookmarks <span className="n">{project.bookmarks.length}</span>
        </button>
        {footPanel && (
          <FootPanel
            kind={footPanel}
            project={project}
            doneTasks={props.doneTasks}
            onToggleTask={props.onToggleTask}
            onClose={() => setFootPanel(null)}
          />
        )}
      </div>
      {hover && !drag && <HoverCard {...hover} />}
    </aside>
  );
}

/** Details on demand: everything the old rows showed at once, after a hover pause. */
function HoverCard({ tab, top, left }: { tab: Tab; top: number; left: number }) {
  const prompt = LAST_PROMPT[tab.transcript];
  return (
    <div className="hover-card popover" style={{ top, left }} role="tooltip">
      <div className="hc-title">{tab.title}</div>
      <div className="hc-meta">
        <span className={`tab-glyph ${tab.restoring ? "idle" : tab.status}`}>
          <Icon name={tab.restoring ? "restoring" : tab.status === "needs" ? "needs" : tab.kind === "shell" ? "terminal" : tab.kind} size={13} />
        </span>
        {PROVIDER_NAME[tab.kind]} · {tab.restoring ? "restoring" : STATUS_WORD[tab.status]}
      </div>
      <dl>
        <dt>Branch</dt>
        <dd>{tab.branch}</dd>
        <dt>Folder</dt>
        <dd>{tab.cwd}</dd>
        {tab.pr && (
          <>
            <dt>Pull request</dt>
            <dd>
              #{tab.pr}
              {tab.diff && (
                <>
                  {" "}
                  · <span style={{ color: "var(--status-working)" }}>+{tab.diff[0]}</span>{" "}
                  <span style={{ color: "var(--status-limit)" }}>−{tab.diff[1]}</span>
                </>
              )}
            </dd>
          </>
        )}
      </dl>
      {prompt && <div className="hc-prompt">“{prompt}”</div>}
    </div>
  );
}

function TabRow({
  tab,
  selectedId,
  onSelect,
  multi,
  onMultiToggle,
  onMove,
  onRename,
  onContextMenu,
  drag,
  setDrag,
  renaming,
  setRenaming,
  onHover,
}: SidebarProps & {
  tab: Tab;
  drag: { id: string; over?: string; where?: "before" | "after" | "into" } | null;
  setDrag: (d: { id: string; over?: string; where?: "before" | "after" | "into" } | null) => void;
  renaming: string | null;
  setRenaming: (id: string | null) => void;
  onHover: (tab: Tab | null, el?: HTMLElement) => void;
}) {
  const selected = tab.id === selectedId || multi.has(tab.id);
  const glyph = tab.restoring ? "restoring" : tab.status === "needs" ? "needs" : tab.kind === "shell" ? "terminal" : tab.kind;
  const dropClass =
    drag?.over === tab.id ? (drag.where === "before" ? " drop-before" : drag.where === "after" ? " drop-after" : "") : "";
  const state = tab.restoring ? "restoring" : tab.status;

  return (
    <div
      className={`tab-row ${state}${tab.child ? " child" : ""}${multi.has(tab.id) ? " multi" : ""}${drag?.id === tab.id ? " dragging" : ""}${dropClass}`}
      role="treeitem"
      tabIndex={0}
      aria-selected={selected}
      aria-label={`${tab.title}, ${PROVIDER_NAME[tab.kind]}, ${tab.restoring ? "restoring" : STATUS_WORD[tab.status]}`}
      draggable={!tab.coordinator && renaming !== tab.id}
      onMouseEnter={(e) => onHover(tab, e.currentTarget)}
      onMouseLeave={() => onHover(null)}
      onClick={(e) => {
        onHover(null);
        if ((e.metaKey || e.shiftKey) && !tab.coordinator) onMultiToggle(tab.id);
        else onSelect(tab.id);
      }}
      onDoubleClick={() => setRenaming(tab.id)}
      onKeyDown={(e) => {
        if (renaming === tab.id) return;
        if (e.key === "Enter") onSelect(tab.id);
        if (e.key === "F2") setRenaming(tab.id);
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(tab, e.clientX, e.clientY);
      }}
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = "move";
        setDrag({ id: tab.id });
      }}
      onDragEnd={() => setDrag(null)}
      onDragOver={(e) => {
        if (!drag || drag.id === tab.id || tab.coordinator) return;
        e.preventDefault();
        const r = e.currentTarget.getBoundingClientRect();
        const where = e.clientY < r.top + r.height / 2 ? "before" : "after";
        if (drag.over !== tab.id || drag.where !== where) setDrag({ ...drag, over: tab.id, where });
      }}
      onDrop={(e) => {
        e.preventDefault();
        if (drag && drag.where && drag.where !== "into") onMove(drag.id, tab.id, drag.where);
        setDrag(null);
      }}
    >
      <span className={`tab-glyph ${tab.restoring ? "idle" : tab.status}`}>
        <Icon name={glyph} size={14} />
      </span>
      <span className="tab-text">
        {renaming === tab.id ? (
          <input
            className="tab-rename"
            autoFocus
            defaultValue={tab.title}
            aria-label="Tab name"
            onClick={(e) => e.stopPropagation()}
            onBlur={(e) => {
              onRename(tab.id, e.currentTarget.value);
              setRenaming(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") setRenaming(null);
            }}
          />
        ) : (
          <span className="tab-title">{tab.title}</span>
        )}
        {tab.id === selectedId && (
          <span className="tab-branch">{tab.restoring ? "Restoring session…" : tab.kind === "shell" ? tab.cwd : tab.branch}</span>
        )}
      </span>
      <button
        className="icon-btn sm row-more"
        aria-label={`Actions for ${tab.title}`}
        onClick={(e) => {
          e.stopPropagation();
          onHover(null);
          const r = e.currentTarget.getBoundingClientRect();
          onContextMenu(tab, r.left, r.bottom + 4);
        }}
      >
        <Icon name="dots" size={13} />
      </button>
    </div>
  );
}

function FootPanel({
  kind,
  project,
  doneTasks,
  onToggleTask,
  onClose,
}: {
  kind: "tasks" | "bookmarks";
  project: Project;
  doneTasks: Set<number>;
  onToggleTask: (i: number) => void;
  onClose: () => void;
}) {
  return (
    <div
      className="popover menu"
      style={{ left: 0, right: 0, bottom: 40, padding: "var(--space-4)", gap: "var(--space-2)" }}
      role="dialog"
      aria-label={kind === "tasks" ? "Tasks" : "Bookmarks"}
      onKeyDown={(e) => e.key === "Escape" && onClose()}
    >
      <div className="menu-label" style={{ display: "flex", alignItems: "center" }}>
        <span style={{ flex: 1 }}>{kind === "tasks" ? "Tasks" : "Bookmarked sessions"}</span>
        <button className="icon-btn sm" aria-label="Close" onClick={onClose}>
          <Icon name="close" size={11} />
        </button>
      </div>
      {kind === "tasks" ? (
        <>
          {project.tasks.length === 0 && (
            <div className="menu-label" style={{ fontWeight: 400, padding: "var(--space-4) var(--space-5)" }}>
              No tasks. Add one to remember something for later.
            </div>
          )}
          {project.tasks.map((task, i) => (
            <div key={task} className="check-row" style={{ padding: "6px var(--space-5)" }}>
              <button
                className="checkbox"
                role="checkbox"
                aria-checked={doneTasks.has(i)}
                aria-label={task}
                onClick={() => onToggleTask(i)}
              >
                {doneTasks.has(i) && <Icon name="check" size={11} strokeWidth={2.2} />}
              </button>
              <span
                style={{
                  color: doneTasks.has(i) ? "var(--text-3)" : "var(--text-1)",
                  textDecoration: doneTasks.has(i) ? "line-through" : "none",
                  flex: 1,
                }}
              >
                {task}
              </span>
              <span className="tab-glyph" title="Send to a new Claude session" style={{ color: "var(--text-3)" }}>
                <Icon name="claude" size={12} />
              </span>
            </div>
          ))}
          <label className="field" style={{ marginTop: 4 }}>
            <Icon name="plus" size={13} />
            <input placeholder="Add a task" aria-label="Add a task" />
          </label>
        </>
      ) : project.bookmarks.length === 0 ? (
        <div className="menu-label" style={{ fontWeight: 400, padding: "var(--space-4) var(--space-5)" }}>
          No bookmarks. Bookmark a session from its menu to keep it here after you close it.
        </div>
      ) : (
        project.bookmarks.map((b) => (
          <button key={b} className="menu-item">
            <Icon name="bookmark" size={13} className="icon" />
            {b}
          </button>
        ))
      )}
    </div>
  );
}
