import { useEffect, useRef, useState } from "react";

import { LAST_PROMPT, type Status, type Tab } from "./data";
import { Icon, type IconName } from "./icons";
import { TranscriptView } from "./Transcript";

export interface PaneProps {
  tab: Tab;
  /** Split panes show a small header so the active pane is identifiable. */
  split?: "focused" | "unfocused";
  findOpen: boolean;
  onCloseFind: () => void;
  autoContinue: boolean;
  onToggleAutoContinue: () => void;
  onAction: (label: string) => void;
  onEditButtons: () => void;
  onOpenFinder: () => void;
  banner?: { tone: "info" | "warn"; text: string; action: string } | null;
  onDismissBanner?: () => void;
}

const SHELL_BUTTONS: { label: string; icon: IconName }[] = [
  { label: "Claude", icon: "claude" },
  { label: "Codex", icon: "codex" },
  { label: "Resume", icon: "history" },
  { label: "Open in editor", icon: "external" },
  { label: "Commit", icon: "check" },
];

const PRIMARY: { label: string; icon: IconName }[] = [
  { label: "Fork", icon: "fork" },
  { label: "Compact", icon: "compact" },
  { label: "Continue", icon: "play" },
  { label: "LGTM", icon: "check" },
  { label: "Review PR", icon: "pr" },
  { label: "Push to main", icon: "push" },
];

const MORE: { group: string; items: string[] }[] = [
  { group: "Session", items: ["Bookmark session", "Fork in new tab", "Transfer to Codex", "Copy & clear", "Copy"] },
  { group: "Workflow", items: ["Create a plan", "Create a PR", "Address issues", "Status", "Make no mistakes"] },
  { group: "Remote", items: ["To DevBox", "Back to Mac", "Transfers", "Reopen conversation…"] },
];

const STATE_LABEL: Record<Status, string> = {
  working: "Working · 7m 10s",
  done: "Turn complete",
  needs: "Claude is asking a question",
  limited: "Rate limited · resumes in 42m",
  idle: "Idle",
};

export function Pane(props: PaneProps) {
  const { tab } = props;
  const isAgent = tab.kind !== "shell";
  const [historyOpen, setHistoryOpen] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const [find, setFind] = useState("");
  const [matchCase, setMatchCase] = useState(false);
  const [regex, setRegex] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setHistoryOpen(false);
    setMoreOpen(false);
  }, [tab.id]);

  useEffect(() => {
    if (!moreOpen) return;
    const close = (e: MouseEvent) => {
      if (!moreRef.current?.contains(e.target as Node)) setMoreOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setMoreOpen(false);
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc);
    };
  }, [moreOpen]);

  const provider = tab.kind === "codex" ? "Codex" : "Claude";
  const stateLabel =
    tab.status === "needs" ? `${provider} is asking a question` : STATE_LABEL[tab.status];

  return (
    <section
      className={`card${props.split === "focused" ? " focused-split" : ""}${props.split === "unfocused" ? " dim" : ""}`}
      aria-label={tab.title}
    >
      {props.split && (
        <div className="pane-header">
          <Icon name={tab.kind === "shell" ? "terminal" : tab.kind} size={13} />
          <span className="title">{tab.title}</span>
          <button className="icon-btn sm" aria-label="Close pane">
            <Icon name="close" size={11} />
          </button>
        </div>
      )}

      {props.banner && (
        <div className={`banner${props.banner.tone === "warn" ? " warn" : ""}`} role="status">
          <Icon name={props.banner.tone === "warn" ? "warning" : "info"} size={15} />
          <span className="grow">{props.banner.text}</span>
          <button className="btn" style={{ height: 26 }}>
            {props.banner.action}
          </button>
          <button className="icon-btn sm" aria-label="Dismiss" onClick={props.onDismissBanner}>
            <Icon name="close" size={11} />
          </button>
        </div>
      )}

      {isAgent && !props.split && (
        <button className="history-bar" aria-expanded={historyOpen} onClick={() => setHistoryOpen((v) => !v)}>
          <Icon name="history" size={14} />
          <span className="label">Last prompt</span>
          <span className="text">{LAST_PROMPT[tab.transcript] || "No messages yet"}</span>
          <Icon name="chevron" size={14} strokeWidth={1.8} />
        </button>
      )}
      {historyOpen && (
        <div className="history-pop popover" role="listbox" aria-label="Message history">
          {[
            ["2m", "And review all the Shopify docs and guidelines too when we’re doing our testing."],
            ["18m", "The publish still says outcome unknown. Find out why before changing the test."],
            ["41m", "Rebase on main and rerun the emulated contract tests."],
            ["1h", "Switch product creation to the GraphQL Admin API and keep REST as a fallback for one release."],
          ].map(([when, msg]) => (
            <button key={msg} className="history-item" onClick={() => setHistoryOpen(false)}>
              <span className="when">{when}</span>
              <span className="msg">{msg}</span>
              <Icon name="arrowDown" size={13} />
            </button>
          ))}
        </div>
      )}

      {props.findOpen && (
        <div className="find-bar" role="search">
          <label className="field">
            <Icon name="search" size={13} />
            <input
              autoFocus
              placeholder="Find"
              aria-label="Find in terminal"
              value={find}
              onChange={(e) => setFind(e.target.value)}
              onKeyDown={(e) => e.key === "Escape" && props.onCloseFind()}
            />
          </label>
          <button className="find-toggle" aria-pressed={matchCase} aria-label="Match case" title="Match case" onClick={() => setMatchCase(!matchCase)}>
            Aa
          </button>
          <button className="find-toggle" aria-pressed={regex} aria-label="Regular expression" title="Regular expression" onClick={() => setRegex(!regex)}>
            .*
          </button>
          <span className="find-count">{find ? "2 of 5" : "No matches"}</span>
          <button className="icon-btn sm" aria-label="Previous match">
            <Icon name="chevron" size={12} className="flip" strokeWidth={1.8} />
          </button>
          <button className="icon-btn sm" aria-label="Next match">
            <Icon name="chevron" size={12} />
          </button>
          <button className="icon-btn sm" aria-label="Close find" onClick={props.onCloseFind}>
            <Icon name="close" size={11} />
          </button>
        </div>
      )}

      <div className="term" tabIndex={0} aria-label="Terminal output">
        <TranscriptView tab={tab} find={props.findOpen ? find : ""} />
      </div>

      {isAgent ? (
        <>
          <div className="agent-input" aria-label={`${provider} input (drawn by ${provider})`}>
            <div className="agent-prompt">
              <span style={{ color: "var(--text-3)" }}>❯</span>
              <span className="cursor" aria-hidden="true" />
            </div>
            <div>
              <span className="claude">{tab.kind === "codex" ? "gpt-5.5-codex" : "Opus 5.5"}</span> high ·{" "}
              {tab.status === "limited" ? "71%" : "26%"} context ·{" "}
              <span style={{ color: "var(--text-2)" }}>{tab.branch}</span>
            </div>
            <div>bypass permissions on · 4 agents</div>
          </div>
          <footer className="agent-footer" aria-label="Agent actions">
            <span className={`agent-state ${tab.status}`} role="status">
              <span className="dot" aria-hidden="true" />
              {stateLabel}
            </span>
            {PRIMARY.map((b) => (
              <button key={b.label} className="chip" title={b.label} aria-label={b.label} onClick={() => props.onAction(b.label)}>
                <Icon name={b.icon} size={14} />
                <span className="chip-label">{b.label}</span>
              </button>
            ))}
            <div style={{ position: "relative" }} ref={moreRef}>
              <button className="chip" aria-expanded={moreOpen} aria-haspopup="menu" onClick={() => setMoreOpen((v) => !v)}>
                More
                <Icon name="chevron" size={12} strokeWidth={1.8} />
              </button>
              {moreOpen && (
                <div className="menu more-menu popover" role="menu">
                  {MORE.map((g) => (
                    <div className="menu-col" key={g.group}>
                      <div className="menu-label">{g.group}</div>
                      {g.items.map((it) => (
                        <button
                          key={it}
                          role="menuitem"
                          className="menu-item"
                          onClick={() => {
                            setMoreOpen(false);
                            if (it === "Reopen conversation…") props.onOpenFinder();
                            else props.onAction(it);
                          }}
                        >
                          {it}
                        </button>
                      ))}
                    </div>
                  ))}
                  <div className="menu-sep" style={{ gridColumn: "1 / -1" }} />
                  <button
                    role="menuitem"
                    className="menu-item"
                    style={{ gridColumn: "1 / -1" }}
                    onClick={() => {
                      setMoreOpen(false);
                      props.onEditButtons();
                    }}
                  >
                    <Icon name="pencil" size={13} className="icon" />
                    Edit buttons…
                  </button>
                </div>
              )}
            </div>
            <span className="footer-spacer" />
            <button
              className="toggle-label"
              role="switch"
              aria-checked={props.autoContinue}
              onClick={props.onToggleAutoContinue}
              title="Send “continue” automatically when a rate limit resets"
            >
              <span className="switch" aria-hidden="true" aria-checked={props.autoContinue} />
              <span className="chip-label">
                {tab.status === "limited" ? (props.autoContinue ? "Continues in 42m" : "Continue when it resets") : "Auto-continue"}
              </span>
            </button>
          </footer>
        </>
      ) : (
        <>
        <div className="shell-input" aria-label="Command input">
          <div className="context-chips">
            <span className="context-chip">
              <Icon name="folder" size={12} />
              {tab.cwd}
            </span>
            <span className="context-chip">
              <Icon name="branch" size={12} />
              {tab.branch}
            </span>
          </div>
          <div className="shell-line">
            <span style={{ color: "var(--accent)" }}>❯</span>
            <span className="cursor" aria-hidden="true" />
          </div>
        </div>
        <footer className="agent-footer" aria-label="Terminal actions" style={{ paddingTop: 0 }}>
          {SHELL_BUTTONS.map((b) => (
            <button key={b.label} className="chip" title={b.label} aria-label={b.label} onClick={() => props.onAction(b.label)}>
              <Icon name={b.icon} size={14} />
              <span className="chip-label">{b.label}</span>
            </button>
          ))}
          <span className="footer-spacer" />
          <button className="icon-btn" aria-label="Edit buttons" title="Edit buttons" onClick={props.onEditButtons}>
            <Icon name="pencil" size={14} />
          </button>
        </footer>
        </>
      )}
    </section>
  );
}
