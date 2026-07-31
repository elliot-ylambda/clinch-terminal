import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { ClientMessage } from "../generated/types/ClientMessage";
import type { ConnectionState } from "../generated/types/ConnectionState";
import type { PaneSnapshot } from "../generated/types/PaneSnapshot";
import type { ProtocolErrorCode } from "../generated/types/ProtocolErrorCode";
import type { ProjectSnapshot } from "../generated/types/ProjectSnapshot";
import type { ServerEnvelope } from "../generated/types/ServerEnvelope";
import type { SessionKind } from "../generated/types/SessionKind";
import type { TabSnapshot } from "../generated/types/TabSnapshot";
import type { TargetRef } from "../generated/types/TargetRef";
import type { TerminalSnapshot } from "../generated/types/TerminalSnapshot";
import type { WorkspaceSnapshot } from "../generated/types/WorkspaceSnapshot";
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
  loadPreferences,
  savePendingPairing,
  savePreferences,
  type DeviceIdentity,
  type MobilePreferences,
} from "../protocol/storage";
import { clearPairingFragment, takePairingFragment, type PairingFragment } from "../protocol/urls";
import { TerminalBus } from "../terminal/bus";
import { TerminalSurface } from "../terminal/TerminalSurface";

type BootState = "loading" | "waiting_approval" | "needs_qr" | "ready" | "error";
type NewSessionMode = "create" | "resume";

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

export function shouldResynchronizeWorkspace(code: ProtocolErrorCode): boolean {
  return code === "revision_conflict" || code === "target_gone" || code === "resync_required";
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

export function App() {
  const pairingFragment = useRef<PairingFragment | null>(initialPairingFragment);

  const [boot, setBoot] = useState<BootState>("loading");
  const [bootMessage, setBootMessage] = useState("Opening your private Clinch connection…");
  const [identity, setIdentity] = useState<DeviceIdentity>();
  const [preferences, setPreferences] = useState<MobilePreferences>({ key: "preferences", oneTapQuickInserts: false });
  const [connection, setConnection] = useState<ConnectionState>("reconnecting");
  const [connectionDetail, setConnectionDetail] = useState<string>();
  const [snapshot, setSnapshot] = useState<WorkspaceSnapshot>();
  const [selectedProjectId, setSelectedProjectId] = useState<string>();
  const selectedProjectIdRef = useRef<string | undefined>(undefined);
  selectedProjectIdRef.current = selectedProjectId;
  const [selectedTarget, setSelectedTarget] = useState<TargetRef>();
  const selectedTargetRef = useRef<TargetRef | undefined>(undefined);
  selectedTargetRef.current = selectedTarget;
  const [terminalSnapshot, setTerminalSnapshot] = useState<TerminalSnapshot>();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [usageOpen, setUsageOpen] = useState(false);
  const [newOpen, setNewOpen] = useState(false);
  const [newMode, setNewMode] = useState<NewSessionMode>("create");
  const [newKind, setNewKind] = useState<SessionKind>("terminal");
  const [newCwd, setNewCwd] = useState("");
  const [newPrompt, setNewPrompt] = useState("");
  const [newResumeId, setNewResumeId] = useState("");
  const [creating, setCreating] = useState(false);
  const [composer, setComposer] = useState("");
  const [resyncing, setResyncing] = useState(false);
  const [notice, setNotice] = useState<string>();
  const [uploadProgress, setUploadProgress] = useState<number>();
  const [uploadActive, setUploadActive] = useState(false);
  const [uploadRetryFile, setUploadRetryFile] = useState<File>();
  const client = useRef<CompanionClient | undefined>(undefined);
  const terminalBus = useMemo(() => new TerminalBus(), []);
  const fileInput = useRef<HTMLInputElement>(null);
  const activeUploadId = useRef<string | undefined>(undefined);
  const uploadCancelled = useRef(false);
  const resyncRequested = useRef(false);
  const creationInFlight = useRef(false);
  const autoOpenedProjects = useRef(new Set<string>());

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        setPreferences(await loadPreferences());
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
        setSnapshot(payload.data);
        {
          const selection = synchronizedSelection(
            payload.data,
            selectedProjectIdRef.current,
            selectedTargetRef.current,
          );
          selectedProjectIdRef.current = selection.projectId;
          selectedTargetRef.current = selection.target;
          setSelectedProjectId(selection.projectId);
          setSelectedTarget(selection.target);
        }
        break;
      case "workspace_changed":
        resyncRequested.current = false;
        setResyncing(false);
        setSnapshot(payload.data.snapshot);
        {
          const selection = synchronizedSelection(
            payload.data.snapshot,
            selectedProjectIdRef.current,
            selectedTargetRef.current,
          );
          selectedProjectIdRef.current = selection.projectId;
          selectedTargetRef.current = selection.target;
          setSelectedProjectId(selection.projectId);
          setSelectedTarget(selection.target);
        }
        break;
      case "terminal_snapshot":
        setTerminalSnapshot(payload.data);
        break;
      case "terminal_stream_closed":
        setNotice(payload.data.reason);
        setTerminalSnapshot(undefined);
        break;
      case "quick_insert_preview":
        setComposer(payload.data.text);
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
        setConnection(payload.data);
        break;
      case "error":
        if (payload.data.retryable && shouldResynchronizeWorkspace(payload.data.code)) {
          setResyncing(true);
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
  }, []);

  useEffect(() => {
    if (boot !== "ready" || !identity?.deviceId) return;
    const companion = new CompanionClient(identity, {
      connection: (state, detail) => {
        setConnection(state);
        setConnectionDetail(detail);
        if (state !== "connected") setTerminalSnapshot(undefined);
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
  }, [boot, handleEnvelope, identity, terminalBus]);

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
    try {
      client.current?.send({
        type: "select_target",
        data: { target: selectedTarget, workspace_revision: snapshot.revision },
      });
    } catch (error) {
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
        selectedProjectIdRef.current = target.project_id;
        setSelectedProjectId(target.project_id);
      }
      selectedTargetRef.current = target;
      setSelectedTarget(target);
      setDrawerOpen(false);
      setTerminalSnapshot(undefined);
    },
    [selectedTarget],
  );

  const acquireLease = useCallback(() => {
    if (!snapshot || !selectedTarget || connection !== "connected") return;
    try {
      client.current?.send({
        type: "acquire_writer_lease",
        data: { target: selectedTarget, workspace_revision: snapshot.revision },
      });
    } catch {
      // Connection state already explains why input is unavailable.
    }
  }, [connection, selectedTarget, snapshot]);

  const sendComposer = useCallback(async () => {
    if (!snapshot || !selectedTarget || !composer.trim() || !canWrite) return;
    const text = composer;
    try {
      acquireLease();
      const response = await client.current?.sendAndWait({
        type: "submit_composer_text",
        data: { target: selectedTarget, workspace_revision: snapshot.revision, text },
      });
      if (response?.payload.type === "error") throw new Error(response.payload.data.message);
      setComposer("");
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
    }
  }, [acquireLease, canWrite, composer, selectedTarget, snapshot]);

  const previewQuickInsert = useCallback(
    async (itemId: string, configurationRevision: number) => {
      if (!snapshot || !selectedTarget || !canWrite) return;
      try {
        const request: ClientMessage = preferences.oneTapQuickInserts
          ? {
              type: "quick_insert_submit",
              data: {
                target: selectedTarget,
                workspace_revision: snapshot.revision,
                item_id: itemId,
                configuration_revision: configurationRevision,
              },
            }
          : {
              type: "quick_insert_preview",
              data: {
                target: selectedTarget,
                workspace_revision: snapshot.revision,
                item_id: itemId,
                configuration_revision: configurationRevision,
              },
            };
        const response = await client.current?.sendAndWait(request);
        if (response?.payload.type === "error") throw new Error(response.payload.data.message);
      } catch (error) {
        setNotice(error instanceof Error ? error.message : String(error));
      }
    },
    [canWrite, preferences.oneTapQuickInserts, selectedTarget, snapshot],
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
    const hasLiveSession = selectedProject.tabs.some((tab) =>
      tab.panes.some((pane) => Boolean(pane.dimensions)),
    );
    const key = `${snapshot.host.app_instance_id}:${selectedProject.id}`;
    if (hasLiveSession || autoOpenedProjects.current.has(key)) return;
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
        <ClinchMark className="wordmark" />
        <div className="project-scroll">
          {projects.map((project) => (
            <button
              className={`project-pill ${project.id === selectedProject?.id ? "active" : ""}`}
              key={project.id}
              onClick={() => {
                if (!snapshot) return;
                selectedProjectIdRef.current = project.id;
                setSelectedProjectId(project.id);
                const target = firstTarget(snapshot, project);
                if (target) {
                  selectTarget(target);
                } else {
                  selectTarget(undefined);
                }
              }}
            >
              <span className={`activity-dot ${project.activity}`} />
              {project.title}
            </button>
          ))}
          <button
            className="project-add-button"
            aria-label="New project"
            disabled={!selectedProject || connection !== "connected" || creating}
            onClick={() => void createProject()}
          >＋</button>
        </div>
        <button className="icon-button" aria-label="Usage and settings" onClick={() => setUsageOpen(true)}>•••</button>
      </header>

      <header className="session-header">
        <button className="icon-button" aria-label="Open project and tab drawer" onClick={() => setDrawerOpen(true)}>☰</button>
        <div className="target-title">
          <strong>{selected.tab?.title ?? "Clinch Remote Control"}</strong>
          <span>{selectedProject?.title ?? snapshot?.host.name ?? "Waiting for Mac"}</span>
        </div>
        <div className={`connection-chip ${connection}`} role="status">
          <span />{resyncing ? "Resyncing" : connectionLabels[connection]}
        </div>
        <button
          className="new-button"
          disabled={!selectedProject || connection !== "connected" || creating}
          onClick={() => {
            if (!selectedProject) return;
            void createTab(selectedProject.id, "terminal", selectedLocalCwd).catch((error) => {
              setNotice(error instanceof Error ? error.message : String(error));
            });
          }}
        >{creating ? "Opening…" : "＋ New"}</button>
      </header>

      <main className="focus-area">
        {terminalSnapshot ? (
          <TerminalSurface
            snapshot={terminalSnapshot}
            bus={terminalBus}
            canResize={ownsWriterLease}
            onFocus={acquireLease}
            onData={(data) => {
              if (!snapshot || !selectedTarget || !canWrite) return;
              acquireLease();
              try {
                client.current?.send({
                  type: "raw_terminal_input",
                  data: {
                    target: selectedTarget,
                    workspace_revision: snapshot.revision,
                    data_base64: bytesToBase64(new TextEncoder().encode(data)),
                  },
                });
              } catch (error) {
                setNotice(error instanceof Error ? error.message : String(error));
              }
            }}
            onResize={(columns, rows) => {
              if (!snapshot || !selectedTarget || connection !== "connected" || !ownsWriterLease) return;
              try {
                client.current?.send({
                  type: "terminal_resize",
                  data: {
                    target: selectedTarget,
                    workspace_revision: snapshot.revision,
                    dimensions: { columns, rows },
                  },
                });
              } catch {
                // Resizing is best-effort and must never queue input.
              }
            }}
          />
        ) : (
          <EmptyFocus
            connected={connection === "connected"}
            hasProjects={projects.length > 0}
            onNew={(kind) => {
              setNewMode("create");
              setNewKind(kind);
              setNewCwd(selectedLocalCwd ?? "");
              setNewOpen(true);
            }}
          />
        )}
      </main>

      <footer className="composer-shell">
        {quickInserts.length > 0 && (
          <div className="quick-inserts" aria-label="Clinch quick inserts">
            {quickInserts.map((item) => (
              <button key={`${item.id}:${item.configuration_revision}`} onClick={() => void previewQuickInsert(item.id, item.configuration_revision)} disabled={!canWrite}>
                {item.label}
              </button>
            ))}
          </div>
        )}
        <div className="composer-row">
          <input
            ref={fileInput}
            type="file"
            hidden
            accept="image/*,.txt,.md,.json,.csv,.pdf,.zip"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) void upload(file);
              event.target.value = "";
            }}
          />
          <button className="attach-button" aria-label="Attach photo or file" disabled={!canWrite || uploadActive} onClick={() => fileInput.current?.click()}>＋</button>
          <textarea
            value={composer}
            onChange={(event) => setComposer(event.target.value)}
            onFocus={acquireLease}
            placeholder={canWrite ? "Type for this exact session…" : leaseMessage ?? connectionLabels[connection]}
            aria-label="Command or agent prompt"
            disabled={!canWrite}
            rows={1}
          />
          <button className="send-button" onClick={() => void sendComposer()} disabled={!canWrite || !composer.trim()}>Send</button>
        </div>
        {(leaseMessage || uploadActive || uploadRetryFile) && (
          <div className="composer-status">
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
                <h2><span className={`activity-dot ${selectedProject.activity}`} />{selectedProject.title}</h2>
                {selectedProject.tabs.map((tab) => (
                  <div className="drawer-tab-group" key={tab.id}>
                    <div className="drawer-tab-meta"><span>{tab.kind.replace("_", " ")}</span>{tab.unread && <i />}</div>
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
                ))}
                {selectedProject.tabs.length === 0 && <p className="muted">No sessions in this project yet.</p>}
              </section>
            ) : <p className="muted">Choose a project to see its sessions.</p>}
            <button className="drawer-new" onClick={() => {
              setNewMode("create");
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
          <label className="preference-row">
            <span><strong>One-tap quick inserts</strong><small>Off by default. Review text in the composer before Send.</small></span>
            <input type="checkbox" checked={preferences.oneTapQuickInserts} onChange={(event) => {
              const next = { ...preferences, oneTapQuickInserts: event.target.checked };
              setPreferences(next);
              void savePreferences(next);
            }} />
          </label>
          <p className="privacy-copy">Terminal data travels directly through your tailnet. Clinch has no account, analytics, or hosted relay in this connection.</p>
          <p className="muted">On iPhone or iPad, use Safari&apos;s Share menu → Add to Home Screen for an app-like launch. Installation is optional.</p>
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

function EmptyFocus({ connected, hasProjects, onNew }: { connected: boolean; hasProjects: boolean; onNew: (kind: SessionKind) => void }) {
  return (
    <div className="empty-focus">
      <div className="empty-orbit"><i /><i /><i /></div>
      <h1>{connected ? (hasProjects ? "Choose a live session" : "Your Mac is ready") : "Waiting for your Mac"}</h1>
      <p>{connected ? "Open an existing tab from the drawer or start something new without leaving focus mode." : "Clinch will reconnect automatically when the Mac wakes and comes online."}</p>
      {connected && <div className="empty-actions"><button onClick={() => onNew("terminal")}>Terminal</button><button onClick={() => onNew("claude_code")}>Claude Code</button><button onClick={() => onNew("codex")}>Codex</button></div>}
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
