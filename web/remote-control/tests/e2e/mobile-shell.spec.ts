import { expect, test } from "@playwright/test";

async function installPairedPhone(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.evaluate(async () => {
    const keys = (await crypto.subtle.generateKey(
      { name: "ECDSA", namedCurve: "P-256" },
      // WebKit's test contexts intermittently reject structured-cloning a
      // non-extractable key into IndexedDB. Production identity creation and
      // its tests still require a non-extractable key; this fixture only needs
      // a persisted signing key so the paired shell can be exercised.
      true,
      ["sign", "verify"],
    )) as CryptoKeyPair;
    const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", keys.publicKey));
    let binary = "";
    for (const byte of publicKey) binary += String.fromCharCode(byte);
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open("clinch-remote-control", 1);
      request.onupgradeneeded = () => request.result.createObjectStore("local-device", { keyPath: "key" });
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    await new Promise<void>((resolve, reject) => {
      const transaction = db.transaction("local-device", "readwrite");
      transaction.objectStore("local-device").put({
        key: "identity",
        privateKey: keys.privateKey,
        publicKeyP256Raw: btoa(binary),
        deviceName: "Test iPhone",
        deviceId: "11111111-1111-4111-8111-111111111111",
        capabilities: ["view", "control", "create_session", "upload"],
      });
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error);
    });
    db.close();
  });

  await page.addInitScript(() => {
    const originalFetch = window.fetch.bind(window);
    window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = input instanceof Request ? input.url : String(input);
      if (url.includes("/api/v1/auth/challenge")) {
        const challenge = btoa(String.fromCharCode(...new Uint8Array(32)));
        return new Response(JSON.stringify({
          id: "22222222-2222-4222-8222-222222222222",
          device_id: "11111111-1111-4111-8111-111111111111",
          challenge,
          expires_at: "2099-01-01T00:00:00Z",
        }), { status: 200, headers: { "content-type": "application/json" } });
      }
      if (url.includes("/api/v1/auth/authenticate")) {
        return new Response(JSON.stringify({
          session_id: "33333333-3333-4333-8333-333333333333",
          device: {
            id: "11111111-1111-4111-8111-111111111111",
            name: "Test iPhone",
            platform: "ios",
            capabilities: ["view", "control", "create_session", "upload"],
            connected: false,
            last_seen_at: "2026-07-30T23:00:00Z",
          },
          expires_at: "2099-01-01T00:00:00Z",
          replayed_from_sequence: null,
        }), { status: 200, headers: { "content-type": "application/json" } });
      }
      return originalFetch(input, init);
    };

    const target = {
      app_instance_id: "44444444-4444-4444-8444-444444444444",
      project_id: "project-demo",
      tab_id: "tab-agent",
      pane_id: "pane-agent",
    };
    const snapshot = {
      revision: 7,
      sequence: 0,
      host: {
        app_instance_id: target.app_instance_id,
        name: "Test Mac",
        connection_path: "unknown",
        capabilities: ["view", "control", "create_session", "upload"],
      },
      projects: [{
        id: target.project_id,
        title: "Demo",
        order: 0,
        active: true,
        activity: "working",
        tabs: [{
          id: target.tab_id,
          title: "Ship remote control",
          kind: "claude_code",
          active: true,
          activity: "working",
          unread: true,
          remote_host: null,
          panes: [{
            id: target.pane_id,
            title: "Ship remote control",
            kind: "claude_code",
            cwd: "/Users/test/demo",
            active: true,
            agent_state: "working",
            dimensions: { columns: 80, rows: 24 },
            writer_lease: null,
            quick_inserts: [{
              id: "qi-1234",
              configuration_revision: 1,
              label: "Codex",
              kind: "built_in",
              submits_immediately: false,
            }],
          }],
        }],
      }, {
        id: "project-empty",
        title: "Empty project",
        order: 1,
        active: false,
        activity: "idle",
        tabs: [],
      }],
      active_target: target,
      usage: [{
        provider: "claude_code",
        state: "available",
        updated_at: "2026-07-30T23:00:00Z",
        reset_at: "2026-07-31T01:00:00Z",
        used_percent: 42,
        model: null,
        limit_windows: [{ label: "5-hour", used_percent: 42, resets_at: "2026-07-31T01:00:00Z" }],
        token_windows: [{
          label: "Today",
          input_tokens: 1200,
          output_tokens: 800,
          cache_read_tokens: 4000,
          cache_write_tokens: 100,
          estimated_cost_usd: 0.34,
        }],
        source: "Latest local Clinch usage snapshot",
        live_plan_gauges_enabled_on_mac: true,
      }],
      recent_agent_sessions: [{
        durable_session_id: "session-1234",
        provider: "claude_code",
        title: "Fix pairing flow",
        cwd: "/Users/test/demo",
        started_at: "2026-07-30T20:00:00Z",
      }],
      paired_devices: [],
    };

    class MockWebSocket {
      static readonly CONNECTING = 0;
      static readonly OPEN = 1;
      static readonly CLOSING = 2;
      static readonly CLOSED = 3;
      readyState = MockWebSocket.CONNECTING;
      binaryType: BinaryType = "arraybuffer";
      onopen: ((event: Event) => void) | null = null;
      onmessage: ((event: MessageEvent<string | ArrayBuffer>) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      onclose: ((event: CloseEvent) => void) | null = null;
      private sequence = 0;
      private selectAttempts = 0;

      constructor(_url: string | URL) {
        window.setTimeout(() => {
          this.readyState = MockWebSocket.OPEN;
          this.onopen?.(new Event("open"));
          this.emit({ type: "hello", data: { supported_versions: [1], host_name: "Test Mac" } });
          this.emit({ type: "snapshot", data: snapshot });
        }, 0);
      }

      send(data: string | ArrayBufferLike | Blob | ArrayBufferView) {
        if (typeof data !== "string") return;
        const envelope = JSON.parse(data) as {
          request_id: string;
          payload: {
            type: string;
            data?: { target?: typeof target; workspace_revision?: number };
          };
        };
        const commands = JSON.parse(localStorage.getItem("remote-command-types") ?? "[]") as string[];
        commands.push(envelope.payload.type);
        localStorage.setItem("remote-command-types", JSON.stringify(commands));
        const payloads = JSON.parse(localStorage.getItem("remote-command-payloads") ?? "[]") as object[];
        payloads.push(envelope.payload);
        localStorage.setItem("remote-command-payloads", JSON.stringify(payloads));
        if (envelope.payload.type === "select_target") {
          this.selectAttempts += 1;
          if (this.selectAttempts === 1) {
            snapshot.revision += 1;
            this.emit({
              type: "error",
              data: {
                code: "revision_conflict",
                message: "The workspace changed; refresh before sending input.",
                retryable: true,
                current_revision: snapshot.revision,
              },
            }, envelope.request_id);
            return;
          }
          this.emit({
            type: "terminal_snapshot",
            data: {
              target: envelope.payload.data?.target ?? target,
              stream_id: "55555555-5555-4555-8555-555555555555",
              workspace_revision: snapshot.revision,
              terminal_sequence: 0,
              data_base64: btoa("$ echo connected\r\nconnected\r\n"),
              dimensions: { columns: 80, rows: 24 },
            },
          }, envelope.request_id);
        } else if (envelope.payload.type === "ping") {
          this.emit({ type: "pong" }, envelope.request_id);
        } else if (envelope.payload.type === "request_snapshot") {
          this.emit({ type: "snapshot", data: snapshot }, envelope.request_id);
        } else if (envelope.payload.type === "create_session" || envelope.payload.type === "create_project") {
          snapshot.revision += 1;
          this.emit({ type: "command_accepted", data: { workspace_revision: snapshot.revision } }, envelope.request_id);
        } else if (envelope.payload.type === "quick_insert_preview") {
          if (envelope.payload.data?.workspace_revision !== snapshot.revision) {
            this.emit({
              type: "error",
              data: {
                code: "revision_conflict",
                message: "The workspace changed; refresh before sending input.",
                retryable: true,
                current_revision: snapshot.revision,
              },
            }, envelope.request_id);
            return;
          }
          this.emit({
            type: "quick_insert_preview",
            data: { item_id: "qi-1234", configuration_revision: 1, text: "codex" },
          }, envelope.request_id);
        } else {
          this.emit({ type: "command_accepted", data: { workspace_revision: snapshot.revision } }, envelope.request_id);
        }
      }

      close(code = 1000, reason = "") {
        if (this.readyState === MockWebSocket.CLOSED) return;
        this.readyState = MockWebSocket.CLOSED;
        this.onclose?.(new CloseEvent("close", { code, reason }));
      }

      private emit(payload: object, requestId: string | null = null) {
        this.sequence += 1;
        this.onmessage?.(new MessageEvent("message", {
          data: JSON.stringify({ version: 1, request_id: requestId, sequence: this.sequence, payload }),
        }));
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: MockWebSocket as unknown as typeof WebSocket,
    });
  });
  await page.reload();
}

test("unpaired phone gets a focused setup screen without horizontal overflow", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("Open Clinch Settings", { exact: false })).toBeVisible();
  const mark = page.getByRole("img", { name: "Clinch" });
  await expect(mark).toBeVisible();
  await expect(mark).toHaveAttribute("src", "./clinch-logo.svg");
  expect(await mark.evaluate((image: HTMLImageElement) => image.complete && image.naturalWidth > 0)).toBe(true);
  const brand = await page.evaluate(() => {
    const root = getComputedStyle(document.documentElement);
    return {
      accent: root.getPropertyValue("--accent").trim().toLowerCase(),
      background: root.getPropertyValue("--background").trim().toLowerCase(),
      fontFamily: root.fontFamily,
    };
  });
  expect(brand).toEqual({
    accent: "#bfff00",
    background: "#050712",
    fontFamily: expect.stringContaining("Inter Variable"),
  });
  const dimensions = await page.evaluate(() => ({
    client: document.documentElement.clientWidth,
    scroll: document.documentElement.scrollWidth,
  }));
  expect(dimensions.scroll).toBeLessThanOrEqual(dimensions.client);
});

test("a first-time QR scan creates a phone key and waits for explicit Mac approval", async ({ page }) => {
  await page.addInitScript(() => {
    window.fetch = async (input: RequestInfo | URL) => {
      const url = input instanceof Request ? input.url : String(input);
      if (url.includes("/api/v1/pair/claim")) {
        localStorage.setItem("pair-claim-count", String(Number(localStorage.getItem("pair-claim-count") ?? "0") + 1));
        return new Response(JSON.stringify({
          claim_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
          claim_secret: "single-use-claim-secret",
          device_name: "iPhone",
          public_key_fingerprint: "0123456789abcdef".repeat(4),
          expires_at: "2099-01-01T00:00:00Z",
        }), { status: 201, headers: { "content-type": "application/json" } });
      }
      if (url.includes("/api/v1/pair/status")) {
        return new Response(JSON.stringify({ status: "pending" }), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response(null, { status: 404 });
    };
  });

  await page.goto("/#bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb:qr-secret");
  await expect(page.getByRole("heading", { name: "Approve on your Mac" })).toBeVisible();
  await expect(page.getByText("Approve “iPhone” in Clinch on your Mac.")).toBeVisible();
  await expect(page.getByRole("img", { name: "Clinch" })).toBeVisible();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe("");

  await page.reload();
  await expect(page.getByRole("heading", { name: "Approve on your Mac" })).toBeVisible();
  await expect(page.getByText("Approve “iPhone” in Clinch on your Mac.")).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem("pair-claim-count"))).toBe("1");
});

test("paired phone exposes projects, drawer, usage, and recent-session resume", async ({ page }) => {
  await installPairedPhone(page);

  await expect(page.getByRole("button", { name: "Demo" })).toBeVisible();
  await expect(page.getByLabel("Selected Clinch terminal output")).toBeVisible();
  await expect(page.getByText("The workspace changed; refresh before sending input.")).toHaveCount(0);
  await expect(page.getByLabel("Terminal keys")).toHaveCount(0);

  await page.getByRole("button", { name: "＋ New" }).click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem("remote-command-types"))).toContain("create_session");
  await page.getByRole("button", { name: "Codex", exact: true }).click();
  const composer = page.getByRole("textbox", { name: "Command or agent prompt" });
  await expect(composer).toHaveValue("codex");
  await expect(page.getByText("The workspace changed; refresh before sending input.")).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => {
    const payloads = JSON.parse(localStorage.getItem("remote-command-payloads") ?? "[]") as Array<{
      type: string;
      data?: { data_base64?: string };
    }>;
    return {
      commandTypes: payloads.map((payload) => payload.type),
      rawInput: payloads
        .filter((payload) => payload.type === "raw_terminal_input")
        .map((payload) => atob(payload.data?.data_base64 ?? "")),
    };
  })).toEqual(expect.objectContaining({ rawInput: expect.arrayContaining(["codex"]) }));

  await composer.fill("echo from phone");
  await composer.press("Enter");
  await expect(composer).toHaveValue("");
  await composer.fill("pwd");
  await composer.press("Meta+Enter");
  await expect(composer).toHaveValue("");
  await expect.poll(() => page.evaluate(() => {
    const payloads = JSON.parse(localStorage.getItem("remote-command-payloads") ?? "[]") as Array<{
      type: string;
      data?: { data_base64?: string };
    }>;
    return payloads
      .filter((payload) => payload.type === "raw_terminal_input")
      .map((payload) => atob(payload.data?.data_base64 ?? ""))
      .filter((data) => data === "\r")
      .length;
  })).toBe(2);

  const projectAdd = page.getByRole("button", { name: "New project", exact: true });
  expect(await projectAdd.evaluate((button) => {
    const bounds = button.getBoundingClientRect();
    return bounds.left >= 0 && bounds.right <= innerWidth;
  })).toBe(true);
  await projectAdd.click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem("remote-command-types"))).toContain("create_project");

  await page.getByRole("button", { name: "Open project and tab drawer" }).click();
  const drawer = page.getByRole("complementary", { name: "Current project sessions" });
  await expect(drawer.getByText("Ship remote control", { exact: true })).toBeVisible();
  await expect(drawer.getByText("Empty project", { exact: true })).toHaveCount(0);

  await drawer.getByRole("button", { name: "Close drawer" }).click();
  await page.getByRole("button", { name: "Usage and settings" }).click();
  const usage = page.getByRole("dialog", { name: "Usage & connection" });
  await expect(usage.getByText("Today", { exact: true })).toBeVisible();
  await expect(usage.getByText("Test iPhone · Test Mac", { exact: false })).toBeVisible();
  await usage.getByRole("button", { name: "Close" }).click();

  await page.getByRole("button", { name: "Open project and tab drawer" }).click();
  await page.getByRole("button", { name: "＋ New session" }).click();
  const newSession = page.getByRole("dialog", { name: "New session" });
  await newSession.getByRole("button", { name: "Resume recent" }).click();
  await expect(newSession.getByText("Fix pairing flow", { exact: true })).toBeVisible();

  const dimensions = await page.evaluate(() => ({
    client: document.documentElement.clientWidth,
    scroll: document.documentElement.scrollWidth,
  }));
  expect(dimensions.scroll).toBeLessThanOrEqual(dimensions.client);
});

test("empty-project focus mode stays centered within a phone viewport", async ({ page }) => {
  await installPairedPhone(page);
  await page.evaluate(() => localStorage.setItem("remote-command-types", "[]"));
  await page.getByRole("button", { name: "Empty project" }).click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem("remote-command-types"))).toContain("create_session");
  const heading = page.getByRole("heading", { name: "Choose a live session" });
  await expect(heading).toBeVisible();

  const layout = await heading.evaluate((element) => {
    const headingBounds = element.getBoundingClientRect();
    const shellBounds = document.querySelector(".app-shell")!.getBoundingClientRect();
    return {
      viewportWidth: innerWidth,
      headingCenter: headingBounds.left + headingBounds.width / 2,
      shellLeft: shellBounds.left,
      shellRight: shellBounds.right,
    };
  });
  expect(Math.abs(layout.headingCenter - layout.viewportWidth / 2)).toBeLessThan(2);
  expect(layout.shellLeft).toBeGreaterThanOrEqual(0);
  expect(layout.shellRight).toBeLessThanOrEqual(layout.viewportWidth);
});
