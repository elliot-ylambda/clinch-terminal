#!/usr/bin/env python3
"""Exercise actual bash/zsh/fish command resolution through a PTY."""
import json
import os
from pathlib import Path
import pty
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]


class CodexShellTests(unittest.TestCase):
    def run_shell(self, shell, *, enabled=True, local=True, tty=True, custom=False, alias=False):
        with tempfile.TemporaryDirectory(prefix="clinch-codex-shell-") as directory:
            root = Path(directory)
            binary = root / "bin with spaces"
            binary.mkdir()
            state = root / "state"
            state.mkdir()
            if enabled:
                (state / "enabled").touch()
            output = root / "result.json"
            for name, source in [("codex", "native"), ("clinch_codex_launch", "pane")]:
                script = binary / name
                script.write_text(
                    "#!/usr/bin/env python3\n"
                    "import json, os, sys\n"
                    f"json.dump([{source!r}, sys.argv[1:]], open(os.environ['CODEX_SHELL_TEST_OUTPUT'], 'w'))\n"
                )
                script.chmod(0o755)
            env = dict(os.environ, PATH=str(binary) + os.pathsep + os.environ["PATH"],
                       WARP_IS_LOCAL_SHELL_SESSION="1" if local else "0",
                       WARP_TERMINAL_SESSION_UUID="pane-123",
                       CLINCH_CONTROL_WRAPPER=str(binary / "clinch"),
                       CLINCH_AGENT_STATE_DIR=str(state), CODEX_SHELL_TEST_OUTPUT=str(output))
            suffix = "fish" if shell == "fish" else "sh"
            integration = ROOT / "app/assets/bundled/bootstrap" / f"clinch_codex.{suffix}"
            env["CODEX_SHELL_TEST_INTEGRATION"] = str(integration)
            if shell == "fish":
                prefix = "function codex; command codex custom $argv; end; " if custom else ""
                script = prefix + 'source "$CODEX_SHELL_TEST_INTEGRATION"; codex --model "two words" "literal*"'
                command = [shutil.which(shell), "--no-config", "-c", script]
            else:
                prefix = 'function codex() { command codex custom "$@"; }; ' if custom else ""
                if alias:
                    prefix += ("shopt -s expand_aliases; " if shell == "bash" else "")
                    prefix += "alias codex='codex alias-argument'; "
                script = prefix + """. "$CODEX_SHELL_TEST_INTEGRATION"; eval 'codex --model "two words" "literal*"'"""
                command = (["/bin/zsh", "-f", "-c", script] if shell == "zsh" else
                           ["/bin/bash", "--noprofile", "--norc", "-c", script])
            if tty:
                master, slave = pty.openpty()
                try:
                    result = subprocess.run(command, env=env, stdin=slave, stdout=slave,
                                            stderr=subprocess.PIPE, timeout=10)
                finally:
                    os.close(slave)
                    os.close(master)
            else:
                result = subprocess.run(command, env=env, stdin=subprocess.DEVNULL,
                                        capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            return json.loads(output.read_text())

    def test_shell_launch_keeps_pane_identity_and_preserves_arguments(self):
        for shell in ["bash", "zsh"] + (["fish"] if shutil.which("fish") else []):
            for options, expected in [({}, "pane"), ({"enabled": False}, "native"),
                                      ({"local": False}, "native"), ({"tty": False}, "native"),
                                      ({"custom": True}, "native")]:
                with self.subTest(shell=shell, options=options):
                    result = self.run_shell(shell, **options)
                    self.assertEqual(result[0], expected)
                    args = ["--model", "two words", "literal*"]
                    if options.get("custom"):
                        args.insert(0, "custom")
                    self.assertEqual(result[1], args)

    def test_existing_shell_alias_still_passes_its_arguments(self):
        for shell in ["bash", "zsh"]:
            with self.subTest(shell=shell):
                self.assertEqual(self.run_shell(shell, alias=True),
                                 ["pane", ["alias-argument", "--model", "two words", "literal*"]])


if __name__ == "__main__":
    unittest.main()
