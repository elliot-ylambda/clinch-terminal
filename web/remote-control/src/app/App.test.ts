import { describe, expect, it } from "vitest";

import type { TargetRef } from "../generated/types/TargetRef";
import type { WorkspaceSnapshot } from "../generated/types/WorkspaceSnapshot";
import { shouldResynchronizeWorkspace, synchronizedSelection } from "./App";

const appInstanceId = "11111111-1111-4111-8111-111111111111";

function target(tabId: string, paneId: string): TargetRef {
  return {
    app_instance_id: appInstanceId,
    project_id: "project",
    tab_id: tabId,
    pane_id: paneId,
  };
}

function workspace(activeTarget: TargetRef): WorkspaceSnapshot {
  return {
    revision: 8,
    sequence: 2,
    host: {
      app_instance_id: appInstanceId,
      name: "Mac",
      connection_path: "unknown",
      capabilities: ["view", "control", "create_session", "upload"],
    },
    projects: [{
      id: "project",
      title: "Project",
      order: 0,
      active: true,
      activity: "working",
      tabs: [
        {
          id: "tab-a",
          title: "Old tab",
          kind: "terminal",
          active: false,
          activity: "idle",
          unread: false,
          remote_host: null,
          panes: [{
            id: "pane-a",
            title: "Old pane",
            kind: "terminal",
            cwd: "/tmp",
            active: true,
            agent_state: null,
            dimensions: { columns: 80, rows: 24 },
            writer_lease: null,
            quick_inserts: [],
          }],
        },
        {
          id: "tab-b",
          title: "New desktop tab",
          kind: "terminal",
          active: true,
          activity: "idle",
          unread: false,
          remote_host: null,
          panes: [{
            id: "pane-b",
            title: "New desktop pane",
            kind: "terminal",
            cwd: "/tmp",
            active: true,
            agent_state: null,
            dimensions: { columns: 80, rows: 24 },
            writer_lease: null,
            quick_inserts: [],
          }],
        },
      ],
    }],
    active_target: activeTarget,
    usage: [],
    recent_agent_sessions: [],
    paired_devices: [],
  };
}

describe("bidirectional workspace selection", () => {
  it("follows the Mac's new active target instead of reactivating the phone's stale tab", () => {
    const stalePhoneTarget = target("tab-a", "pane-a");
    const activeMacTarget = target("tab-b", "pane-b");

    expect(synchronizedSelection(workspace(activeMacTarget), "project", stalePhoneTarget)).toEqual({
      projectId: "project",
      target: activeMacTarget,
    });
  });

  it("silently resynchronizes stale workspace navigation errors", () => {
    expect(shouldResynchronizeWorkspace("revision_conflict")).toBe(true);
    expect(shouldResynchronizeWorkspace("target_gone")).toBe(true);
    expect(shouldResynchronizeWorkspace("resync_required")).toBe(true);
    expect(shouldResynchronizeWorkspace("capability_denied")).toBe(false);
  });
});
