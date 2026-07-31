import type { ClientEnvelope } from "../generated/types/ClientEnvelope";
import type { ClientMessage } from "../generated/types/ClientMessage";
import type { ConnectionState } from "../generated/types/ConnectionState";
import type { ServerEnvelope } from "../generated/types/ServerEnvelope";
import { PROTOCOL_VERSION } from "../generated/constants";
import { decodeTerminalOutput, type TerminalOutputFrame } from "./binary";
import { authenticate } from "./pairing";
import type { DeviceIdentity } from "./storage";
import { websocketUrl } from "./urls";

interface PendingRequest {
  resolve: (envelope: ServerEnvelope) => void;
  reject: (error: Error) => void;
  timeout: number;
}

export interface CompanionClientEvents {
  connection: (state: ConnectionState, detail?: string) => void;
  envelope: (envelope: ServerEnvelope) => void;
  terminal: (frame: TerminalOutputFrame) => void;
  resync: (required: boolean) => void;
}

export class CompanionClient {
  private socket?: WebSocket;
  private stopped = true;
  private connecting = false;
  private reconnectAttempt = 0;
  private reconnectTimer?: number;
  private pingTimer?: number;
  private lastSequence = 0;
  private pending = new Map<string, PendingRequest>();

  constructor(
    private identity: DeviceIdentity,
    private readonly events: CompanionClientEvents,
  ) {}

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    document.addEventListener("visibilitychange", this.visibilityChanged);
    window.addEventListener("online", this.online);
    window.addEventListener("offline", this.offline);
    void this.connect();
  }

  stop(): void {
    this.stopped = true;
    document.removeEventListener("visibilitychange", this.visibilityChanged);
    window.removeEventListener("online", this.online);
    window.removeEventListener("offline", this.offline);
    window.clearTimeout(this.reconnectTimer);
    window.clearInterval(this.pingTimer);
    this.socket?.close(1000, "Phone disconnected");
    this.rejectPending(new Error("Disconnected"));
  }

  updateIdentity(identity: DeviceIdentity): void {
    this.identity = identity;
  }

  send(payload: ClientMessage, requestId = crypto.randomUUID()): string {
    if (this.socket?.readyState !== WebSocket.OPEN) throw new Error("Mac is not connected");
    const envelope: ClientEnvelope = {
      version: PROTOCOL_VERSION,
      request_id: requestId,
      payload,
    };
    this.socket.send(JSON.stringify(envelope));
    return requestId;
  }

  sendAndWait(payload: ClientMessage, timeoutMs = 15_000): Promise<ServerEnvelope> {
    const requestId = crypto.randomUUID();
    return new Promise((resolve, reject) => {
      const timeout = window.setTimeout(() => {
        this.pending.delete(requestId);
        reject(new Error("The Mac did not answer in time"));
      }, timeoutMs);
      this.pending.set(requestId, { resolve, reject, timeout });
      try {
        this.send(payload, requestId);
      } catch (error) {
        window.clearTimeout(timeout);
        this.pending.delete(requestId);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  sendBinary(frame: ArrayBuffer): void {
    if (this.socket?.readyState !== WebSocket.OPEN) throw new Error("Mac is not connected");
    this.socket.send(frame);
  }

  private async connect(): Promise<void> {
    if (this.stopped || this.connecting || document.visibilityState === "hidden") return;
    if (!navigator.onLine) {
      this.events.connection("mac_offline", "Phone is offline");
      return;
    }
    this.connecting = true;
    this.events.connection(this.reconnectAttempt ? "reconnecting" : "reconnecting");
    try {
      const authenticated = await authenticate(this.identity, this.lastSequence);
      if (authenticated.replayed_from_sequence == null) this.lastSequence = 0;
      if (this.stopped) return;
      const socket = new WebSocket(websocketUrl());
      socket.binaryType = "arraybuffer";
      this.socket = socket;
      socket.onopen = () => {
        this.connecting = false;
        this.reconnectAttempt = 0;
        this.events.connection("connected");
        this.pingTimer = window.setInterval(() => {
          try {
            this.send({ type: "ping" });
          } catch {
            // The close handler owns reconnect behavior.
          }
        }, 20_000);
      };
      socket.onmessage = (event: MessageEvent<string | ArrayBuffer>) => this.received(event.data);
      socket.onerror = () => socket.close();
      socket.onclose = () => {
        window.clearInterval(this.pingTimer);
        if (this.socket === socket) this.socket = undefined;
        this.connecting = false;
        this.rejectPending(new Error("Connection to the Mac was interrupted"));
        if (!this.stopped) {
          // A 15-minute WebSocket authorization intentionally expires even though the paired
          // device remains valid. Re-authentication decides whether this was routine renewal or
          // an actual device revocation.
          this.scheduleReconnect();
        }
      };
    } catch (error) {
      this.connecting = false;
      const message = error instanceof Error ? error.message : String(error);
      if (isPermanentAuthorizationError(message)) {
        this.stopped = true;
        this.events.connection("authorization_revoked", message);
      } else {
        this.events.connection("mac_offline", message);
        this.scheduleReconnect();
      }
    }
  }

  private received(data: string | ArrayBuffer): void {
    if (data instanceof ArrayBuffer) {
      try {
        this.events.terminal(decodeTerminalOutput(data));
      } catch (error) {
        this.events.connection("version_incompatible", error instanceof Error ? error.message : String(error));
        this.socket?.close(1003, "Invalid binary protocol");
      }
      return;
    }
    let envelope: ServerEnvelope;
    try {
      envelope = JSON.parse(data) as ServerEnvelope;
    } catch {
      this.socket?.close(1003, "Invalid JSON protocol");
      return;
    }
    if (envelope.version !== PROTOCOL_VERSION) {
      this.events.connection("version_incompatible");
      this.socket?.close(1003, "Protocol version mismatch");
      return;
    }
    if (envelope.request_id) {
      const pending = this.pending.get(envelope.request_id);
      if (pending) {
        window.clearTimeout(pending.timeout);
        this.pending.delete(envelope.request_id);
        pending.resolve(envelope);
      }
    }
    if (envelope.sequence != null) {
      if (envelope.sequence <= this.lastSequence) return;
      if (this.lastSequence && envelope.sequence !== this.lastSequence + 1) {
        if (envelope.payload.type !== "snapshot") {
          this.events.resync(true);
          try {
            this.send({ type: "request_snapshot" });
          } catch {
            this.scheduleReconnect();
          }
          return;
        }
      }
      this.lastSequence = envelope.sequence;
      this.events.resync(false);
    }
    this.events.envelope(envelope);
  }

  private scheduleReconnect(): void {
    if (this.stopped || this.reconnectTimer) return;
    this.events.connection("reconnecting");
    const delay = Math.min(15_000, 750 * 2 ** this.reconnectAttempt++);
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = undefined;
      void this.connect();
    }, delay);
  }

  private rejectPending(error: Error): void {
    for (const pending of this.pending.values()) {
      window.clearTimeout(pending.timeout);
      pending.reject(error);
    }
    this.pending.clear();
  }

  private visibilityChanged = (): void => {
    if (document.visibilityState === "visible" && this.socket?.readyState !== WebSocket.OPEN) {
      window.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = undefined;
      void this.connect();
    }
  };

  private online = (): void => {
    window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    void this.connect();
  };

  private offline = (): void => {
    this.events.connection("mac_offline", "Phone is offline");
    this.socket?.close();
  };
}

export function isPermanentAuthorizationError(message: string): boolean {
  return /revok|pair|authoriz|not been approved|record expired/i.test(message);
}
