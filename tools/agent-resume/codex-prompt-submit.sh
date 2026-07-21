#!/usr/bin/env bash
# Codex UserPromptSubmit hook. This helper only mirrors the stdin payload; it never rewrites the
# pane registry or resume flags, which remain the SessionStart hook's responsibility.
BIN="$(cd "$(dirname "$0")" && pwd)"
# Prompt history is auxiliary. Match the Claude capture path's fail-open behavior so an incomplete
# first-launch repair or an unexpected local filesystem error can never reject a Codex prompt.
"$BIN/prompt-mirror.sh" codex >/dev/null 2>&1 || true
exit 0
