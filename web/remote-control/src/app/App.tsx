import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { ClientMessage } from "../generated/types/ClientMessage";
import type { ConnectionState } from "../generated/types/ConnectionState";
import type { PaneSnapshot } from "../generated/types/PaneSnapshot";
import type { ProtocolErrorCode } from "../generated/types/ProtocolErrorCode";
import type { ProjectBadgeSnapshot } from "../generated/types/ProjectBadgeSnapshot";
import type { ProjectSnapshot } from "../generated/types/ProjectSnapshot";
import type { ServerEnvelope } from "../generated/types/ServerEnvelope";
import type { SessionKind } from "../generated/types/SessionKind";
import type { TabSnapshot } from "../generated/types/TabSnapshot";
import type { TargetRef } from "../generated/types/TargetRef";
import type { TerminalKey } from "../generated/types/TerminalKey";
import type { TerminalSnapshot } from "../generated/types/TerminalSnapshot";
import type { WorkspaceSnapshot } from "../generated/types/WorkspaceSnapshot";
import type { WriterLeaseSnapshot } from "../generated/types/WriterLeaseSnapshot";
import { MAX_UPLOAD_CHUNK_BYTES } from "../generated/constants";
import { encodeUploadChunk } from "../protocol/binary";
import { CompanionClient } from "../protocol/client";
import { claimPhone, finishPairing, waitForApproval } from "../protocol/pairing";
import {
  bytesToBase64,
  clearIdentity,
  clearPendingPairing,
  createIdentity,
  loadIdentity,
  loadPendingPairing,
  savePendingPairing,
  type DeviceIdentity,
} from "../protocol/storage";
import { clearPairingFragment, takePairingFragment, type PairingFragment } from "../protocol/urls";
import { TerminalBus } from "../terminal/bus";
import { TerminalSurface, type TerminalSurfaceHandle } from "../terminal/TerminalSurface";

type BootState = "loading" | "waiting_approval" | "needs_qr" | "ready" | "error";
type NewSessionMode = "create" | "resume";

interface FittedTerminalViewport {
  targetKey: string;
  columns: number;
  rows: number;
}

interface QueuedTerminalInput {
  targetKey: string;
  data: string;
}

const connectionLabels: Record<ConnectionState, string> = {
  connected: "Connected",
  reconnecting: "Reconnecting",
  mac_offline: "Mac offline",
  tailscale_needed: "Tailscale needed",
  authorization_revoked: "Authorization revoked",
  version_incompatible: "Update required",
};

// Capture and remove the one-time secret exactly once, before React StrictMode can render the
// component twice. The secret never becomes part of an HTTP request or referrer.
const initialPairingFragment = takePairingFragment(location);
if (initialPairingFragment) clearPairingFragment();

function ClinchMark({ className }: { className: string }) {
  return <img className={className} src="./clinch-logo.svg" alt="Clinch" />;
}

function targetKey(target: TargetRef): string {
  return `${target.app_instance_id}:${target.project_id}:${target.tab_id}:${target.pane_id}`;
}

function fittedViewportKey(viewport: FittedTerminalViewport): string {
  return `${viewport.targetKey}:${viewport.columns}:${viewport.rows}`;
}

const LAST_TARGETS_STORAGE_KEY = "clinch-remote-control:last-target-by-project";

export function parseRememberedTargets(serialized: string | null): Map<string, TargetRef> {
  if (!serialized) return new Map();
  try {
    const candidates = JSON.parse(serialized) as unknown;
    if (!Array.isArray(candidates)) return new Map();
    const targets = new Map<string, TargetRef>();
    for (const candidate of candidates) {
      if (
        typeof candidate === "object"
        && candidate !== null
        && typeof (candidate as TargetRef).app_instance_id === "string"
        && typeof (candidate as TargetRef).project_id === "string"
        && typeof (candidate as TargetRef).tab_id === "string"
        && typeof (candidate as TargetRef).pane_id === "string"
      ) {
        const target = candidate as TargetRef;
        targets.set(target.project_id, target);
      }
    }
    return targets;
  } catch {
    return new Map();
  }
}

function targetFor(project: ProjectSnapshot, tab: TabSnapshot, pane: PaneSnapshot, appInstanceId: string): TargetRef {
  return {
    app_instance_id: appInstanceId,
    project_id: project.id,
    tab_id: tab.id,
    pane_id: pane.id,
  };
}

function firstTarget(snapshot: WorkspaceSnapshot, project?: ProjectSnapshot): TargetRef | undefined {
  const selectedProject = project ?? snapshot.projects.find((candidate) => candidate.active) ?? snapshot.projects[0];
  if (!selectedProject) return undefined;
  const activeTab = selectedProject.tabs.find((candidate) => candidate.active);
  const tabs = activeTab
    ? [activeTab, ...selectedProject.tabs.filter((candidate) => candidate.id !== activeTab.id)]
    : selectedProject.tabs;
  for (const tab of tabs) {
    const activePane = tab.panes.find((candidate) => candidate.active && candidate.dimensions);
    const pane = activePane ?? tab.panes.find((candidate) => candidate.dimensions);
    if (pane) return targetFor(selectedProject, tab, pane, snapshot.host.app_instance_id);
  }
  return undefined;
}

export function preferredTargetForProject(
  snapshot: WorkspaceSnapshot,
  project: ProjectSnapshot,
  rememberedTarget: TargetRef | undefined,
): TargetRef | undefined {
  if (rememberedTarget?.project_id === project.id) {
    const remembered = resolveTarget(snapshot, rememberedTarget);
    if (remembered.project?.id === project.id && remembered.pane?.dimensions) {
      return rememberedTarget;
    }
  }
  return firstTarget(snapshot, project);
}

function resolveTarget(snapshot: WorkspaceSnapshot | undefined, target: TargetRef | undefined) {
  if (!snapshot || !target || snapshot.host.app_instance_id !== target.app_instance_id) return {};
  const project = snapshot.projects.find((candidate) => candidate.id === target.project_id);
  const tab = project?.tabs.find((candidate) => candidate.id === target.tab_id);
  const pane = tab?.panes.find((candidate) => candidate.id === target.pane_id);
  return { project, tab, pane };
}

export function synchronizedSelection(
  snapshot: WorkspaceSnapshot,
  currentProjectId: string | undefined,
  currentTarget: TargetRef | undefined,
): { projectId: string | undefined; target: TargetRef | undefined } {
  const active = resolveTarget(snapshot, snapshot.active_target ?? undefined);
  if (snapshot.active_target && active.project && active.pane?.dimensions) {
    return { projectId: active.project.id, target: snapshot.active_target };
  }

  const current = resolveTarget(snapshot, currentTarget);
  const activeProject = snapshot.projects.find((project) => project.active);
  const preferredProject =
    activeProject ??
    snapshot.projects.find((project) => project.id === currentProjectId) ??
    current.project ??
    snapshot.projects[0];
  if (!preferredProject) return { projectId: undefined, target: undefined };
  if (current.project?.id === preferredProject.id && current.pane?.dimensions) {
    return { projectId: preferredProject.id, target: currentTarget };
  }
  return { projectId: preferredProject.id, target: firstTarget(snapshot, preferredProject) };
}

export function workspaceWithWriterLease(
  snapshot: WorkspaceSnapshot,
  target: TargetRef,
  lease: WriterLeaseSnapshot | null,
): WorkspaceSnapshot {
  if (snapshot.host.app_instance_id !== target.app_instance_id) return snapshot;
  let changed = false;
  const projects = snapshot.projects.map((project) => {
    if (project.id !== target.project_id) return project;
    const tabs = project.tabs.map((tab) => {
      if (tab.id !== target.tab_id) return tab;
      const panes = tab.panes.map((pane) => {
        if (pane.id !== target.pane_id) return pane;
        changed = true;
        return { ...pane, writer_lease: lease };
      });
      return { ...tab, panes };
    });
    return { ...project, tabs };
  });
  return changed ? { ...snapshot, projects } : snapshot;
}

export function shouldResynchronizeWorkspace(code: ProtocolErrorCode): boolean {
  return code === "revision_conflict"
    || code === "target_gone"
    || code === "resync_required"
    || code === "stale_quick_insert";
}

export function isRetryableWorkspaceResponse(response: ServerEnvelope | undefined): boolean {
  return response?.payload.type === "error"
    && response.payload.data.retryable
    && shouldResynchronizeWorkspace(response.payload.data.code);
}

function defaultDeviceName(): string {
  if (/iPad/i.test(navigator.userAgent)) return "iPad";
  if (/iPhone/i.test(navigator.userAgent)) return "iPhone";
  return "Mobile browser";
}

const compactNumber = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });

function formatTokens(value: number): string {
  return compactNumber.format(Math.max(0, value));
}

function connectionPathLabel(path: WorkspaceSnapshot["host"]["connection_path"] | undefined): string {
  if (path === "tailnet_direct") return "Direct tailnet path";
  if (path === "tailnet_relay") return "Tailscale relay path";
  if (path === "loopback_development") return "Loopback development path";
  return "Private Tailscale path";
}

/** Keeps a newly updated web bundle usable for one refresh while an older dev gateway restarts. */
export function badgesForProject(project: ProjectSnapshot): ProjectBadgeSnapshot {
  const snapshotBadges = (project as ProjectSnapshot & { badges?: ProjectBadgeSnapshot }).badges;
  if (snapshotBadges) return snapshotBadges;

  let done = 0;
  let working = 0;
  let runningCommands = 0;
  let hasOtherUnread = false;
  for (const tab of project.tabs) {
    const doneInTab = tab.panes.filter((pane) => pane.agent_state === "done").length;
    working += tab.panes.filter((pane) => pane.agent_state === "working").length;
    if (tab.unread) done += doneInTab;
    if (tab.activity === "running_command") runningCommands += 1;
    if (tab.unread && doneInTab === 0) hasOtherUnread = true;
  }
  return {
    has_other_unread: hasOtherUnread,
    done,
    working,
    running_commands: runningCommands,
  };
}

function projectBadgeLabel(project: ProjectSnapshot): string {
  const badges = badgesForProject(project);
  const labels = [
    badges.done > 0 ? `${badges.done} done` : undefined,
    badges.working > 0 ? `${badges.working} working` : undefined,
    badges.running_commands > 0 ? `${badges.running_commands} commands running` : undefined,
    badges.has_other_unread ? "unread activity" : undefined,
  ].filter(Boolean);
  return labels.length > 0 ? `${project.title}, ${labels.join(", ")}` : project.title;
}

function drawerTabActivity(tab: TabSnapshot): { className: string; label: string } | undefined {
  if (tab.activity === "working") return { className: "working", label: "Working" };
  if (tab.activity === "done") return { className: "done", label: "Done" };
  if (tab.activity === "needs_attention") return { className: "done", label: "Needs attention" };
  if (tab.activity === "running_command") return { className: "command", label: "Command running" };
  if (tab.kind === "claude_code" || tab.kind === "codex") {
    return { className: "idle", label: "Idle" };
  }
  return undefined;
}

export function App() {
  const pairingFragment = useRef<PairingFragment | null>(initialPairingFragment);

  const [boot, setBoot] = useState<BootState>("loading");
  const [bootMessage, setBootMessage] = useState("Opening your private Clinch connection…");
  const [identity, setIdentity] = useState<DeviceIdentity>();
  const identityRef = useRef<DeviceIdentity | undefined>(undefined);
  identityRef.current = identity;
  const [connection, setConnection] = useState<ConnectionState>("reconnecting");
  const connectionRef = useRef<ConnectionState>("reconnecting");
  connectionRef.current = connection;
  const [connectionDetail, setConnectionDetail] = useState<string>();
  const [snapshot, setSnapshot] = useState<WorkspaceSnapshot>();
  const snapshotRef = useRef<WorkspaceSnapshot | undefined>(undefined);
  snapshotRef.current = snapshot;
  const [selectedProjectId, setSelectedProjectId] = useState<string>();
  const selectedProjectIdRef = useRef<string | undefined>(undefined);
  selectedProjectIdRef.current = selectedProjectId;
  const [selectedTarget, setSelectedTarget] = useState<TargetRef>();
  const selectedTargetRef = useRef<TargetRef | undefined>(undefined);
  selectedTargetRef.current = selectedTarget;
  const lastTargetByProject = useRef(new Map<string, TargetRef>());
  const loadedRememberedTargets = useRef(false);
  if (!loadedRememberedTargets.current) {
    loadedRememberedTargets.current = true;
    try {
      lastTargetByProject.current = parseRememberedTargets(localStorage.getItem(LAST_TARGETS_STORAGE_KEY));
    } catch {
      // Storage can be unavailable in private browser modes; in-memory project history still works.
    }
  }
  const [terminalSnapshot, setTerminalSnapshot] = useState<TerminalSnapshot>();
  const terminalSnapshotRef = useRef<TerminalSnapshot | undefined>(undefined);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [usageOpen, setUsageOpen] = useState(false);
  const [newOpen, setNewOpen] = useState(false);
  const [newMode, setNewMode] = useState<NewSessionMode>("create");
  const [newKind, setNewKind] = useState<SessionKind>("terminal");
  const [newCwd, setNewCwd] = useState("");
  const [newPrompt, setNewPrompt] = useState("");
  const [newResumeId, setNewResumeId] = useState("");
  const [creating, setCreating] = useState(false);
  const [quickInsertBusy, setQuickInsertBusy] = useState(false);
  const [resyncing, setResyncing] = useState(false);
  const [notice, setNotice] = useState<string>();
  const [uploadProgress, setUploadProgress] = useState<number>();
  const [uploadActive, setUploadActive] = useState(false);
  const [uploadRetryFile, setUploadRetryFile] = useState<File>();
  const client = useRef<CompanionClient | undefined>(undefined);
  const terminalBus = useMemo(() => new TerminalBus(), []);
  const terminalSurface = useRef<TerminalSurfaceHandle>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const activeUploadId = useRef<string | undefined>(undefined);
  const uploadCancelled = useRef(false);
  const resyncRequested = useRef(false);
  const creationInFlight = useRef(false);
  const quickInsertInFlight = useRef(false);
  const targetSelectionInFlight = useRef<string | undefined>(undefined);
  const autoOpenedProjects = useRef(new Set<string>());
  const terminalViewport = useRef<FittedTerminalViewport | undefined>(undefined);
  const terminalResizableViewport = useRef<FittedTerminalViewport | undefined>(undefined);
  const terminalReadyViewport = useRef<string | undefined>(undefined);
  const terminalPreparation = useRef<{ generation: number; promise: Promise<boolean> } | undefined>(undefined);
  const terminalPreparationGeneration = useRef(0);
  const queuedTerminalInput = useRef<QueuedTerminalInput[]>([]);

  const resetTerminalPreparation = useCallback((discardInput = true) => {
    terminalPreparationGeneration.current += 1;
    terminalPreparation.current = undefined;
    terminalViewport.current = undefined;
    terminalResizableViewport.current = undefined;
    terminalReadyViewport.current = undefined;
    if (discardInput) queuedTerminalInput.current = [];
  }, []);

  const rememberTarget = useCallback((target: TargetRef) => {
    lastTargetByProject.current.set(target.project_id, target);
    try {
      localStorage.setItem(LAST_TARGETS_STORAGE_KEY, JSON.stringify([...lastTargetByProject.current.values()]));
    } catch {
      // Remembering targets is an optional local convenience, never a connection requirement.
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        let localIdentity = await loadIdentity();
        let pendingReceipt = await loadPendingPairing();
        if (pendingReceipt && new Date(pendingReceipt.expires_at).getTime() <= Date.now()) {
          await clearPendingPairing();
          pendingReceipt = undefined;
        }
        if (localIdentity?.deviceId && pendingReceipt) {
          await clearPendingPairing();
          pendingReceipt = undefined;
        }
        const invitation = pairingFragment.current;
        if (invitation) {
          setBootMessage("Creating this phone's private device key…");
          localIdentity ??= await createIdentity(defaultDeviceName());
          setIdentity(localIdentity);
          setBootMessage("Sending the one-time pairing request…");
          pendingReceipt = await claimPhone(invitation, localIdentity);
          await savePendingPairing(pendingReceipt);
        }
        if (pendingReceipt && localIdentity) {
          setBoot("waiting_approval");
          setBootMessage(`Approve “${pendingReceipt.device_name}” in Clinch on your Mac.`);
          const status = await waitForApproval(pendingReceipt);
          if (status.status === "approved") {
            localIdentity = await finishPairing(localIdentity, status);
            await clearPendingPairing();
          } else if (status.status === "rejected") {
            await clearPendingPairing();
            throw new Error("The pairing request was rejected on the Mac.");
          } else {
            await clearPendingPairing();
            throw new Error("The pairing QR code expired. Generate a new one in Clinch.");
          }
        } else if (pendingReceipt) {
          await clearPendingPairing();
        }
        if (cancelled) return;
        if (localIdentity?.deviceId) {
          setIdentity(localIdentity);
          setBoot("ready");
        } else {
          setBoot("needs_qr");
          setBootMessage("Open Clinch Settings on your Mac and scan a pairing QR code.");
        }
      } catch (error) {
        if (!cancelled) {
          setBoot("error");
          setBootMessage(error instanceof Error ? error.message : String(error));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleEnvelope = useCallback((envelope: ServerEnvelope) => {
    const payload = envelope.payload;
    switch (payload.type) {
      case "snapshot":
        resyncRequested.current = false;
        setResyncing(false);
        snapshotRef.current = payload.data;
        setSnapshot(payload.data);
        {
          const previousTarget = selectedTargetRef.current;
          const selection = synchronizedSelection(
            payload.data,
            selectedProjectIdRef.current,
            selectedTargetRef.current,
          );
          if (
            previousTarget
            && (!selection.target || targetKey(previousTarget) !== targetKey(selection.target))
          ) resetTerminalPreparation();
          selectedProjectIdRef.current = selection.projectId;
          selectedTargetRef.current = selection.target;
          setSelectedProjectId(selection.projectId);
          setSelectedTarget(selection.target);
        }
        break;
      case "workspace_changed":
        resyncRequested.current = false;
        setResyncing(false);
        snapshotRef.current = payload.data.snapshot;
        setSnapshot(payload.data.snapshot);
        {
          const previousTarget = selectedTargetRef.current;
          const selection = synchronizedSelection(
            payload.data.snapshot,
            selectedProjectIdRef.current,
            selectedTargetRef.current,
          );
          if (
            previousTarget
            && (!selection.target || targetKey(previousTarget) !== targetKey(selection.target))
          ) resetTerminalPreparation();
          selectedProjectIdRef.current = selection.projectId;
          selectedTargetRef.current = selection.target;
          setSelectedProjectId(selection.projectId);
          setSelectedTarget(selection.target);
          if (
            selection.target
            && previousTarget
            && targetKey(previousTarget) !== targetKey(selection.target)
          ) rememberTarget(selection.target);
        }
        break;
      case "terminal_snapshot":
        if (
          !terminalSnapshotRef.current
          || terminalSnapshotRef.current.stream_id !== payload.data.stream_id
          || targetKey(terminalSnapshotRef.current.target) !== targetKey(payload.data.target)
        ) resetTerminalPreparation();
        if (
          selectedTargetRef.current
          && targetKey(selectedTargetRef.current) === targetKey(payload.data.target)
        ) {
          targetSelectionInFlight.current = undefined;
        }
        terminalSnapshotRef.current = payload.data;
        setTerminalSnapshot(payload.data);
        break;
      case "terminal_stream_closed":
        targetSelectionInFlight.current = undefined;
        terminalSnapshotRef.current = undefined;
        resetTerminalPreparation();
        setNotice(payload.data.reason);
        setTerminalSnapshot(undefined);
        break;
      case "writer_lease_changed":
        if (
          selectedTargetRef.current
          && targetKey(selectedTargetRef.current) === targetKey(payload.data.target)
          && payload.data.lease?.device_id !== identity?.deviceId
        ) resetTerminalPreparation();
        if (snapshotRef.current) {
          const next = workspaceWithWriterLease(
            snapshotRef.current,
            payload.data.target,
            payload.data.lease,
          );
          snapshotRef.current = next;
          setSnapshot(next);
        }
        break;
      case "command_accepted":
        // Mutation responses carry the new authoritative revision even though the full topology
        // snapshot arrives on the next gateway poll. Advancing it immediately prevents a second
        // exact-target action from being rejected during that short window.
        if (snapshotRef.current && payload.data.workspace_revision > snapshotRef.current.revision) {
          const next = { ...snapshotRef.current, revision: payload.data.workspace_revision };
          snapshotRef.current = next;
          setSnapshot(next);
        }
        break;
      case "upload_progress":
        if (activeUploadId.current === payload.data.upload_id) {
          setUploadProgress(payload.data.total ? payload.data.received / payload.data.total : 0);
        }
        break;
      case "upload_completed":
        activeUploadId.current = undefined;
        setUploadActive(false);
        setUploadRetryFile(undefined);
        setUploadProgress(undefined);
        setNotice(`Inserted ${payload.data.inserted_path} without pressing Enter.`);
        break;
      case "connection_state":
        if (payload.data !== "connected") targetSelectionInFlight.current = undefined;
        setConnection(payload.data);
        break;
      case "error":
        targetSelectionInFlight.current = undefined;
        if (payload.data.retryable && shouldResynchronizeWorkspace(payload.data.code)) {
          setNotice(undefined);
          setResyncing(true);
          terminalSnapshotRef.current = undefined;
          resetTerminalPreparation();
          setTerminalSnapshot(undefined);
          if (!resyncRequested.current) {
            resyncRequested.current = true;
            try {
              client.current?.send({ type: "request_snapshot" });
            } catch (error) {
              resyncRequested.current = false;
              setNotice(error instanceof Error ? error.message : String(error));
            }
          }
        } else {
          setNotice(payload.data.message);
        }
        break;
      default:
        break;
    }
  }, [identity?.deviceId, rememberTarget, resetTerminalPreparation]);

  useEffect(() => {
    if (boot !== "ready" || !identity?.deviceId) return;
    const companion = new CompanionClient(identity, {
      connection: (state, detail) => {
        setConnection(state);
        setConnectionDetail(detail);
        if (state !== "connected") {
          targetSelectionInFlight.current = undefined;
          terminalSnapshotRef.current = undefined;
          resetTerminalPreparation();
          setTerminalSnapshot(undefined);
        }
      },
      envelope: handleEnvelope,
      terminal: (frame) => terminalBus.emit(frame),
      resync: setResyncing,
    });
    client.current = companion;
    companion.start();
    return () => {
      companion.stop();
      if (client.current === companion) client.current = undefined;
    };
  }, [boot, handleEnvelope, identity, resetTerminalPreparation, terminalBus]);

  const selected = resolveTarget(snapshot, selectedTarget);
  const selectedProject = snapshot?.projects.find((project) => project.id === selectedProjectId) ?? selected.project;
  const selectedLocalCwd = selected.tab?.remote_host ? undefined : selected.pane?.cwd ?? undefined;
  const ownsWriterLease = selected.pane?.writer_lease?.device_id === identity?.deviceId;
  const canWrite =
    connection === "connected" &&
    !resyncing &&
    Boolean(selectedTarget) &&
    (!selected.pane?.writer_lease || selected.pane.writer_lease.device_id === identity?.deviceId);

  useEffect(() => {
    if (connection !== "connected" || !snapshot || !selectedTarget) return;
    if (
      terminalSnapshot &&
      targetKey(terminalSnapshot.target) === targetKey(selectedTarget)
    ) {
      return;
    }
    const selectionKey = `${targetKey(selectedTarget)}:${snapshot.revision}`;
    if (targetSelectionInFlight.current === selectionKey) return;
    targetSelectionInFlight.current = selectionKey;
    try {
      client.current?.send({
        type: "select_target",
        data: { target: selectedTarget, workspace_revision: snapshot.revision },
      });
    } catch (error) {
      if (targetSelectionInFlight.current === selectionKey) {
        targetSelectionInFlight.current = undefined;
      }
      setNotice(error instanceof Error ? error.message : String(error));
    }
  }, [connection, selectedTarget, snapshot, terminalSnapshot]);

  const selectTarget = useCallback(
    (target: TargetRef | undefined) => {
      if (selectedTarget && (!target || targetKey(selectedTarget) !== targetKey(target))) {
        try {
          client.current?.send({
            type: "release_writer_lease",
            data: { target: selectedTarget },
          });
        } catch {
          // Disconnect cleanup also releases the lease and restores desktop sizing.
        }
      }
      if (target) {
        rememberTarget(target);
        selectedProjectIdRef.current = target.project_id;
        setSelectedProjectId(target.project_id);
      }
      selectedTargetRef.current = target;
      targetSelectionInFlight.current = undefined;
      terminalSnapshotRef.current = undefined;
      resetTerminalPreparation();
      setSelectedTarget(target);
      setDrawerOpen(false);
      setTerminalSnapshot(undefined);
    },
    [rememberTarget, resetTerminalPreparation, selectedTarget],
  );

  const sendRawInputNow = useCallback((expectedTargetKey: string, data: string): boolean => {
    if (!data) return true;
    const currentSnapshot = snapshotRef.current;
    const currentTarget = selectedTargetRef.current;
    const activeTerminalSnapshot = terminalSnapshotRef.current;
    const deviceId = identityRef.current?.deviceId;
    if (
      !currentSnapshot
      || !currentTarget
      || !activeTerminalSnapshot
      || !deviceId
      || connectionRef.current !== "connected"
      || targetKey(currentTarget) !== expectedTargetKey
      || targetKey(activeTerminalSnapshot.target) !== expectedTargetKey
    ) return false;
    const lease = resolveTarget(currentSnapshot, currentTarget).pane?.writer_lease;
    if (lease?.device_id !== deviceId) return false;
    try {
      client.current?.send({
        type: "raw_terminal_input",
        data: {
          target: currentTarget,
          workspace_revision: currentSnapshot.revision,
          data_base64: bytesToBase64(new TextEncoder().encode(data)),
        },
      });
      return true;
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
      return false;
    }
  }, []);

  const flushQueuedTerminalInput = useCallback((expectedTargetKey: string) => {
    const queued = queuedTerminalInput.current;
    queuedTerminalInput.current = [];
    for (const input of queued) {
      if (input.targetKey === expectedTargetKey) sendRawInputNow(expectedTargetKey, input.data);
    }
  }, [sendRawInputNow]);

  const prepareTerminalForInput = useCallback((): Promise<boolean> => {
    const currentSnapshot = snapshotRef.current;
    const currentTarget = selectedTargetRef.current;
    const companion = client.current;
    const deviceId = identityRef.current?.deviceId;
    if (
      !currentSnapshot
      || !currentTarget
      || !companion
      || !deviceId
      || connectionRef.current !== "connected"
    ) return Promise.resolve(false);

    const expectedTargetKey = targetKey(currentTarget);
    const existing = terminalPreparation.current;
    const generation = terminalPreparationGeneration.current;
    if (existing?.generation === generation) return existing.promise;

    const promise = (async () => {
      const viewportDeadline = Date.now() + 1_500;
      try {
        const initialLease = resolveTarget(currentSnapshot, currentTarget).pane?.writer_lease;
        if (initialLease && initialLease.device_id !== deviceId) {
          setNotice(`${initialLease.device_name} currently has control of this terminal.`);
          return false;
        }
        if (!initialLease) {
          const response = await companion.sendAndWait({
            type: "acquire_writer_lease",
            data: { target: currentTarget, workspace_revision: currentSnapshot.revision },
          }, 5_000);
          if (response.payload.type === "error") throw new Error(response.payload.data.message);
        }

        while (
          terminalPreparationGeneration.current === generation
          && connectionRef.current === "connected"
          && selectedTargetRef.current
          && targetKey(selectedTargetRef.current) === expectedTargetKey
        ) {
          // TerminalSurface only marks a viewport resizable after React has committed writer
          // ownership and, for alternate-screen CLIs, cleared the stale desktop-sized buffer.
          const viewport = terminalResizableViewport.current;
          if (!viewport || viewport.targetKey !== expectedTargetKey) {
            // Acquiring the lease triggers a React commit before TerminalSurface can report the
            // writer-owned viewport. Keep the originating tap alive through that short handoff so
            // accessory keys and one-tap quick inserts cannot disappear on first use.
            if (Date.now() >= viewportDeadline) return false;
            await new Promise<void>((resolve) => window.setTimeout(resolve, 16));
            continue;
          }
          const viewportKey = fittedViewportKey(viewport);
          if (terminalReadyViewport.current === viewportKey) {
            flushQueuedTerminalInput(expectedTargetKey);
            return true;
          }
          const latestSnapshot = snapshotRef.current;
          const latestTarget = selectedTargetRef.current;
          const activeTerminalSnapshot = terminalSnapshotRef.current;
          if (
            !latestSnapshot
            || !latestTarget
            || !activeTerminalSnapshot
            || targetKey(activeTerminalSnapshot.target) !== expectedTargetKey
          ) return false;
          const sequenceBeforeResize = Math.max(
            activeTerminalSnapshot.terminal_sequence,
            terminalBus.sequenceFor(activeTerminalSnapshot.stream_id),
          );
          const response = await companion.sendAndWait({
            type: "terminal_resize",
            data: {
              target: latestTarget,
              workspace_revision: latestSnapshot.revision,
              dimensions: { columns: viewport.columns, rows: viewport.rows },
            },
          }, 5_000);
          if (response.payload.type === "error") throw new Error(response.payload.data.message);
          if (terminalPreparationGeneration.current !== generation) return false;
          // The Mac's resize response confirms the ioctl, but zsh/Codex/Claude can repaint on the
          // PTY just afterward. Wait for that stream activity to settle before releasing queued
          // keys, otherwise a late cursor redraw can split one word across the existing prompt.
          await terminalBus.waitForQuiescenceAfter(
            activeTerminalSnapshot.stream_id,
            sequenceBeforeResize,
          );

          const latestViewport = terminalResizableViewport.current;
          if (
            latestViewport
            && latestViewport.targetKey === expectedTargetKey
            && fittedViewportKey(latestViewport) === viewportKey
          ) {
            terminalReadyViewport.current = viewportKey;
            flushQueuedTerminalInput(expectedTargetKey);
            return true;
          }
          // The iOS keyboard or an orientation change altered the viewport while the Mac was
          // acknowledging this resize. Loop once more so input can never overtake the new size.
        }
      } catch (error) {
        if (terminalPreparationGeneration.current === generation) {
          setNotice(error instanceof Error ? error.message : String(error));
        }
      }
      return false;
    })();
    terminalPreparation.current = { generation, promise };
    void promise.finally(() => {
      if (terminalPreparation.current?.promise === promise) terminalPreparation.current = undefined;
    });
    return promise;
  }, [flushQueuedTerminalInput, terminalBus]);

  const acquireLease = useCallback(() => {
    void prepareTerminalForInput();
  }, [prepareTerminalForInput]);

  const sendRawInput = useCallback((data: string): boolean => {
    if (!data) return true;
    const currentSnapshot = snapshotRef.current;
    const currentTarget = selectedTargetRef.current;
    const deviceId = identityRef.current?.deviceId;
    if (!currentSnapshot || !currentTarget || !deviceId || connectionRef.current !== "connected") {
      return false;
    }
    const lease = resolveTarget(currentSnapshot, currentTarget).pane?.writer_lease;
    if (lease && lease.device_id !== deviceId) return false;
    const expectedTargetKey = targetKey(currentTarget);
    const viewport = terminalResizableViewport.current;
    if (
      viewport?.targetKey === expectedTargetKey
      && terminalReadyViewport.current === fittedViewportKey(viewport)
    ) return sendRawInputNow(expectedTargetKey, data);

    queuedTerminalInput.current.push({ targetKey: expectedTargetKey, data });
    void prepareTerminalForInput();
    return true;
  }, [prepareTerminalForInput, sendRawInputNow]);

  const sendTerminalKey = useCallback(
    async (key: TerminalKey) => {
      if (!canWrite) return;
      // Keep the iOS keyboard open while an accessory key takes focus long enough to activate.
      terminalSurface.current?.focus();
      if (!(await prepareTerminalForInput())) return;
      const currentSnapshot = snapshotRef.current;
      const currentTarget = selectedTargetRef.current;
      if (!currentSnapshot || !currentTarget || connectionRef.current !== "connected") return;
      try {
        client.current?.send({
          type: "terminal_key",
          data: {
            target: currentTarget,
            workspace_revision: currentSnapshot.revision,
            key,
          },
        });
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      }
    },
    [canWrite, prepareTerminalForInput],
  );

  const submitQuickInsert = useCallback(
    async (itemId: string, configurationRevision: number) => {
      if (
        quickInsertInFlight.current
        || !canWrite
      ) return;
      // Keep native keyboard activation inside the originating tap, then serialize submission
      // behind the exact target's lease and phone-sized PTY handoff.
      terminalSurface.current?.focus();
      quickInsertInFlight.current = true;
      setQuickInsertBusy(true);
      try {
        if (!(await prepareTerminalForInput())) return;
        const currentSnapshot = snapshotRef.current;
        const currentTarget = selectedTargetRef.current;
        if (!currentSnapshot || !currentTarget || connectionRef.current !== "connected") return;
        const response = await client.current?.sendAndWait({
          type: "quick_insert_submit",
          data: {
            target: currentTarget,
            workspace_revision: currentSnapshot.revision,
            item_id: itemId,
            configuration_revision: configurationRevision,
          },
        });
        if (isRetryableWorkspaceResponse(response)) return;
        if (response?.payload.type === "error") throw new Error(response.payload.data.message);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      } finally {
        quickInsertInFlight.current = false;
        setQuickInsertBusy(false);
      }
    },
    [
      canWrite,
      prepareTerminalForInput,
    ],
  );

  const runCreation = useCallback(async (message: ClientMessage): Promise<boolean> => {
    if (creationInFlight.current) return false;
    creationInFlight.current = true;
    setCreating(true);
    try {
      const response = await client.current?.sendAndWait(message);
      if (!response) throw new Error("The Mac is not connected");
      if (response.payload.type === "error") {
        if (response.payload.data.retryable && shouldResynchronizeWorkspace(response.payload.data.code)) {
          return false;
        }
        throw new Error(response.payload.data.message);
      }
      return true;
    } finally {
      creationInFlight.current = false;
      setCreating(false);
    }
  }, []);

  const createTab = useCallback(
    async (projectId: string, kind: SessionKind, cwd?: string | null, initialPrompt?: string) => {
      if (!snapshot) return false;
      return runCreation({
        type: "create_session",
        data: {
          app_instance_id: snapshot.host.app_instance_id,
          workspace_revision: snapshot.revision,
          project_id: projectId,
          kind,
          cwd: cwd?.trim() || null,
          initial_prompt: initialPrompt?.trim() || null,
        },
      });
    },
    [runCreation, snapshot],
  );

  const createProject = useCallback(async () => {
    if (!snapshot || !selectedProject) return;
    try {
      await runCreation({
        type: "create_project",
        data: {
          app_instance_id: snapshot.host.app_instance_id,
          workspace_revision: snapshot.revision,
          project_id: selectedProject.id,
          cwd: selectedLocalCwd?.trim() || null,
        },
      });
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
    }
  }, [runCreation, selectedLocalCwd, selectedProject, snapshot]);

  const createSession = useCallback(async () => {
    if (!snapshot) return;
    const projectId = selectedProject?.id ?? snapshot.projects[0]?.id;
    if (!projectId) {
      setNotice("Choose a project first.");
      return;
    }
    try {
      const recent = snapshot.recent_agent_sessions.find(
        (session) => session.durable_session_id === newResumeId.trim(),
      );
      const created = newMode === "resume"
        ? await runCreation({
            type: "resume_session",
            data: {
              app_instance_id: snapshot.host.app_instance_id,
              workspace_revision: snapshot.revision,
              project_id: projectId,
              provider: recent?.provider ?? (newKind === "codex" ? "codex" : "claude_code"),
              durable_session_id: newResumeId.trim(),
              cwd: newCwd,
            },
          })
        : await createTab(projectId, newKind, newCwd, newPrompt);
      if (created) {
        setNewOpen(false);
        setNewPrompt("");
      }
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
    }
  }, [createTab, newCwd, newKind, newMode, newPrompt, newResumeId, runCreation, selectedProject?.id, snapshot]);

  useEffect(() => {
    if (connection !== "connected" || resyncing || !snapshot || !selectedProject) return;
    // A newly-created terminal exists before its private shell bootstrap is safe to stream.
    // Count that pane now so reloads cannot create duplicate terminals while it starts.
    const hasSession = selectedProject.tabs.some((tab) => tab.panes.length > 0);
    const key = `${snapshot.host.app_instance_id}:${selectedProject.id}`;
    if (hasSession || autoOpenedProjects.current.has(key)) return;
    autoOpenedProjects.current.add(key);
    void createTab(selectedProject.id, "terminal")
      .then((created) => {
        if (!created) autoOpenedProjects.current.delete(key);
      })
      .catch((error) => {
        autoOpenedProjects.current.delete(key);
        setNotice(error instanceof Error ? error.message : String(error));
      });
  }, [connection, createTab, resyncing, selectedProject, snapshot]);

  const upload = useCallback(
    async (file: File) => {
      if (!snapshot || !selectedTarget || !canWrite) return;
      try {
        uploadCancelled.current = false;
        activeUploadId.current = undefined;
        setUploadActive(true);
        setUploadRetryFile(file);
        setUploadProgress(0);
        const bytes = new Uint8Array(await file.arrayBuffer());
        const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
        const sha256 = [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");
        const response = await client.current?.sendAndWait({
          type: "upload_begin",
          data: {
            target: selectedTarget,
            workspace_revision: snapshot.revision,
            filename: file.name,
            mime: file.type || "application/octet-stream",
            size: file.size,
            sha256,
          },
        });
        if (!response || response.payload.type !== "upload_ready") {
          throw new Error(response?.payload.type === "error" ? response.payload.data.message : "Upload was not accepted");
        }
        const { upload_id: uploadId, chunk_size: serverChunkSize } = response.payload.data;
        activeUploadId.current = uploadId;
        if (uploadCancelled.current) {
          client.current?.send({ type: "upload_cancel", data: { upload_id: uploadId } });
          throw new Error("Upload canceled");
        }
        const chunkSize = Math.min(serverChunkSize, MAX_UPLOAD_CHUNK_BYTES);
        for (let offset = 0, index = 0; offset < bytes.length; offset += chunkSize, index += 1) {
          if (uploadCancelled.current) throw new Error("Upload canceled");
          client.current?.sendBinary(encodeUploadChunk(uploadId, index, bytes.slice(offset, offset + chunkSize)));
          setUploadProgress(Math.min(1, (offset + chunkSize) / bytes.length));
          await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
        }
        if (uploadCancelled.current) throw new Error("Upload canceled");
        const completed = await client.current?.sendAndWait({ type: "upload_commit", data: { upload_id: uploadId } });
        if (completed?.payload.type === "error") throw new Error(completed.payload.data.message);
      } catch (error) {
        const canceled = uploadCancelled.current;
        const uploadId = activeUploadId.current;
        if (uploadId) {
          try {
            client.current?.send({ type: "upload_cancel", data: { upload_id: uploadId } });
          } catch {
            // Disconnect cleanup removes the staging file on the Mac.
          }
        }
        activeUploadId.current = undefined;
        setUploadActive(false);
        setUploadProgress(undefined);
        setNotice(canceled ? "Upload canceled. You can retry when ready." : error instanceof Error ? error.message : String(error));
      }
    },
    [canWrite, selectedTarget, snapshot],
  );

  const cancelUpload = useCallback(() => {
    uploadCancelled.current = true;
    const uploadId = activeUploadId.current;
    if (uploadId) {
      try {
        client.current?.send({ type: "upload_cancel", data: { upload_id: uploadId } });
      } catch {
        // Disconnect cleanup removes the staging file on the Mac.
      }
    }
  }, []);

  const rememberTerminalViewport = useCallback((columns: number, rows: number) => {
    const target = selectedTargetRef.current;
    if (!target) return;
    const viewport = { targetKey: targetKey(target), columns, rows };
    const previous = terminalViewport.current;
    terminalViewport.current = viewport;
    if (!previous || fittedViewportKey(previous) !== fittedViewportKey(viewport)) {
      // Gate input immediately when the browser starts fitting a new viewport. onResize marks the
      // same dimensions safe only after the writer-owned xterm buffer is ready for the Mac redraw.
      terminalResizableViewport.current = undefined;
      terminalReadyViewport.current = undefined;
    }
  }, []);

  const resizeSelectedTerminal = useCallback((columns: number, rows: number) => {
    const target = selectedTargetRef.current;
    if (!target) return;
    const viewport = { targetKey: targetKey(target), columns, rows };
    terminalViewport.current = viewport;
    terminalResizableViewport.current = viewport;
    if (terminalReadyViewport.current !== fittedViewportKey(viewport)) {
      terminalReadyViewport.current = undefined;
    }
    void prepareTerminalForInput();
  }, [prepareTerminalForInput]);

  if (boot !== "ready") {
    return <PairingScreen state={boot} message={bootMessage} />;
  }

  const projects = snapshot?.projects ?? [];
  const quickInserts = selected.pane?.quick_inserts ?? [];
  const leaseMessage = selected.pane?.writer_lease
    ? selected.pane.writer_lease.device_id === identity?.deviceId
      ? "This phone has control"
      : `${selected.pane.writer_lease.device_name} has control`
    : undefined;

  return (
    <div className="app-shell">
      <header className="project-strip" aria-label="Projects">
        <button
          className="project-drawer-button"
          aria-label="Open project and tab drawer"
          aria-expanded={drawerOpen}
          onClick={() => setDrawerOpen(true)}
        >
          <ClinchMark className="wordmark" />
        </button>
        <div className="project-scroll">
          {projects.map((project) => {
            const badges = badgesForProject(project);
            return (
              <button
                aria-label={projectBadgeLabel(project)}
                className={`project-pill ${project.id === selectedProject?.id ? "active" : ""}`}
                key={project.id}
                onClick={() => {
                  if (project.id === selectedProject?.id) {
                    setDrawerOpen(true);
                    return;
                  }
                  if (!snapshot) return;
                  selectedProjectIdRef.current = project.id;
                  setSelectedProjectId(project.id);
                  const target = preferredTargetForProject(
                    snapshot,
                    project,
                    lastTargetByProject.current.get(project.id),
                  );
                  if (target) {
                    selectTarget(target);
                  } else {
                    selectTarget(undefined);
                  }
                }}
              >
                <span className="project-badges" aria-hidden="true">
                  {badges.has_other_unread && <i className="project-unread-dot" />}
                  {badges.done > 0 && <i className="project-count done">{badges.done}</i>}
                  {badges.working > 0 && <i className="project-count working">{badges.working}</i>}
                  {badges.running_commands > 0 && <i className="project-count command">{badges.running_commands}</i>}
                </span>
                <span className="project-label">{project.title}</span>
              </button>
            );
          })}
        </div>
        <button
          className="project-add-button"
          aria-label="New project"
          disabled={!selectedProject || connection !== "connected" || creating}
          onClick={() => void createProject()}
        >＋</button>
        <span
          className={`header-connection ${connection}`}
          role="status"
          aria-label={resyncing ? "Resyncing" : connectionLabels[connection]}
          title={resyncing ? "Resyncing" : connectionLabels[connection]}
        ><span className={`activity-dot ${resyncing ? "reconnecting" : connection}`} /></span>
        <button className="icon-button" aria-label="Usage and settings" onClick={() => setUsageOpen(true)}>•••</button>
      </header>

      <main className="focus-area">
        {terminalSnapshot ? (
          <TerminalSurface
            ref={terminalSurface}
            snapshot={terminalSnapshot}
            bus={terminalBus}
            canResize={ownsWriterLease}
            onViewport={rememberTerminalViewport}
            onFocus={acquireLease}
            onStreamGap={() => {
              targetSelectionInFlight.current = undefined;
              terminalSnapshotRef.current = undefined;
              resetTerminalPreparation();
              setTerminalSnapshot(undefined);
            }}
            onData={(data) => {
              sendRawInput(data);
            }}
            onResize={(columns, rows) => {
              resizeSelectedTerminal(columns, rows);
            }}
          />
        ) : (
          <EmptyFocus
            connected={connection === "connected"}
            hasProjects={projects.length > 0}
            startingSession={Boolean(selectedProject?.tabs.some((tab) =>
              tab.panes.some((pane) => !pane.dimensions),
            ))}
            onNew={(kind) => {
              setNewMode("create");
              setNewKind(kind);
              setNewCwd(selectedLocalCwd ?? "");
              setNewOpen(true);
            }}
          />
        )}
      </main>

      <footer className="keyboard-accessory" aria-label="Terminal keyboard tools">
        <input
          ref={fileInput}
          id="remote-control-attachment"
          name="remote-control-attachment"
          type="file"
          hidden
          accept="image/*,.txt,.md,.json,.csv,.pdf,.zip"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) void upload(file);
            event.target.value = "";
          }}
        />
        <div className="keyboard-accessory-row">
          <button className="attach-button" aria-label="Attach photo or file" disabled={!canWrite || uploadActive} onClick={() => fileInput.current?.click()}>＋</button>
          <div className="quick-inserts" aria-label="Clinch quick inserts">
            {quickInserts.map((item) => (
              <button
                key={`${item.id}:${item.configuration_revision}`}
                onPointerDown={(event) => event.preventDefault()}
                onClick={() => void submitQuickInsert(item.id, item.configuration_revision)}
                disabled={!canWrite || quickInsertBusy}
              >
                {item.label}
              </button>
            ))}
          </div>
          <button className="keyboard-dismiss" aria-label="Close keyboard" disabled={!terminalSnapshot} onClick={() => terminalSurface.current?.blur()}>⌄</button>
        </div>
        <div className="terminal-key-row" aria-label="Terminal keys">
          {([
            ["escape", "esc", "Escape"],
            ["tab", "tab", "Tab"],
            ["arrow_left", "←", "Left arrow"],
            ["arrow_up", "↑", "Up arrow"],
            ["arrow_down", "↓", "Down arrow"],
            ["arrow_right", "→", "Right arrow"],
          ] as const).map(([key, label, accessibleLabel]) => (
            <button
              key={key}
              aria-label={accessibleLabel}
              className={label.length > 1 ? "text-key" : undefined}
              disabled={!canWrite}
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => void sendTerminalKey(key)}
            >{label}</button>
          ))}
        </div>
        {(leaseMessage || uploadActive || uploadRetryFile) && (
          <div className="keyboard-status">
            <span>{uploadActive ? `Uploading ${Math.round((uploadProgress ?? 0) * 100)}%` : uploadRetryFile ? `Ready to retry ${uploadRetryFile.name}` : leaseMessage}</span>
            {uploadActive ? (
              <button onClick={cancelUpload}>Cancel</button>
            ) : uploadRetryFile ? (
              <button onClick={() => void upload(uploadRetryFile)} disabled={!canWrite}>Retry</button>
            ) : null}
          </div>
        )}
      </footer>

      {drawerOpen && (
        <>
          <button className="drawer-scrim" aria-label="Close tab drawer" onClick={() => setDrawerOpen(false)} />
          <aside className="tab-drawer" aria-label="Current project sessions">
            <div className="drawer-heading"><strong>Open sessions</strong><button aria-label="Close drawer" onClick={() => setDrawerOpen(false)}>×</button></div>
            {selectedProject ? (
              <section key={selectedProject.id}>
                <h2>{selectedProject.title}</h2>
                {selectedProject.tabs.map((tab) => {
                  const activity = drawerTabActivity(tab);
                  return (
                  <div className="drawer-tab-group" key={tab.id}>
                    <div className="drawer-tab-meta">
                      <span>{tab.kind.replace("_", " ")}</span>
                      {activity && <i className={activity.className} aria-label={activity.label} title={activity.label} />}
                    </div>
                    {tab.panes.map((pane) => {
                      const target = targetFor(selectedProject, tab, pane, snapshot!.host.app_instance_id);
                      return (
                        <button className={selectedTarget && targetKey(target) === targetKey(selectedTarget) ? "active" : ""} key={pane.id} disabled={!pane.dimensions} onClick={() => pane.dimensions && selectTarget(target)}>
                          <span>{pane.title || tab.title}</span>
                          <small>{pane.dimensions ? pane.cwd ?? tab.remote_host ?? pane.agent_state ?? "Terminal" : "Not controllable on phone"}</small>
                        </button>
                      );
                    })}
                  </div>
                  );
                })}
                {selectedProject.tabs.length === 0 && <p className="muted">No sessions in this project yet.</p>}
              </section>
            ) : <p className="muted">Choose a project to see its sessions.</p>}
            <button className="drawer-new" onClick={() => {
              setDrawerOpen(false);
              setNewMode("create");
              setNewKind("terminal");
              setNewCwd(selectedLocalCwd ?? "");
              setNewOpen(true);
            }}>＋ New session</button>
          </aside>
        </>
      )}

      {usageOpen && (
        <Sheet title="Usage & connection" onClose={() => setUsageOpen(false)}>
          <div className="sheet-connection">
            <span className={`activity-dot ${connection}`} />{connectionLabels[connection]}
            <small>
              {identity?.deviceName ?? "This phone"} · {snapshot?.host.name ?? "Mac"} · {connectionPathLabel(snapshot?.host.connection_path)}
              {connectionDetail ? ` · ${connectionDetail}` : ""}
            </small>
          </div>
          <h3>Claude Code & Codex</h3>
          {(snapshot?.usage.length ?? 0) === 0 ? <p className="muted">No local usage snapshot is available yet.</p> : snapshot?.usage.map((usage) => (
            <div className="usage-card" key={usage.provider}>
              <div><strong>{usage.provider === "claude_code" ? "Claude Code" : "Codex"}</strong><span>{usage.state}</span></div>
              {usage.limit_windows.length > 0 ? (
                <div className="usage-limit-list">
                  {usage.limit_windows.map((window) => (
                    <div className="usage-limit" key={window.label}>
                      <div><span>{window.label}</span><strong>{Math.round(window.used_percent)}%</strong></div>
                      <div className="usage-track"><i style={{ width: `${Math.max(0, Math.min(100, window.used_percent))}%` }} /></div>
                      {window.resets_at && <small>Resets {new Date(window.resets_at).toLocaleString()}</small>}
                    </div>
                  ))}
                </div>
              ) : usage.used_percent != null ? (
                <div className="usage-track"><i style={{ width: `${Math.max(0, Math.min(100, usage.used_percent))}%` }} /></div>
              ) : null}
              <div className="usage-token-grid">
                {usage.token_windows.map((window) => (
                  <div key={window.label}>
                    <span>{window.label}</span>
                    <strong>{formatTokens(window.input_tokens + window.output_tokens)} I/O</strong>
                    <small>
                      {formatTokens(window.cache_read_tokens + window.cache_write_tokens)} cache
                      {window.estimated_cost_usd > 0 ? ` · $${window.estimated_cost_usd.toFixed(2)}` : ""}
                    </small>
                  </div>
                ))}
              </div>
              <small>{usage.updated_at ? `Updated ${new Date(usage.updated_at).toLocaleTimeString()}` : usage.source}</small>
            </div>
          ))}
          <p className="privacy-copy">Terminal data travels directly through your tailnet. Clinch has no account, analytics, or hosted relay in this connection.</p>
          <p className="muted">On iPhone or iPad, use Chrome or Safari&apos;s Share menu → Add to Home Screen, then open the Clinch icon for a full-screen app without browser bars. Installation is optional.</p>
          <a className="connection-help" href="https://clinch.sh/remote-control" target="_blank" rel="noreferrer">Connection help & security guide ↗</a>
          <button className="secondary-wide" onClick={() => {
            client.current?.stop();
            setConnection("mac_offline");
            setConnectionDetail("Disconnected on this phone until the page is reopened.");
            setUsageOpen(false);
          }}>Disconnect for now</button>
          <button className="danger-wide" onClick={() => void clearIdentity().then(() => location.reload())}>Forget this phone&apos;s key</button>
        </Sheet>
      )}

      {newOpen && (
        <Sheet title="New session" onClose={() => setNewOpen(false)}>
          <div className="segmented session-mode">
            <button className={newMode === "create" ? "active" : ""} onClick={() => setNewMode("create")}>Start new</button>
            <button className={newMode === "resume" ? "active" : ""} onClick={() => {
              setNewMode("resume");
              const first = snapshot?.recent_agent_sessions[0];
              if (first) {
                setNewResumeId(first.durable_session_id);
                setNewKind(first.provider);
                setNewCwd(first.cwd ?? newCwd);
              } else if (newKind === "terminal") {
                setNewKind("claude_code");
              }
            }}>Resume recent</button>
          </div>
          {newMode === "create" ? (
            <div className="segmented">
              {(["terminal", "claude_code", "codex"] as SessionKind[]).map((kind) => (
                <button className={newKind === kind ? "active" : ""} key={kind} onClick={() => setNewKind(kind)}>{kind.replace("_", " ")}</button>
              ))}
            </div>
          ) : (
            <>
              <div className="segmented session-mode">
                {(["claude_code", "codex"] as SessionKind[]).map((kind) => (
                  <button className={newKind === kind ? "active" : ""} key={kind} onClick={() => setNewKind(kind)}>{kind.replace("_", " ")}</button>
                ))}
              </div>
              {snapshot && snapshot.recent_agent_sessions.length > 0 && (
                <div className="recent-sessions" aria-label="Recent agent conversations">
                  {snapshot.recent_agent_sessions.map((session) => (
                    <button
                      className={newResumeId === session.durable_session_id ? "active" : ""}
                      key={`${session.provider}:${session.durable_session_id}`}
                      onClick={() => {
                        setNewResumeId(session.durable_session_id);
                        setNewKind(session.provider);
                        setNewCwd(session.cwd ?? newCwd);
                      }}
                    >
                      <span>{session.title}</span>
                      <small>{session.provider.replace("_", " ")}{session.started_at ? ` · ${new Date(session.started_at).toLocaleDateString()}` : ""}</small>
                    </button>
                  ))}
                </div>
              )}
              <label className="field-label">Session ID<input value={newResumeId} onChange={(event) => setNewResumeId(event.target.value)} placeholder="Choose a recent conversation or paste its ID" /></label>
            </>
          )}
          <label className="field-label">Project<select value={selectedProject?.id ?? ""} disabled><option>{selectedProject?.title ?? "Select a project from the top bar"}</option></select></label>
          <label className="field-label">Working directory (optional)<input value={newCwd} onChange={(event) => setNewCwd(event.target.value)} placeholder="Use Clinch's default directory" /></label>
          {newMode === "create" && newKind !== "terminal" && <label className="field-label">Initial prompt (optional)<textarea value={newPrompt} onChange={(event) => setNewPrompt(event.target.value)} rows={4} /></label>}
          <button className="primary-wide" onClick={() => void createSession()} disabled={creating || (newMode === "resume" && (!newResumeId.trim() || !newCwd.trim()))}>{creating ? "Opening…" : newMode === "resume" ? "Resume" : "Create"} on {snapshot?.host.name ?? "Mac"}</button>
        </Sheet>
      )}

      {notice && <button className="notice" role="alert" onClick={() => setNotice(undefined)}>{notice}<span>×</span></button>}
    </div>
  );
}

function PairingScreen({ state, message }: { state: BootState; message: string }) {
  return (
    <main className="pairing-screen">
      <ClinchMark className="pairing-mark" />
      <p className="eyebrow">Clinch Remote Control</p>
      <h1>{state === "needs_qr" ? "Scan once. Stay connected." : state === "error" ? "Couldn’t connect" : state === "waiting_approval" ? "Approve on your Mac" : "Securing this phone"}</h1>
      <p>{message}</p>
      {(state === "loading" || state === "waiting_approval") && <div className="spinner" aria-label="Working" />}
      {state === "error" && <button className="primary-wide" onClick={() => location.reload()}>Try again</button>}
      <div className="pairing-security"><span>◇</span><div><strong>Two private gates</strong><small>Your tailnet admits the phone; Clinch separately verifies this device key. No Clinch sign-in or relay.</small></div></div>
    </main>
  );
}

function EmptyFocus({ connected, hasProjects, startingSession, onNew }: { connected: boolean; hasProjects: boolean; startingSession: boolean; onNew: (kind: SessionKind) => void }) {
  const heading = !connected
    ? "Waiting for your Mac"
    : startingSession
      ? "Starting terminal…"
      : hasProjects
        ? "Choose a live session"
        : "Your Mac is ready";
  const detail = !connected
    ? "Clinch will reconnect automatically when the Mac wakes and comes online."
    : startingSession
      ? "Clinch is finishing the private shell setup before terminal output is shared."
      : "Open an existing tab from the drawer or start something new without leaving focus mode.";
  return (
    <div className="empty-focus">
      <div className="empty-orbit"><i /><i /><i /></div>
      <h1>{heading}</h1>
      <p>{detail}</p>
      {connected && !startingSession && <div className="empty-actions"><button onClick={() => onNew("terminal")}>Terminal</button><button onClick={() => onNew("claude_code")}>Claude Code</button><button onClick={() => onNew("codex")}>Codex</button></div>}
    </div>
  );
}

function Sheet({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  return (
    <>
      <button className="sheet-scrim" aria-label={`Close ${title}`} onClick={onClose} />
      <section className="sheet" role="dialog" aria-modal="true" aria-label={title}>
        <div className="sheet-handle" />
        <header><h2>{title}</h2><button aria-label="Close" onClick={onClose}>×</button></header>
        <div className="sheet-body">{children}</div>
      </section>
    </>
  );
}
