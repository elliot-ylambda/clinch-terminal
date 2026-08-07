import { describe, expect, it } from "vitest";

import type { TargetRef } from "../generated/types/TargetRef";
import type { WorkspaceSnapshot } from "../generated/types/WorkspaceSnapshot";
import { isSafeTerminalResize } from "../terminal/TerminalSurface";
import {
  badgesForProject,
  drawerSessionSections,
  isRetryableWorkspaceResponse,
  parseRememberedTargets,
  preferredTargetForProject,
  shouldResynchronizeWorkspace,
  synchronizedSelection,
  workspaceWithWriterLease,
} from "./App";

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
      badges: { has_other_unread: true, done: 2, working: 3, running_commands: 4 },
      tabs: [
        {
          id: "tab-a",
          title: "Old tab",
          section_id: null,
          section_name: null,
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
            desktop_watching: false,
            size_pinned_by: null,
            quick_inserts: [],
          }],
        },
        {
          id: "tab-b",
          title: "New desktop tab",
          section_id: null,
          section_name: null,
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
            desktop_watching: false,
            size_pinned_by: null,
            quick_inserts: [],
          }],
        },
      ],
      tasks: [],
    }],
    active_target: activeTarget,
    usage: [],
    recent_agent_sessions: [],
    paired_devices: [],
  };
}

describe("bidirectional workspace selection", () => {
  it("uses the Mac's desktop-derived project badge counts", () => {
    const project = workspace(target("tab-b", "pane-b")).projects[0];
    expect(project && badgesForProject(project)).toEqual({
      has_other_unread: true,
      done: 2,
      working: 3,
      running_commands: 4,
    });
  });

  it("follows the Mac's new active target instead of reactivating the phone's stale tab", () => {
    const stalePhoneTarget = target("tab-a", "pane-a");
    const activeMacTarget = target("tab-b", "pane-b");

    expect(synchronizedSelection(workspace(activeMacTarget), "project", stalePhoneTarget)).toEqual({
      projectId: "project",
      target: activeMacTarget,
    });
  });

  it("restores the last live tab when returning to a project", () => {
    const activeMacTarget = target("tab-b", "pane-b");
    const rememberedPhoneTarget = target("tab-a", "pane-a");
    const snapshot = workspace(activeMacTarget);
    const project = snapshot.projects[0];
    expect(project && preferredTargetForProject(snapshot, project, rememberedPhoneTarget))
      .toEqual(rememberedPhoneTarget);

    const closedTarget = target("tab-closed", "pane-closed");
    expect(project && preferredTargetForProject(snapshot, project, closedTarget))
      .toEqual(activeMacTarget);
  });

  it("persists only valid opaque targets for refresh-safe project memory", () => {
    const remembered = target("tab-a", "pane-a");
    expect(parseRememberedTargets(JSON.stringify([remembered, { project_id: "incomplete" }])).get("project"))
      .toEqual(remembered);
    expect(parseRememberedTargets("not-json").size).toBe(0);
  });

  it("silently resynchronizes stale workspace navigation errors", () => {
    expect(shouldResynchronizeWorkspace("revision_conflict")).toBe(true);
    expect(shouldResynchronizeWorkspace("target_gone")).toBe(true);
    expect(shouldResynchronizeWorkspace("resync_required")).toBe(true);
    expect(shouldResynchronizeWorkspace("stale_quick_insert")).toBe(true);
    expect(shouldResynchronizeWorkspace("capability_denied")).toBe(false);
  });

  it("applies a writer lease response immediately without changing workspace revision", () => {
    const snapshot = workspace(target("tab-b", "pane-b"));
    const next = workspaceWithWriterLease(snapshot, target("tab-b", "pane-b"), {
      device_id: "phone",
      device_name: "iPhone",
      expires_at: "2026-07-31T18:15:00Z",
    });
    expect(next.revision).toBe(snapshot.revision);
    expect(next.projects[0]?.tabs[1]?.panes[0]?.writer_lease?.device_id).toBe("phone");
  });

  it("does not surface a retryable stale-revision response as an action error", () => {
    expect(isRetryableWorkspaceResponse({
      version: 1,
      request_id: "22222222-2222-4222-8222-222222222222",
      sequence: 3,
      payload: {
        type: "error",
        data: {
          code: "revision_conflict",
          message: "The workspace changed; refresh before sending input.",
          retryable: true,
          current_revision: 9,
        },
      },
    })).toBe(true);
  });

  it("preserves consecutive named and unsectioned session boundaries in the drawer", () => {
    const tabs = workspace(target("tab-b", "pane-b")).projects[0]!.tabs;
    const sections = drawerSessionSections([
      { ...tabs[0]!, section_id: "build", section_name: "Build" },
      { ...tabs[1]!, section_id: "build", section_name: "Build" },
      { ...tabs[1]!, id: "tab-review", section_id: "build-2", section_name: "Build" },
      { ...tabs[1]!, id: "tab-other", section_name: null },
    ]);

    expect(sections.map((section) => [section.name, section.tabs.map((tab) => tab.id)])).toEqual([
      ["Build", ["tab-a", "tab-b"]],
      ["Build", ["tab-review"]],
      [undefined, ["tab-other"]],
    ]);
  });
});

describe("mobile terminal sizing", () => {
  it("rejects transient startup geometry before it can narrow the Mac PTY", () => {
    expect(isSafeTerminalResize(2, 36, 12, 900)).toBe(false);
    expect(isSafeTerminalResize(53, 2, 400, 24)).toBe(false);
    expect(isSafeTerminalResize(53, 36, 400, 900)).toBe(true);
  });
});
