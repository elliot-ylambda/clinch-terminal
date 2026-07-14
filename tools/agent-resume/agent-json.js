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
    ].join("|");
}

function promptLine(argv) {
    const payload = parseObject(readStdin(), "hook payload");
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

function wireClaude(argv) {
    const oldCommand = argv[0];
    const captureCommand = argv[1];
    if (!oldCommand || !captureCommand) {
        throw new Error("wire-claude requires old and current hook paths");
    }

    const settings = parseObject(readStdin(), "Claude settings");
    if (settings.hooks === null || Array.isArray(settings.hooks) || typeof settings.hooks !== "object") {
        settings.hooks = {};
    }

    // Remove every managed legacy/current entry first. That makes the update
    // idempotent, repairs duplicates, and moves a misplaced managed hook back
    // to the three supported lifecycle events while preserving unrelated hooks.
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
                return hook.command !== oldCommand && hook.command !== captureCommand;
            });
            return group.hooks.length > 0;
        });
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
    if (settings.hooks === null || Array.isArray(settings.hooks) || typeof settings.hooks !== "object") {
        return JSON.stringify(settings, null, 2) + "\n";
    }

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
                return hook.command !== oldCommand && hook.command !== captureCommand;
            });
            return group.hooks.length > 0;
        });
        if (settings.hooks[eventName].length === 0) {
            delete settings.hooks[eventName];
        }
    });

    if (Object.keys(settings.hooks).length === 0) {
        delete settings.hooks;
    }
    return JSON.stringify(settings, null, 2) + "\n";
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
            /(?:clinch|warp)_agent_resume_launch\s+(claude|codex)\s+([A-Za-z0-9-]+)/,
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
        const match = row.command.match(/(?:clinch|warp)_agent_resume_launch\s+(claude|codex)\s+([A-Za-z0-9-]+)/);
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
    case "prompt-line":
        return promptLine(argv);
    case "wire-claude":
        return wireClaude(argv);
    case "unwire-claude":
        return unwireClaude(argv);
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
