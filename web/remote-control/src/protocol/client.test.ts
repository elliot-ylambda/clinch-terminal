import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CompanionClient, isPermanentAuthorizationError } from "./client";
import { authenticate } from "./pairing";
import type { DeviceIdentity } from "./storage";

vi.mock("./pairing", () => ({ authenticate: vi.fn() }));

describe("Remote Control reconnect authorization", () => {
  it("stops reconnecting for revoked or expired device authorization", () => {
    expect(isPermanentAuthorizationError("This phone's authorization was revoked.")).toBe(true);
    expect(isPermanentAuthorizationError("The authorization record expired.")).toBe(true);
  });

  it("keeps reconnecting for an offline Mac or a transient network failure", () => {
    expect(isPermanentAuthorizationError("Failed to fetch")).toBe(false);
    expect(isPermanentAuthorizationError("The Mac did not answer in time")).toBe(false);
  });
});

class FakeSocket {
  static readonly OPEN = 1;
  static instances: FakeSocket[] = [];
  readyState = 0;
  binaryType = "";
  onopen: (() => void) | null = null;
  onmessage: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;

  constructor() {
    FakeSocket.instances.push(this);
  }

  open() {
    this.readyState = FakeSocket.OPEN;
    this.onopen?.();
  }

  close() {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.onclose?.();
  }

  send() {}
}

describe("Remote Control session reuse", () => {
  const identity = { deviceId: "device" } as DeviceIdentity;
  const authenticated = { replayed_from_sequence: null } as Awaited<ReturnType<typeof authenticate>>;

  beforeEach(() => {
    FakeSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.mocked(authenticate).mockReset().mockResolvedValue(authenticated);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  async function connectedClient() {
    const client = new CompanionClient(identity, {
      connection: () => {},
      envelope: () => {},
      terminal: () => {},
      resync: () => {},
    });
    client.start();
    await vi.waitFor(() => expect(FakeSocket.instances).toHaveLength(1));
    FakeSocket.instances[0].open();
    return client;
  }

  it("reopens the socket on the existing session without re-authenticating", async () => {
    const client = await connectedClient();
    expect(authenticate).toHaveBeenCalledTimes(1);

    // Returning to the app after it was pocketed.
    FakeSocket.instances[0].close();
    (client as unknown as { online: () => void }).online();
    await vi.waitFor(() => expect(FakeSocket.instances).toHaveLength(2));
    expect(authenticate).toHaveBeenCalledTimes(1);
    client.stop();
  });

  it("authenticates once when the Mac refuses the saved session", async () => {
    const client = await connectedClient();
    FakeSocket.instances[0].close();
    (client as unknown as { online: () => void }).online();
    await vi.waitFor(() => expect(FakeSocket.instances).toHaveLength(2));

    // Upgrade rejected (401): closed before it ever opened.
    FakeSocket.instances[1].close();
    await vi.waitFor(() => expect(FakeSocket.instances).toHaveLength(3));
    expect(authenticate).toHaveBeenCalledTimes(2);

    // A fresh authentication that still fails goes through backoff, not another immediate retry.
    FakeSocket.instances[2].close();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(FakeSocket.instances).toHaveLength(3);
    expect(authenticate).toHaveBeenCalledTimes(2);
    client.stop();
  });

  it("authenticates again once the saved session is too old to reuse", async () => {
    const client = await connectedClient();
    const now = Date.now();
    vi.spyOn(Date, "now").mockReturnValue(now + 15 * 60_000);
    FakeSocket.instances[0].close();
    (client as unknown as { online: () => void }).online();
    await vi.waitFor(() => expect(FakeSocket.instances).toHaveLength(2));
    expect(authenticate).toHaveBeenCalledTimes(2);
    vi.mocked(Date.now).mockRestore();
    client.stop();
  });
});
