// Native JSON helper for Clinch's agent-resume shell integration.
//
// macOS ships JavaScript for Automation (JXA) as part of /usr/bin/osascript,
// which lets the capture hooks parse and update JSON without requiring jq,
// Homebrew, Python, or any other user-installed runtime.

ObjC.import("Foundation");

function readStdin() {
    const data = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
    const string = $.NSString.alloc.initWithDataEncoding(data, $.NSUTF8StringEncoding);
    return string ? (ObjC.unwrap(string) || "") : "";
}

function readFile(path) {
    const string = $.NSString.stringWithContentsOfFileEncodingError(
        path,
        $.NSUTF8StringEncoding,
        null,
    );
    return string ? (ObjC.unwrap(string) || "") : "";
}

function listDirectory(path) {
    const values = $.NSFileManager.defaultManager.contentsOfDirectoryAtPathError(path, null);
    return values ? (ObjC.deepUnwrap(values) || []) : [];
}

function asString(value, fallback) {
    return typeof value === "string" ? value : (fallback || "");
}

function base64(value) {
    const data = $(asString(value, "")).dataUsingEncoding($.NSUTF8StringEncoding);
    return ObjC.unwrap(data.base64EncodedStringWithOptions(0));
}

function parseObject(text, label) {
    const value = JSON.parse(text);
    if (value === null || Array.isArray(value) || typeof value !== "object") {
        throw new Error(label + " must contain a JSON object");
    }
    return value;
}

function hookFields() {
    const payload = parseObject(readStdin(), "hook payload");
    return [
        base64(asString(payload.session_id, "")),
        base64(asString(payload.cwd, "")),
        base64(asString(payload.hook_event_name, "SessionStart")),
        base64(asString(payload.permission_mode, "")),
        base64(asString(payload.model, "")),
        base64(asString(payload.reason, "")),
    ].join("|");
}

// Forking through older embedded Codex needs explicit standard settings. A builtin label
// alone is insufficient: custom network rules, roots, or filesystem entries must not be lost.
function canonicalCodexProfile(settings, id) {
    const active = settings.active_permission_profile;
    const profile = settings.permission_profile;
    if (!active || active.id !== id || profile.network !== "restricted") return false;
    const fs = profile.file_system;
    if (!fs || fs.type !== "restricted" || !Array.isArray(fs.entries)) return false;
    const cwd = settings.cwd;
    const roots = settings.runtime_workspace_roots;
    if (roots && (roots.length !== 1 || roots[0] !== cwd)) return false;
    const required = new Set(["special:root:read"]);
    const allowed = new Set(required);
    if (id === ":workspace") {
        if (typeof cwd !== "string" || !cwd.startsWith("/")) return false;
        ["special:slash_tmp:write", "special:tmpdir:write", "path:" + cwd + ":write"].forEach(key => {
            required.add(key); allowed.add(key);
        });
        [".git", ".agents", ".codex", ".aws"].forEach(name => allowed.add("path:" + cwd + "/" + name + ":read"));
    }
    for (const entry of fs.entries) {
        const path = entry.path || {};
        const key = path.type === "special" ? "special:" + ((path.value || {}).kind) + ":" + entry.access
            : "path:" + path.path + ":" + entry.access;
        if (!allowed.has(key)) return false;
        required.delete(key);
    }
    return required.size === 0;
}

// Fold the same durable settings records used by Codex resume. Hook permission_mode is
// deliberately not used: "bypassPermissions" can mean never-ask inside a read-only sandbox.
function codexResumeFlags(path, sessionId, mode) {
    let identity = "";
    let settings = null;
    let reviewer = "user";
    const lines = readFile(path).split("\n");
    for (const line of lines) {
        let record;
        try { record = JSON.parse(line); } catch (_) { continue; }
        const payload = record.payload || {};
        if (record.type === "session_meta") identity = asString(payload.id, "");
        if (record.type === "turn_context") {
            settings = payload;
            if (payload.approvals_reviewer) reviewer = payload.approvals_reviewer;
        }
        if (record.type === "event_msg" && payload.type === "thread_settings_applied"
            && payload.thread_id === sessionId) {
            settings = payload.thread_settings;
            if (settings && settings.approvals_reviewer) reviewer = settings.approvals_reviewer;
        }
    }
    if (!settings || !identity) return ""; // Older transcripts retain their recorded flags.
    if (identity !== sessionId) throw new Error("saved settings belong to another conversation");
    // Modern Codex restores the complete persisted profile and workspace roots when direct
    // launch overrides are omitted. Keep this authoritative path instead of flattening it.
    if (mode === "native-resume") return "CLINCH_NATIVE_SAVED_SETTINGS";
    const token = value => typeof value === "string" && /^[A-Za-z0-9._:/-]+$/.test(value);
    const profile = settings.permission_profile;
    const active = settings.active_permission_profile;
    let sandbox = settings.sandbox_policy && settings.sandbox_policy.type;
    if (profile) {
        if (profile.type === "disabled") sandbox = "danger-full-access";
        else if (profile.type === "managed" && canonicalCodexProfile(settings, ":workspace")) sandbox = "workspace-write";
        else if (profile.type === "managed" && canonicalCodexProfile(settings, ":read-only")) sandbox = "read-only";
        else throw new Error("custom saved permission profile cannot be restored as a built-in mode");
    }
    const policy = settings.approval_policy;
    if (!["read-only", "workspace-write", "danger-full-access"].includes(sandbox)
        || !["on-request", "never"].includes(policy)
        || !["user", "auto_review"].includes(reviewer)) {
        throw new Error("saved permission settings are not supported by this launcher");
    }
    const args = [];
    if (sandbox === "workspace-write" && policy === "on-request" && reviewer === "auto_review") {
        args.push("--approve-for-me");
    } else {
        args.push("--sandbox", sandbox, "--ask-for-approval", policy,
            "-c", "approvals_reviewer=" + reviewer);
    }
    if (token(settings.model)) args.push("--model", settings.model);
    const effort = settings.reasoning_effort || settings.effort;
    if (token(effort)) args.push("-c", "model_reasoning_effort=" + effort);
    return args.join("\n");
}

function codexHookFlags() {
    const payload = parseObject(readStdin(), "Codex hook payload");
    const path = asString(payload.transcript_path, "");
    const sessionId = asString(payload.session_id, "");
    if (path && sessionId) {
        const flags = codexResumeFlags(path, sessionId);
        if (flags) return flags;
    }
    // Older providers may omit transcript_path. Never turn a lossy permission_mode into
    // full access. Native resume remains authoritative when complete settings are absent.
    const model = asString(payload.model, "");
    return /^[A-Za-z0-9._:/-]+$/.test(model) ? "--model\n" + model : "";
}

function promptLine(argv) {
    const payload = parseObject(readStdin(), "hook payload");
    // Devin passes the event on its hook's command line rather than relying on the payload.
    const event = asString(argv[3], "") || asString(payload.hook_event_name, "");
    if (event === "Stop") {
        const record = {
            ts: asString(argv[0], ""),
            stop: true,
        };
        // Devin's own session store isn't documented, so this log is also where `agent read` finds
        // its replies. Claude and Codex keep full transcripts, and their logs stay small.
        if (asString(argv[4], "") === "devin") {
            record.response = asString(payload.last_assistant_message, "");
        }
        return JSON.stringify(record);
    }
    if (event !== "UserPromptSubmit") {
        return "";
    }
    const prompt = asString(payload.prompt, "");
    if (!prompt.trim()) {
        return "";
    }
    return JSON.stringify({
        ts: asString(argv[0], ""),
        cwd: asString(argv[1], ""),
        bridge: asString(argv[2], ""),
        prompt: prompt,
    });
}

function toolbeltPromptFingerprint(text) {
    // Two independent 32-bit FNV-style lanes plus the normalized UTF-16 length make accidental
    // collisions impractical without pulling a non-system hashing runtime into the hook.
    let left = 2166136261;
    let right = 2246822519;
    for (let index = 0; index < text.length; index += 1) {
        const code = text.charCodeAt(index);
        left = Math.imul(left ^ code, 16777619);
        right = Math.imul(right ^ code, 3266489917);
    }
    function hex(value) {
        const output = (value >>> 0).toString(16);
        return "00000000".slice(output.length) + output;
    }
    return hex(left) + hex(right) + ":" + text.length;
}

function normalizeToolbeltPrompt(text) {
    const trimmed = asString(text, "").trim().replace(/\s+/g, " ");
    return trimmed.toLocaleLowerCase();
}

function isLearnableToolbeltPrompt(prompt, normalized) {
    if (prompt.length > 4096 || normalized.length < 12 || normalized.split(" ").length < 3) {
        return false;
    }
    // Credentials, private material, and common token formats.
    if (/(?:password|passwd|secret|api[_ -]?key|access[_ -]?token|authorization)\s*[:=]/i.test(prompt) ||
            /-----BEGIN [A-Z ]*PRIVATE KEY-----/.test(prompt) ||
            /(?:gh[pousr]_|sk-(?:proj-)?|xox[baprs]-|AKIA)[A-Za-z0-9_\-]{12,}/.test(prompt) ||
            /\b(?:bearer\s+)?[A-Za-z0-9_\/+\-=]{32,}\b/i.test(prompt) ||
            /\beyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\b/.test(prompt) ||
            /:\/\/[^/\s:@]+:[^/\s@]+@/.test(prompt)) {
        return false;
    }
    // Do not turn destructive shell/database/infrastructure operations into a convenient button.
    if (/\b(?:sudo\s+)?rm\s+(?:-[A-Za-z]*r[A-Za-z]*f|-[A-Za-z]*f[A-Za-z]*r)\b/i.test(normalized) ||
            /\bgit\s+(?:reset\s+--hard|clean\s+-[A-Za-z]*f|push\b[^\n]*--force)|\b(?:terraform|tofu)\s+destroy\b|\bkubectl\s+delete\b|\b(?:drop\s+(?:database|table)|truncate\s+table|delete\s+from)\b|\b(?:mkfs|shutdown|reboot)\b/i.test(normalized)) {
        return false;
    }
    // Obvious one-off identifiers and timestamp-shaped values are poor reusable controls.
    if (/\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b/i.test(prompt) ||
            /\b[0-9a-f]{24,}\b/i.test(prompt) ||
            /\b20\d\d-[01]\d-[0-3]\d[t ][0-2]\d:[0-5]\d(?::[0-5]\d)?\b/i.test(prompt) ||
            /\b[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,}\b/i.test(prompt)) {
        return false;
    }
    return true;
}

function newToolbeltSuggestionId() {
    return asString(ObjC.unwrap($.NSUUID.UUID.UUIDString), "").toLocaleLowerCase();
}

function toolbeltLearn(argv) {
    const storePath = asString(argv[0], "");
    const provider = asString(argv[1], "");
    const sessionId = asString(argv[2], "");
    const timestamp = asString(argv[3], "");
    if (!storePath || !/^(?:claude|codex|devin)$/.test(provider) ||
            !/^[A-Za-z0-9-]+$/.test(sessionId)) {
        throw new Error("toolbelt-learn requires a store, provider, and safe session id");
    }

    const payload = parseObject(readStdin(), "hook payload");
    if (asString(payload.hook_event_name, "") !== "UserPromptSubmit") {
        return "";
    }
    const prompt = asString(payload.prompt, "");
    const normalized = normalizeToolbeltPrompt(prompt);
    if (!isLearnableToolbeltPrompt(prompt, normalized)) {
        return "";
    }

    let store = { schema: 1, patterns: [] };
    const previous = readFile(storePath);
    if (previous) {
        try {
            const parsed = parseObject(previous, "toolbelt learning store");
            if (parsed.schema === 1 && Array.isArray(parsed.patterns)) {
                store = parsed;
            }
        } catch (_) {
            // A malformed optional learning store must not break prompt capture. Start fresh.
        }
    }

    const fingerprint = toolbeltPromptFingerprint(normalized);
    const sessionKey = provider + ":" + sessionId;
    let pattern = store.patterns.find(function (item) {
        return item && item.fingerprint === fingerprint;
    });
    if (!pattern) {
        pattern = {
            id: newToolbeltSuggestionId(),
            fingerprint: fingerprint,
            sessions: [],
            conversation_count: 0,
            providers: [],
            first_seen: timestamp,
            last_seen: timestamp,
        };
        store.patterns.push(pattern);
    }
    if (!Array.isArray(pattern.sessions)) {
        pattern.sessions = [];
    }
    if (!Array.isArray(pattern.providers)) {
        pattern.providers = [];
    }
    if (pattern.sessions.indexOf(sessionKey) < 0) {
        pattern.sessions.push(sessionKey);
        pattern.conversation_count = Math.max(
            Number.isFinite(pattern.conversation_count) ? pattern.conversation_count : 0,
            pattern.sessions.length,
        );
        if (pattern.sessions.length > 32) {
            pattern.sessions = pattern.sessions.slice(-32);
        }
    }
    if (pattern.providers.indexOf(provider) < 0) {
        pattern.providers.push(provider);
    }
    pattern.last_seen = timestamp;
    if (pattern.conversation_count >= 2) {
        pattern.text = prompt;
    }

    store.patterns.forEach(function (item) {
        if (item && Array.isArray(item.sessions) && item.sessions.length > 32) {
            item.sessions = item.sessions.slice(-32);
        }
    });
    store.patterns.sort(function (a, b) {
        return asString(a.last_seen, "") < asString(b.last_seen, "") ? 1 : -1;
    });
    store.patterns = store.patterns.slice(0, 512);
    return JSON.stringify(store, null, 2) + "\n";
}

function registryEntry(argv) {
    const command = asString(argv[0], "");
    const cwd = asString(argv[1], "");
    const bridge = asString(argv[2], "");
    const ownerPid = asString(argv[3], "");
    const ownerTty = asString(argv[4], "");
    // Retain the human-readable spacing used by existing registry files while
    // delegating complete string escaping to JSON.stringify.
    const fields = [
        "\"command\": " + JSON.stringify(command),
        "\"cwd\": " + JSON.stringify(cwd),
    ];
    if (bridge) {
        fields.push("\"bridge\": " + JSON.stringify(bridge));
    }
    if (ownerPid) {
        fields.push("\"owner_pid\": " + JSON.stringify(ownerPid));
    }
    if (ownerTty) {
        fields.push("\"owner_tty\": " + JSON.stringify(ownerTty));
    }
    return "{ " + fields.join(", ") + " }";
}

function registryFields(argv) {
    const path = argv[0];
    if (!path) {
        throw new Error("registry-fields requires a registry entry path");
    }
    const entry = parseObject(readFile(path), "registry entry");
    return [
        base64(asString(entry.command, "")),
        base64(asString(entry.cwd, "")),
        base64(asString(entry.bridge, "")),
        base64(asString(entry.owner_pid, "")),
        base64(asString(entry.owner_tty, "")),
    ].join("|");
}

function journalLine(operation, argv) {
    const row = {
        ts: asString(argv[0], ""),
        op: operation,
        pane: asString(argv[1], ""),
    };
    if (operation === "write") {
        row.command = asString(argv[2], "");
        row.cwd = asString(argv[3], "");
        row.bridge = asString(argv[4], "");
    }
    return JSON.stringify(row);
}

function scrubBridgeEntry(argv) {
    const path = argv[0];
    const bridge = asString(argv[1], "");
    if (!path || !bridge) {
        throw new Error("scrub-bridge-entry requires a file and bridge id");
    }
    const entry = parseObject(readFile(path), "registry entry");
    if (asString(entry.bridge, "") !== bridge) {
        return "";
    }
    delete entry.bridge;
    return JSON.stringify(entry);
}

function journalScrub(argv) {
    const ts = asString(argv[0], "");
    const pane = asString(argv[1], "");
    const path = argv[2];
    const bridge = asString(argv[3], "");
    if (!path || !bridge) {
        throw new Error("journal-scrub requires a file and bridge id");
    }
    const entry = parseObject(readFile(path), "registry entry");
    if (asString(entry.bridge, "") !== bridge) {
        return "";
    }
    return JSON.stringify({
        ts: ts,
        op: "scrub-bridge",
        pane: pane,
        command: asString(entry.command, ""),
        cwd: asString(entry.cwd, ""),
        bridge: bridge,
    });
}

// Drops every hook `isManaged` recognizes, leaving unrelated hooks and their ordering alone.
function removeManagedHooks(settings, isManaged) {
    Object.keys(settings.hooks).forEach(function (eventName) {
        const groups = settings.hooks[eventName];
        if (!Array.isArray(groups)) {
            return;
        }
        settings.hooks[eventName] = groups.filter(function (group) {
            if (group === null || Array.isArray(group) || typeof group !== "object") {
                return true;
            }
            if (!Array.isArray(group.hooks)) {
                return true;
            }
            group.hooks = group.hooks.filter(function (hook) {
                if (hook === null || Array.isArray(hook) || typeof hook !== "object") {
                    return true;
                }
                return !isManaged(hook);
            });
            return group.hooks.length > 0;
        });
    });
}

function hasHooksObject(settings) {
    return settings.hooks !== null && !Array.isArray(settings.hooks) && typeof settings.hooks === "object";
}

function removeEmptyHookEvents(settings) {
    Object.keys(settings.hooks).forEach(function (eventName) {
        if (Array.isArray(settings.hooks[eventName]) && settings.hooks[eventName].length === 0) {
            delete settings.hooks[eventName];
        }
    });
    if (Object.keys(settings.hooks).length === 0) {
        delete settings.hooks;
    }
}

function wireClaude(argv) {
    const oldCommand = argv[0];
    const captureCommand = argv[1];
    if (!oldCommand || !captureCommand) {
        throw new Error("wire-claude requires old and current hook paths");
    }

    const settings = parseObject(readStdin(), "Claude settings");
    if (!hasHooksObject(settings)) {
        settings.hooks = {};
    }

    // Remove every managed legacy/current entry first. That makes the update
    // idempotent, repairs duplicates, and moves a misplaced managed hook back
    // to its supported lifecycle events while preserving unrelated hooks.
    removeManagedHooks(settings, function (hook) {
        return hook.command === oldCommand || hook.command === captureCommand;
    });

    ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd"].forEach(function (eventName) {
        if (!Array.isArray(settings.hooks[eventName])) {
            settings.hooks[eventName] = [];
        }
        settings.hooks[eventName].push({
            hooks: [{ type: "command", command: captureCommand }],
        });
    });

    return JSON.stringify(settings, null, 2) + "\n";
}

function unwireClaude(argv) {
    const oldCommand = argv[0];
    const captureCommand = argv[1];
    if (!oldCommand || !captureCommand) {
        throw new Error("unwire-claude requires old and current hook paths");
    }

    const settings = parseObject(readStdin(), "Claude settings");
    if (!hasHooksObject(settings)) {
        return JSON.stringify(settings, null, 2) + "\n";
    }
    removeManagedHooks(settings, function (hook) {
        return hook.command === oldCommand || hook.command === captureCommand;
    });
    removeEmptyHookEvents(settings);
    return JSON.stringify(settings, null, 2) + "\n";
}

const DEVIN_HOOK_NAME = "/devin-hook.sh";
const DEVIN_HOOK_EVENTS = [
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "PostToolUse",
    "PostCompaction",
    "Stop",
    "SessionEnd",
];

function isDevinHook(hook) {
    return typeof hook.command === "string" && hook.command.indexOf(DEVIN_HOOK_NAME) !== -1;
}

function shellQuote(text) {
    return "'" + text.replace(/'/g, "'\\''") + "'";
}

// Each event names itself on the command line, so a payload without an event name can't be mistaken
// for a session start. Devin treats a slow hook as an error rather than waiting forever on it, and
// the timeout keeps a wedged helper from holding up a turn.
function wireDevin(argv) {
    const hookPath = argv[0];
    if (!hookPath) {
        throw new Error("wire-devin requires the hook path");
    }
    const settings = parseObject(readStdin(), "Devin config");
    if (!hasHooksObject(settings)) {
        settings.hooks = {};
    }
    removeManagedHooks(settings, isDevinHook);
    DEVIN_HOOK_EVENTS.forEach(function (eventName) {
        if (!Array.isArray(settings.hooks[eventName])) {
            settings.hooks[eventName] = [];
        }
        settings.hooks[eventName].push({
            hooks: [{
                type: "command",
                command: shellQuote(hookPath) + " " + eventName,
                timeout: 10,
            }],
        });
    });
    return JSON.stringify(settings, null, 2) + "\n";
}

function unwireDevin() {
    const settings = parseObject(readStdin(), "Devin config");
    if (hasHooksObject(settings)) {
        removeManagedHooks(settings, isDevinHook);
        removeEmptyHookEvents(settings);
    }
    return JSON.stringify(settings, null, 2) + "\n";
}

function clipped(text, max) {
    return text.length > max ? text.slice(0, max - 3) + "..." : text;
}

// The status event for one Devin hook, as the pane's terminal receives it. Text fields are bounded
// so the event stays one short write. JSON.stringify leaves DEL and the C1 controls raw, and a C1
// string terminator in a prompt could end the escape sequence carrying this event.
function devinEvent(argv) {
    const hookEvent = asString(argv[0], "");
    const name = {
        SessionStart: "session_start",
        UserPromptSubmit: "prompt_submit",
        PermissionRequest: "permission_request",
        // `/compact` fires no prompt hooks, only this. Claude Code reports a compaction as a
        // session start too, which tells Clinch what was typed before it has been handled.
        PostCompaction: "session_start",
        Stop: "stop",
    }[hookEvent];
    if (!name) {
        return "";
    }
    const payload = parseObject(readStdin(), "hook payload");
    const cwd = asString(argv[2], "");
    const event = {
        v: 1,
        agent: "devin",
        event: name,
        session_id: asString(argv[1], ""),
        cwd: cwd,
        project: cwd.split("/").filter(Boolean).pop() || "",
    };
    if (name === "prompt_submit") {
        event.query = clipped(asString(payload.prompt, ""), 200);
    } else if (name === "stop") {
        event.response = clipped(asString(payload.last_assistant_message, ""), 200);
    } else if (name === "permission_request") {
        const input = payload.tool_input;
        const fields = input !== null && typeof input === "object" ? input : {};
        const preview = asString(fields.command, "") || asString(fields.file_path, "")
            || clipped(JSON.stringify(fields), 80);
        event.tool_name = clipped(asString(payload.tool_name, "unknown"), 80);
        event.summary = clipped("Wants to run " + event.tool_name + ": " + preview, 160);
    }
    return JSON.stringify(event).replace(/[\u007f-\u009f]/g, function (c) {
        return "\\u" + ("0000" + c.charCodeAt(0).toString(16)).slice(-4);
    });
}

function devinSessionContext(argv) {
    return JSON.stringify({
        hookSpecificOutput: {
            hookEventName: "SessionStart",
            additionalContext: asString(argv[0], ""),
        },
    });
}

// Return one pane's journaled writes newest-first. Fields are base64 encoded and
// pipe-delimited so Bash 3.2 can consume arbitrary cwd/flag text without jq. A later
// scrub-bridge record clears that bridge from older candidates, while a still-later
// write may legitimately add it again.
function paneHistory(argv) {
    const directory = argv[0];
    const pane = asString(argv[1], "");
    const agent = asString(argv[2], "");
    if (!directory || !pane || !agent) {
        throw new Error("pane-history requires directory, pane, and agent");
    }

    const writes = [];
    const clearedAt = {};
    let sequence = 0;
    parseJsonLines(readFile(directory + "/journal.jsonl"), function (row) {
        sequence += 1;
        if (!row || row.pane !== pane || typeof row.command !== "string") {
            return;
        }
        const match = row.command.match(
            /(?:clinch|warp)_agent_resume_launch\s+(claude|codex|devin)\s+([A-Za-z0-9-]+)/,
        );
        if (!match || match[1] !== agent) {
            return;
        }
        const bridge = asString(row.bridge, "");
        const key = match[2] + "\u0000" + bridge;
        if (row.op === "scrub-bridge" && bridge) {
            clearedAt[key] = sequence;
            return;
        }
        if (row.op === "write") {
            writes.push({
                sequence: sequence,
                command: row.command,
                cwd: asString(row.cwd, ""),
                bridge: bridge,
                key: key,
            });
        }
    });

    writes.sort(function (a, b) {
        return b.sequence - a.sequence;
    });
    return writes.map(function (row) {
        const cleared = row.bridge && clearedAt[row.key] > row.sequence;
        return [
            base64(row.command),
            base64(row.cwd),
            base64(cleared ? "" : row.bridge),
        ].join("|");
    }).join("\n");
}

function parseJsonLines(text, callback) {
    text.split(/\r?\n/).forEach(function (line) {
        if (!line.trim()) {
            return;
        }
        try {
            callback(JSON.parse(line));
        } catch (_) {
            // Journals are append-only and fail-soft. One truncated/corrupt line
            // must not hide every healthy conversation around it.
        }
    });
}

function listConversations(argv) {
    const directory = argv[0];
    const cwdFilter = asString(argv[1], "");
    const outputFormat = asString(argv[2], "human");
    if (!directory) {
        throw new Error("list requires a registry directory");
    }

    const sessions = {};
    function sessionFor(id, agent) {
        agent = asString(agent, "claude");
        const key = agent + "\u0000" + id;
        if (!sessions[key]) {
            sessions[key] = {
                id: id,
                start: "",
                cwd: "",
                cwdTs: "",
                bridge: "",
                bridgeTs: "",
                prompt: "",
                agent: agent,
                mirrorSourcePriority: 0,
            };
        }
        return sessions[key];
    }
    function observe(id, ts, cwd, bridge, agent) {
        if (!id) {
            return null;
        }
        agent = asString(agent, "");
        const session = sessionFor(id, agent);
        ts = asString(ts, "");
        if (ts && (!session.start || ts < session.start)) {
            session.start = ts;
        }
        cwd = asString(cwd, "");
        if (cwd && (!session.cwdTs || ts >= session.cwdTs)) {
            session.cwd = cwd;
            session.cwdTs = ts;
        }
        bridge = asString(bridge, "");
        if (bridge && (!session.bridgeTs || ts >= session.bridgeTs)) {
            session.bridge = bridge;
            session.bridgeTs = ts;
        }
        return session;
    }
    function clearBridge(id, ts, bridge, agent) {
        bridge = asString(bridge, "");
        if (!id || !bridge) {
            return;
        }
        const session = sessionFor(id, agent);
        ts = asString(ts, "");
        if (session.bridge === bridge && (!session.bridgeTs || !ts || ts >= session.bridgeTs)) {
            session.bridge = "";
            session.bridgeTs = ts;
        }
    }

    const journalPath = directory + "/journal.jsonl";
    parseJsonLines(readFile(journalPath), function (row) {
        if (!row || (row.op !== "write" && row.op !== "scrub-bridge") ||
                typeof row.command !== "string") {
            return;
        }
        const match = row.command.match(/(?:clinch|warp)_agent_resume_launch\s+(claude|codex|devin)\s+([A-Za-z0-9-]+)/);
        if (match) {
            observe(match[2], row.ts, row.cwd, row.bridge, match[1]);
            if (row.op === "scrub-bridge") {
                clearBridge(match[2], row.ts, row.bridge, match[1]);
            }
        }
    });

    const promptsDirectory = directory + "/prompts";
    function readPromptDirectory(path, agent, priority) {
        listDirectory(path).forEach(function (name) {
            if (typeof name !== "string" || !name.endsWith(".jsonl")) {
                return;
            }
            const id = name.slice(0, -6);
            const known = sessionFor(id, agent);
            if (known.mirrorSourcePriority > priority) {
                return;
            }
            if (known.mirrorSourcePriority < priority) {
                known.prompt = "";
                known.mirrorSourcePriority = priority;
            }
            parseJsonLines(readFile(path + "/" + name), function (row) {
                if (!row || typeof row.prompt !== "string") {
                    return;
                }
                const session = observe(id, row.ts, row.cwd, row.bridge, agent);
                if (session && !session.prompt && row.prompt) {
                    session.prompt = row.prompt;
                }
            });
        });
    }
    // Provider-scoped mirrors are canonical. Flat files are read only as the legacy Claude
    // layout and cannot overwrite a scoped source for the same session id.
    readPromptDirectory(promptsDirectory + "/claude", "claude", 2);
    readPromptDirectory(promptsDirectory + "/codex", "codex", 2);
    readPromptDirectory(promptsDirectory + "/devin", "devin", 2);
    readPromptDirectory(promptsDirectory, "claude", 1);

    const conversations = Object.keys(sessions)
        .map(function (key) { return sessions[key]; })
        .filter(function (session) {
            return session.start && (!cwdFilter || session.cwd === cwdFilter);
        })
        .sort(function (a, b) {
            return a.start < b.start ? 1 : (a.start > b.start ? -1 : 0);
        });

    if (outputFormat === "json") {
        return JSON.stringify(conversations.map(function (session) {
            const url = session.bridge
                ? "https://claude.ai/code/" + session.bridge
                : null;
            return {
                ts: session.start,
                session_id: session.id,
                cwd: session.cwd,
                bridge: session.bridge || null,
                url: url,
                first_prompt: session.prompt || null,
            };
        }), null, 2);
    }
    if (outputFormat !== "human") {
        throw new Error("unknown list output format: " + outputFormat);
    }

    return conversations
        .map(function (session) {
            const location = session.bridge
                ? "https://claude.ai/code/" + session.bridge
                : "local";
            const prompt = session.prompt
                .replace(/[\r\n\t]+/g, " ")
                .replace(/\s+$/g, "")
                .slice(0, 80);
            return session.start + "  " + session.id.slice(0, 8) + "  " +
                session.cwd + "  " + location + "  \"" + prompt + "\"";
        })
        .join("\n");
}

function run(argv) {
    const command = argv.shift();
    switch (command) {
    case "hook-fields":
        return hookFields();
    case "codex-resume-flags":
        return codexResumeFlags(argv[0], argv[1], argv[2]);
    case "codex-hook-flags":
        return codexHookFlags();
    case "prompt-line":
        return promptLine(argv);
    case "toolbelt-learn":
        return toolbeltLearn(argv);
    case "wire-claude":
        return wireClaude(argv);
    case "unwire-claude":
        return unwireClaude(argv);
    case "wire-devin":
        return wireDevin(argv);
    case "unwire-devin":
        return unwireDevin();
    case "devin-event":
        return devinEvent(argv);
    case "devin-session-context":
        return devinSessionContext(argv);
    case "registry-entry":
        return registryEntry(argv);
    case "registry-fields":
        return registryFields(argv);
    case "journal-write":
        return journalLine("write", argv);
    case "journal-remove":
        return journalLine("remove", argv);
    case "scrub-bridge-entry":
        return scrubBridgeEntry(argv);
    case "journal-scrub":
        return journalScrub(argv);
    case "list":
        return listConversations(argv);
    case "pane-history":
        return paneHistory(argv);
    default:
        throw new Error("unknown agent-json command: " + (command || "<empty>"));
    }
}
