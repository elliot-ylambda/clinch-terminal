# The same pane-local Codex launch used by the bash/zsh integration.
if test "$WARP_IS_LOCAL_SHELL_SESSION" = 1
    and test -n "$WARP_TERMINAL_SESSION_UUID"
    and test -n "$CLINCH_CONTROL_WRAPPER"
    and test -x (command dirname "$CLINCH_CONTROL_WRAPPER")/clinch_codex_launch
    and not functions -q codex
    function codex
        set -l launcher (command dirname "$CLINCH_CONTROL_WRAPPER")/clinch_codex_launch
        set -l capture_state "$CLINCH_AGENT_STATE_DIR"
        if test -z "$capture_state"
            set capture_state "$HOME/Library/Application Support/sh.clinch.Clinch/agent-integration"
        end
        if test -t 0; and test -t 1; and test -f "$capture_state/enabled"; and test -x "$launcher"
            "$launcher" $argv
        else
            command codex $argv
        end
    end
end
