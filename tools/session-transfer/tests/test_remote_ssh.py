"""Opt-in real SSH round trip, restricted to temporary fixture homes on both hosts."""

import os
import re
import shlex
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

import test_sessions

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import session_cli as cli


@unittest.skipUnless(
    os.environ.get("CLINCH_SESSION_TEST_HOST"), "Opt-in SSH fixture test"
)
class SshRoundTrip(unittest.TestCase):
    def test_round_trip_with_changed_history_and_checkout(self):
        host = os.environ["CLINCH_SESSION_TEST_HOST"]
        fixture = test_sessions.SessionTests()
        fixture.setUp()
        actual_run = subprocess.run
        remote = None

        def ssh_script(source):
            return actual_run(
                [
                    "ssh",
                    "-o",
                    "BatchMode=yes",
                    "--",
                    host,
                    "python3 -c " + shlex.quote(source),
                ],
                capture_output=True,
                check=True,
            ).stdout

        try:
            remote = (
                ssh_script(
                    "import tempfile; print(tempfile.mkdtemp(prefix='clinch-ssh-fixture-'))"
                )
                .decode()
                .strip()
            )
            self.assertRegex(remote, r"^/tmp/clinch-ssh-fixture-[A-Za-z0-9_-]+$")
            fake_orca = remote + "/orca"
            setup = (
                "from pathlib import Path\n"
                f"root=Path({remote!r})\n"
                "(root/'home').mkdir()\n"
                f"p=Path({fake_orca!r})\n"
                'p.write_text(\'#!/usr/bin/env python3\\nimport json; print(json.dumps({"handle":"fixture"}))\\n\')\n'
                "p.chmod(0o755)\n"
            )
            ssh_script(setup)

            def isolated_run(argv, **kwargs):
                if argv[0] == "ssh" and argv[-1].startswith("python3 -c "):
                    argv = list(argv)
                    env = {
                        "HOME": remote + "/home",
                        "WARP_AGENT_RESUME_DIR": remote + "/home/.warp/agent-resume",
                        "CLAUDE_CONFIG_DIR": remote + "/home/.claude",
                        "CODEX_HOME": remote + "/home/.codex",
                        "PYTHONDONTWRITEBYTECODE": "1",
                    }
                    argv[-1] = (
                        "env "
                        + " ".join(shlex.quote(k + "=" + v) for k, v in env.items())
                        + " "
                        + argv[-1]
                    )
                return actual_run(argv, **kwargs)

            with patch.object(cli.subprocess, "run", side_effect=isolated_run):
                exported = cli.call("local", "export", session=fixture.record)
                received = cli.call(
                    host,
                    "import",
                    bundle=exported["bundle"],
                    sha256=exported["sha256"],
                    destination=remote + "/checkout",
                    agent_home=remote + "/home/.claude",
                    orca=fake_orca,
                )
                self.assertEqual(received["status"], "launched")
                record = received["session"]
                self.assertTrue(record["cwd"].startswith(remote))

                # Simulate a finished destination turn; the fake Orca never starts
                # Claude, and no model/API request is made on either host.
                advance = (
                    "import json\nfrom pathlib import Path\n"
                    f"record={record!r}\n"
                    "with Path(record['transcript']).open('a') as f:\n"
                    " f.write(json.dumps({'sessionId':record['session_id'],'type':'assistant','cwd':record['cwd'],'message':{'content':'Fixture response'}})+'\\n')\n"
                    "(Path(record['cwd'])/'file.txt').write_text('Changed on Linux\\n')\n"
                    f"for p in Path({remote!r}+'/home/.clinch/session-transfer/launches').glob('*.json'):\n"
                    " v=json.loads(p.read_text());v['time']=0;p.write_text(json.dumps(v))\n"
                )
                ssh_script(advance)
                returning = cli.call(host, "export", session=record)
                returned = cli.call(
                    "local",
                    "import",
                    bundle=returning["bundle"],
                    sha256=returning["sha256"],
                    destination=str(fixture.root / "returned"),
                    agent_home=str(fixture.home / ".claude"),
                    orca=str(fixture.orca),
                )
                self.assertEqual(returned["status"], "launched")
                self.assertEqual(
                    (fixture.root / "returned/file.txt").read_text(),
                    "Changed on Linux\n",
                )
                self.assertIn("Fixture response", fixture.transcript.read_text())
                self.assertEqual((fixture.repo / "file.txt").read_text(), "base\n")
        finally:
            if remote and re.fullmatch(
                r"/tmp/clinch-ssh-fixture-[A-Za-z0-9_-]+", remote
            ):
                ssh_script(f"import shutil; shutil.rmtree({remote!r})")
            fixture.tearDown()


if __name__ == "__main__":
    unittest.main()
