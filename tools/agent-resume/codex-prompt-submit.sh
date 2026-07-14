#!/usr/bin/env bash
# Codex UserPromptSubmit hook. This helper only mirrors the stdin payload; it never rewrites the
# pane registry or resume flags, which remain the SessionStart hook's responsibility.
BIN="$(cd "$(dirname "$0")" && pwd)"
exec "$BIN/prompt-mirror.sh" codex
