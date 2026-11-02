import { useState } from "react";

import { Icon } from "./icons";

const TREE: { name: string; depth: number; dir?: boolean; open?: boolean; git?: "M" | "A" | "?" }[] = [
  { name: "app", depth: 0, dir: true, open: true },
  { name: "src", depth: 1, dir: true, open: true },
  { name: "workspace", depth: 2, dir: true, open: true },
  { name: "view.rs", depth: 3, git: "M" },
  { name: "vertical_tabs.rs", depth: 3, git: "M" },
  { name: "coordinator.rs", depth: 3 },
  { name: "tab.rs", depth: 2, git: "M" },
  { name: "menu.rs", depth: 2 },
  { name: "crates", depth: 0, dir: true },
  { name: "specs", depth: 0, dir: true, open: true },
  { name: "clinch-redesign", depth: 1, dir: true, git: "?" },
  { name: "web", depth: 0, dir: true },
  { name: "AGENTS.md", depth: 0 },
  { name: "Cargo.toml", depth: 0 },
  { name: "Makefile", depth: 0 },
];

const GIT_COLOR = { M: "var(--status-needs)", A: "var(--status-working)", "?": "var(--status-working)" };

export function FilesPanel({ onClose }: { onClose: () => void }) {
  const [selected, setSelected] = useState("view.rs");
  return (
    <aside className="side-panel" aria-label="File explorer">
      <div className="panel-head">
        <span className="grow">clinch-terminal</span>
        <button className="icon-btn sm" aria-label="Collapse all">
          <Icon name="minus" size={13} />
        </button>
        <button className="icon-btn sm" aria-label="Close file explorer" onClick={onClose}>
          <Icon name="close" size={12} />
        </button>
      </div>
      <div className="panel-body" role="tree">
        {TREE.map((f) => (
          <button
            key={f.name}
            role="treeitem"
            className="tree-row"
            aria-selected={selected === f.name}
            style={{ ["--depth" as string]: f.depth }}
            onClick={() => setSelected(f.name)}
          >
            {f.dir ? (
              <Icon name={f.open ? "chevron" : "chevronRight"} size={12} className="tree-icon" strokeWidth={1.8} />
            ) : (
              <span style={{ width: 12 }} />
            )}
            <Icon name={f.dir ? "folder" : "file"} size={14} className="tree-icon" />
            <span>{f.name}</span>
            {f.git && (
              <span className="git" style={{ color: GIT_COLOR[f.git] }} aria-label={f.git === "M" ? "modified" : "untracked"}>
                {f.git}
              </span>
            )}
          </button>
        ))}
      </div>
    </aside>
  );
}

const SKILLS = [
  { name: "clinch-coordinate", desc: "Coordinate Claude Code and Codex sessions across projects.", src: "Clinch", used: true },
  { name: "clinch-control", desc: "Control and inspect the running Clinch app from an agent.", src: "Clinch" },
  { name: "clinch-toolbelt", desc: "Create and reorder footer buttons.", src: "Clinch" },
  { name: "agent-attribution", desc: "Truthful commit and PR attribution.", src: "~/.agents/skills" },
  { name: "plan-review", desc: "Critique a plan as a principal engineer before it runs.", src: "~/.claude/skills" },
  { name: "ship", desc: "Ship the current feature to production.", src: "~/.claude/commands" },
];

export function SkillsPanel({ onClose }: { onClose: () => void }) {
  return (
    <aside className="side-panel" aria-label="Skills">
      <div className="panel-head">
        <span className="grow">Skills</span>
        <button className="icon-btn sm" aria-label="Close skills" onClick={onClose}>
          <Icon name="close" size={12} />
        </button>
      </div>
      <div className="panel-body">
        <div className="panel-section">Used in this session</div>
        {SKILLS.filter((s) => s.used).map((s) => (
          <SkillRow key={s.name} {...s} />
        ))}
        <div className="panel-section">Available</div>
        {SKILLS.filter((s) => !s.used).map((s) => (
          <SkillRow key={s.name} {...s} />
        ))}
      </div>
    </aside>
  );
}

function SkillRow({ name, desc, src, used }: { name: string; desc: string; src: string; used?: boolean }) {
  return (
    <div className="skill-row">
      <span className="name" style={{ display: "flex", alignItems: "center", gap: 6 }}>
        {used && <span className="dot" style={{ background: "var(--status-working)" }} aria-label="used" />}
        {name}
      </span>
      <span className="desc">{desc}</span>
      <span className="desc" style={{ fontFamily: "var(--font-mono)" }}>
        {src}
      </span>
    </div>
  );
}
