# Keep an interactive Codex session's hooks attached to this Clinch pane. A shared
# Codex daemon cannot inherit each client's pane UUID. This shell-local function
# leaves user-defined commands, other terminals, and noninteractive scripts alone.
if [ "${WARP_IS_LOCAL_SHELL_SESSION:-}" = 1 ] \
    && [ -n "${WARP_TERMINAL_SESSION_UUID:-}" ] \
    && [ -n "${CLINCH_CONTROL_WRAPPER:-}" ] \
    && [ -x "${CLINCH_CONTROL_WRAPPER%/*}/clinch_codex_launch" ] \
    && ! typeset -f codex >/dev/null 2>&1; then
    function codex() {
        local launcher="${CLINCH_CONTROL_WRAPPER%/*}/clinch_codex_launch"
        local capture_state="${CLINCH_AGENT_STATE_DIR:-$HOME/Library/Application Support/sh.clinch.Clinch/agent-integration}"
        if [ -t 0 ] && [ -t 1 ] && [ -f "$capture_state/enabled" ] && [ -x "$launcher" ]; then
            "$launcher" "$@"
        else
            command codex "$@"
        fi
    }
fi
