import type { Project } from "./data";
import { Icon } from "./icons";
import type { RemoteState } from "./Settings";

export function TitleBar({
  projects,
  activeId,
  onProject,
  sidebarOpen,
  onToggleSidebar,
  panel,
  onPanel,
  updateAvailable,
  onUpdate,
  remote,
  onRemote,
  onUsage,
  onGear,
  onFinder,
  onNewProject,
}: {
  projects: Project[];
  activeId: string;
  onProject: (id: string) => void;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  panel: string | null;
  onPanel: (p: "files" | "skills") => void;
  updateAvailable: boolean;
  onUpdate: () => void;
  remote: RemoteState;
  onRemote: (x: number, y: number) => void;
  onUsage: (x: number, y: number) => void;
  onGear: (x: number, y: number) => void;
  onFinder: () => void;
  onNewProject: () => void;
}) {
  const remoteOn = remote === "paired";
  return (
    <header className="titlebar">
      <div className="traffic" aria-hidden="true">
        <span style={{ background: "#ff5f57" }} />
        <span style={{ background: "#febc2e" }} />
        <span style={{ background: "#28c840" }} />
      </div>
      <div className="titlebar-group">
        <button className="icon-btn" aria-label="Toggle sidebar" aria-pressed={sidebarOpen} title="Toggle sidebar  ⌘B" onClick={onToggleSidebar}>
          <Icon name="sidebar" size={17} strokeWidth={1.5} />
        </button>
        <button className="icon-btn" aria-label="File explorer" aria-pressed={panel === "files"} title="File explorer" onClick={() => onPanel("files")}>
          <Icon name="folder" size={17} strokeWidth={1.5} />
        </button>
        <button className="icon-btn" aria-label="Skills" aria-pressed={panel === "skills"} title="Skills" onClick={() => onPanel("skills")}>
          <Icon name="sparkle" size={16} strokeWidth={1.5} />
        </button>
        <button className="icon-btn" aria-label="Reopen a conversation" title="Reopen a conversation  ⌘⇧O" onClick={onFinder}>
          <Icon name="history" size={16} strokeWidth={1.5} />
        </button>
      </div>
      <nav className="projects" aria-label="Projects">
        {projects.map((p) => (
          <button key={p.id} className="project-pill" aria-current={p.id === activeId ? "page" : undefined} onClick={() => onProject(p.id)}>
            {p.name}
            {p.waiting > 0 && (
              <span className="badge" aria-label={`${p.waiting} need you`}>
                {p.waiting}
              </span>
            )}
          </button>
        ))}
        <button className="icon-btn" aria-label="New project" onClick={onNewProject}>
          <Icon name="plus" size={15} />
        </button>
      </nav>
      <div className="titlebar-right">
        <button
          className="usage"
          aria-label="Plan usage: Claude 5-hour 54%, weekly 86%; Codex weekly 100%"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            onUsage(r.right, r.bottom + 6);
          }}
        >
          <span className="meter">
            <span className="meter-name">Claude</span>
            <span className="meter-bar">
              <span style={{ width: "54%" }} />
            </span>
            <span className="meter-value">54%</span>
          </span>
          <span className="meter max">
            <span className="meter-name">Codex</span>
            <span className="meter-bar">
              <span style={{ width: "100%" }} />
            </span>
            <span className="meter-value">100%</span>
          </span>
        </button>
        <button
          className={`icon-btn${remoteOn ? " remote-on" : ""}`}
          aria-label={remoteOn ? "Remote Control: phone connected" : "Remote Control"}
          title={remoteOn ? "Remote Control · Elliot’s iPhone connected" : "Remote Control"}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            onRemote(r.right, r.bottom + 6);
          }}
        >
          <Icon name="phone" size={16} strokeWidth={1.5} />
        </button>
        {updateAvailable && (
          <button className="update-pill" onClick={onUpdate}>
            <span className="dot" style={{ background: "var(--accent)" }} />
            Update
          </button>
        )}
        <button
          className="icon-btn"
          aria-label="Clinch menu"
          aria-haspopup="menu"
          title="Settings and help"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            onGear(r.right, r.bottom + 6);
          }}
        >
          <Icon name="gear" size={16} strokeWidth={1.5} />
        </button>
      </div>
    </header>
  );
}
