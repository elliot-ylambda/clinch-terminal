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
    const reviewTarget = {
      ...target,
      tab_id: "tab-review",
      pane_id: "pane-review",
    };
    const otherTarget = {
      ...target,
      project_id: "project-other",
      tab_id: "tab-other",
      pane_id: "pane-other",
    };
    const wideTerminalSnapshot = [
      "\x1b[2J\x1b[H\x1b[?7h\x1b[2;1H  Hooks need review",
      "\x1b[3;1H  8 hooks are new or changed.",
      "\x1b[4;1H  Hooks can run outside the sandbox after you trust them.",
      "\x1b[6;1H> 1. Review hooks",
      "\x1b[7;1H  2. Trust all and continue",
      "\x1b[8;1H  3. Continue without trusting (hooks won't run)",
      `\x1b[10;1H  Press enter to confirm or esc to go back${" ".repeat(278)}`,
      "\x1b[10;320H\x1b[?25l",
    ].join("");
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
        badges: { has_other_unread: true, done: 2, working: 3, running_commands: 4 },
        tabs: [{
          id: target.tab_id,
          title: "Ship remote control",
          kind: "claude_code",
          active: true,
          activity: "done",
          unread: true,
          remote_host: null,
          panes: [{
            id: target.pane_id,
            title: "Ship remote control",
            kind: "claude_code",
            cwd: "/Users/test/demo",
            active: true,
            agent_state: "done",
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
        }, {
          id: reviewTarget.tab_id,
          title: "Review docs",
          kind: "codex",
          active: false,
          activity: "idle",
          unread: false,
          remote_host: null,
          panes: [{
            id: reviewTarget.pane_id,
            title: "Review docs",
            kind: "codex",
            cwd: "/Users/test/demo",
            active: true,
            agent_state: "idle",
            dimensions: { columns: 80, rows: 24 },
            writer_lease: null,
            quick_inserts: [],
          }],
        }],
      }, {
        id: "project-empty",
        title: "Empty project",
        order: 1,
        active: false,
        activity: "idle",
        badges: { has_other_unread: false, done: 0, working: 0, running_commands: 0 },
        tabs: [],
      }, {
        id: otherTarget.project_id,
        title: "Other project",
        order: 2,
        active: false,
        activity: "idle",
        badges: { has_other_unread: false, done: 0, working: 0, running_commands: 0 },
        tabs: [{
          id: otherTarget.tab_id,
          title: "Other terminal",
          kind: "terminal",
          active: true,
          activity: "idle",
          unread: false,
          remote_host: null,
          panes: [{
            id: otherTarget.pane_id,
            title: "Other terminal",
            kind: "terminal",
            cwd: "/Users/test/other",
            active: true,
            agent_state: null,
            dimensions: { columns: 80, rows: 24 },
            writer_lease: null,
            quick_inserts: [],
          }],
        }],
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
          const selectedTarget = envelope.payload.data?.target ?? target;
          snapshot.active_target = selectedTarget;
          for (const project of snapshot.projects) {
            project.active = project.id === selectedTarget.project_id;
            for (const tab of project.tabs) {
              tab.active = tab.id === selectedTarget.tab_id;
            }
          }
          this.emit({
            type: "terminal_snapshot",
            data: {
              target: selectedTarget,
              stream_id: "55555555-5555-4555-8555-555555555555",
              workspace_revision: snapshot.revision,
              terminal_sequence: 0,
              data_base64: btoa(wideTerminalSnapshot),
              dimensions: { columns: 320, rows: 93 },
            },
          }, envelope.request_id);
        } else if (envelope.payload.type === "ping") {
          this.emit({ type: "pong" }, envelope.request_id);
        } else if (envelope.payload.type === "request_snapshot") {
          this.emit({ type: "snapshot", data: snapshot }, envelope.request_id);
        } else if (envelope.payload.type === "acquire_writer_lease") {
          (snapshot.projects[0].tabs[0].panes[0] as unknown as {
            writer_lease: { device_id: string; device_name: string; expires_at: string } | null;
          }).writer_lease = {
            device_id: "11111111-1111-4111-8111-111111111111",
            device_name: "Test iPhone",
            expires_at: "2099-01-01T00:00:00Z",
          };
          this.emit({
            type: "writer_lease_changed",
            data: {
              target,
              lease: snapshot.projects[0].tabs[0].panes[0].writer_lease,
            },
          }, envelope.request_id);
          window.setTimeout(() => this.emit({ type: "workspace_changed", data: { snapshot } }), 0);
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

  const demoProject = page.getByRole("button", {
    name: "Demo, 2 done, 3 working, 4 commands running, unread activity",
  });
  await expect(demoProject).toBeVisible();
  await expect(demoProject.locator(".project-count.done")).toHaveText("2");
  await expect(demoProject.locator(".project-count.working")).toHaveText("3");
  await expect(demoProject.locator(".project-count.command")).toHaveText("4");
  expect(await demoProject.locator(".project-count").evaluateAll((badges) =>
    badges.map((badge) => getComputedStyle(badge).color),
  )).toEqual(["rgb(55, 128, 233)", "rgb(191, 255, 0)", "rgb(116, 121, 135)"]);
  await expect(page.getByLabel("Selected Clinch terminal output")).toBeVisible();
  await expect.poll(() => page.locator(".xterm-rows > div").evaluateAll((rows) => {
    const lines = rows.map((row) => row.textContent ?? "");
    return {
      title: lines.filter((line) => line.includes("Hooks need review")).length,
      instructions: lines.filter((line) => line.includes("Press enter to confirm or esc to go back")).length,
      duplicatedTail: lines.filter((line) => line.trim() === "to go back").length,
    };
  })).toEqual({ title: 1, instructions: 1, duplicatedTail: 0 });
  await expect(page.getByText("The workspace changed; refresh before sending input.")).toHaveCount(0);
  await expect(page.getByLabel("Terminal keys")).toHaveCount(0);
  await expect(page.locator(".session-header")).toHaveCount(0);

  const drawerToggle = page.getByRole("button", { name: "Open project and tab drawer" });
  await expect(drawerToggle.getByRole("img", { name: "Clinch" })).toBeVisible();
  await demoProject.click();
  await expect(page.getByRole("complementary", { name: "Current project sessions" })).toBeVisible();
  await page.getByRole("button", { name: "Close drawer" }).click();
  await drawerToggle.click();
  await page.getByRole("complementary", { name: "Current project sessions" })
    .getByRole("button", { name: "＋ New session" }).click();
  const createSession = page.getByRole("dialog", { name: "New session" });
  await createSession.getByRole("button", { name: "Create on Test Mac" }).click();
  await expect(createSession).toBeHidden();
  await expect.poll(() => page.evaluate(() => localStorage.getItem("remote-command-types"))).toContain("create_session");
  await page.evaluate(() => localStorage.setItem("remote-command-payloads", "[]"));
  const quickInsert = page.getByRole("button", { name: "Codex", exact: true });
  await quickInsert.evaluate((button: HTMLButtonElement) => {
    button.click();
    button.click();
  });
  await expect(page.getByRole("textbox", { name: "Command or agent prompt" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Send", exact: true })).toHaveCount(0);
  await expect(page.getByText("The workspace changed; refresh before sending input.")).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => {
    const payloads = JSON.parse(localStorage.getItem("remote-command-payloads") ?? "[]") as Array<{
      type: string;
      data?: { data_base64?: string; dimensions?: { columns: number; rows: number } };
    }>;
    return {
      quickInsertPreviews: payloads.filter((payload) => payload.type === "quick_insert_preview").length,
      pastedCodex: payloads
        .filter((payload) => payload.type === "raw_terminal_input")
        .map((payload) => atob(payload.data?.data_base64 ?? ""))
        .filter((input) => input === "codex").length,
    };
  })).toEqual({ quickInsertPreviews: 1, pastedCodex: 1 });

  const terminalInput = page.getByRole("textbox", { name: "Terminal input" });
  await expect(terminalInput).toBeFocused();
  await terminalInput.press("Enter");
  await terminalInput.pressSequentially("echo from phone");
  await terminalInput.press("Enter");
  await expect.poll(() => page.evaluate(() => {
    const payloads = JSON.parse(localStorage.getItem("remote-command-payloads") ?? "[]") as Array<{
      type: string;
      data?: { data_base64?: string; dimensions?: { columns: number; rows: number } };
    }>;
    return payloads
      .filter((payload) => payload.type === "raw_terminal_input")
      .map((payload) => atob(payload.data?.data_base64 ?? ""))
      .join("");
  })).toBe("codex\recho from phone\r");

  const keyboardTools = page.getByRole("contentinfo", { name: "Terminal keyboard tools" });
  await expect(keyboardTools.getByRole("button", { name: "Attach photo or file" })).toBeVisible();
  await expect(keyboardTools.getByRole("button", { name: "Codex", exact: true })).toBeVisible();
  await keyboardTools.getByRole("button", { name: "Close keyboard" }).click();
  await expect(terminalInput).not.toBeFocused();
  await expect.poll(() => page.evaluate(() => {
    const payloads = JSON.parse(localStorage.getItem("remote-command-payloads") ?? "[]") as Array<{
      type: string;
      data?: { data_base64?: string; dimensions?: { columns: number; rows: number } };
    }>;
    return {
      rawReturns: payloads
        .filter((payload) => payload.type === "raw_terminal_input")
        .map((payload) => atob(payload.data?.data_base64 ?? ""))
        .filter((data) => data === "\r").length,
      leaseRequests: payloads.filter((payload) => payload.type === "acquire_writer_lease").length,
      composerSubmits: payloads.filter((payload) => payload.type === "submit_composer_text").length,
      unsafeResizes: payloads.filter((payload) =>
        payload.type === "terminal_resize"
        && ((payload.data?.dimensions?.columns ?? 0) < 20 || (payload.data?.dimensions?.rows ?? 0) < 4),
      ).length,
    };
  })).toEqual({
    rawReturns: 2,
    leaseRequests: 1,
    composerSubmits: 0,
    unsafeResizes: 0,
  });

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
  const doneIndicator = drawer.getByLabel("Done");
  await expect(doneIndicator).toBeVisible();
  expect(await doneIndicator.evaluate((indicator) => getComputedStyle(indicator).backgroundColor))
    .toBe("rgb(55, 128, 233)");

  await drawer.getByRole("button", { name: "Close drawer" }).click();
  await page.getByRole("button", { name: "Usage and settings" }).click();
  const usage = page.getByRole("dialog", { name: "Usage & connection" });
  await expect(usage.getByText("Today", { exact: true })).toBeVisible();
  await expect(usage.getByText("Test iPhone · Test Mac", { exact: false })).toBeVisible();
  await expect(usage.getByText("full-screen app without browser bars", { exact: false })).toBeVisible();
  await usage.getByRole("button", { name: "Close" }).click();

  await page.getByRole("button", { name: "Open project and tab drawer" }).click();
  await page.getByRole("complementary", { name: "Current project sessions" })
    .getByRole("button", { name: /Review docs/ })
    .click();
  await page.getByRole("button", { name: "Other project", exact: true }).click();
  await demoProject.click();
  await expect.poll(() => page.evaluate(() => {
    const payloads = JSON.parse(localStorage.getItem("remote-command-payloads") ?? "[]") as Array<{
      type: string;
      data?: { target?: { tab_id?: string } };
    }>;
    return payloads.filter((payload) => payload.type === "select_target").at(-1)?.data?.target?.tab_id;
  })).toBe("tab-review");
  await expect.poll(() => page.evaluate(() => {
    const targets = JSON.parse(localStorage.getItem("clinch-remote-control:last-target-by-project") ?? "[]") as Array<{
      project_id?: string;
      tab_id?: string;
    }>;
    return targets.find((target) => target.project_id === "project-demo")?.tab_id;
  })).toBe("tab-review");

  // The per-project selection survives a mobile refresh. The Mac's active target still wins on
  // initial synchronization, but switching away and back restores this project's remembered tab.
  await page.reload();
  await expect(page.getByLabel("Selected Clinch terminal output")).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const targets = JSON.parse(localStorage.getItem("clinch-remote-control:last-target-by-project") ?? "[]") as Array<{
      project_id?: string;
      tab_id?: string;
    }>;
    return targets.find((target) => target.project_id === "project-demo")?.tab_id;
  })).toBe("tab-review");
  await page.evaluate(() => localStorage.setItem("remote-command-payloads", "[]"));
  await page.getByRole("button", { name: "Other project", exact: true }).click();
  await demoProject.click();
  await expect.poll(() => page.evaluate(() => {
    const payloads = JSON.parse(localStorage.getItem("remote-command-payloads") ?? "[]") as Array<{
      type: string;
      data?: { target?: { tab_id?: string } };
    }>;
    return payloads.filter((payload) => payload.type === "select_target").at(-1)?.data?.target?.tab_id;
  })).toBe("tab-review");

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
