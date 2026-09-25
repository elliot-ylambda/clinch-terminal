import contextlib
import io
import json
import os
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

import test_sessions

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import session_cli as cli
import session_handoff as handoff
import session_inventory as inventory
import session_worker as worker


class HandoffTests(unittest.TestCase):
    def setUp(self):
        self.fixture = test_sessions.SessionTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.tearDown)
        self.root = self.fixture.root
        self.log = self.root / "control.jsonl"
        self.wrapper = self.fixture.bin / "clinch"
        self.wrapper.write_text(
            "#!"
            + sys.executable
            + "\nimport json,os,sys\n"
            + f"with open({str(self.log)!r},'a') as f: f.write(json.dumps({{'args':sys.argv[1:],'pane':os.environ.get('WARP_TERMINAL_SESSION_UUID')}})+'\\n')\n"
            + "if 'create' in sys.argv and 'tab' in sys.argv: print(json.dumps({'created':True,'window':{'id':'window-exact'},'tab':{'id':'tab-exact'}}))\n"
            + "elif 'section' in sys.argv and 'list' in sys.argv: print(json.dumps({'sections':[]}))\n"
            + "else: print(json.dumps({'ok':True}))\n"
        )
        self.wrapper.chmod(0o755)
        self.env = patch.dict(
            os.environ,
            {
                "CLINCH_CONTROL_WRAPPER": str(self.wrapper),
                "CLINCH_CONTROL_PID": "123",
                "WARP_TERMINAL_SESSION_UUID": "11" * 16,
                "CODEX_THREAD_ID": "",
            },
        )
        self.env.start()
        self.addCleanup(self.env.stop)

    def args(self, *extra):
        return cli.parser().parse_args(
            ["to-devbox", "--database", str(self.fixture.db), *extra]
        )

    def routed(self, host, operation, **params):
        if host == "devbox":
            if operation == "home":
                return str(self.root / "devbox-home")
            if operation == "import":
                params["agent_home"] = str(self.fixture.destination_home)
                params["orca"] = str(self.fixture.orca)
        return worker.dispatch(dict(params, operation=operation))

    def test_current_pane_resolution_and_preview_do_not_queue(self):
        with patch.object(handoff, "call", side_effect=self.routed):
            result = handoff.prepare(self.args("--dry-run"))
        self.assertEqual(result["job"]["session"]["session_id"], self.fixture.sid)
        self.assertEqual(result["job"]["session"]["origin_pane"], "11" * 16)
        self.assertEqual(result["status"], "preview")
        self.assertFalse((worker.state_dir() / "jobs").exists())
        calls = [json.loads(line)["args"] for line in self.log.read_text().splitlines()]
        self.assertTrue(all("create" not in call for call in calls))

    def test_missing_binding_and_conflicting_current_thread_fail_closed(self):
        with (
            patch.dict(os.environ, CLINCH_CONTROL_WRAPPER="/missing"),
            self.assertRaisesRegex(ValueError, "current Clinch terminal"),
        ):
            handoff.prepare(self.args("--dry-run"))
        with (
            patch.object(
                inventory,
                "inventory",
                return_value={
                    "sessions": [
                        {
                            "pane_id": "11" * 16,
                            "agent": "codex",
                            "session_id": "another",
                        }
                    ]
                },
            ),
            patch.dict(os.environ, CODEX_THREAD_ID="current-thread"),
            self.assertRaisesRegex(ValueError, "disagree"),
        ):
            handoff.current_record(self.args())
        with (
            patch.dict(os.environ, CODEX_THREAD_ID="current-thread"),
            self.assertRaisesRegex(ValueError, "disagree"),
        ):
            handoff.current_record(self.args())

    def test_queue_waits_for_exit_and_copies_last_turn(self):
        inspected = 0

        def route(host, operation, **params):
            nonlocal inspected
            value = self.routed(host, operation, **params)
            if operation == "inspect":
                inspected += 1
                value["active_pids"] = [123] if inspected == 1 else []
            return value

        def finish_turn(_):
            if inspected == 1:
                with self.fixture.transcript.open("a") as log:
                    log.write(
                        json.dumps(
                            {
                                "sessionId": self.fixture.sid,
                                "type": "assistant",
                                "message": {"content": "final saved turn"},
                            }
                        )
                        + "\n"
                    )

        with (
            patch.object(handoff, "call", side_effect=route),
            patch.object(handoff.time, "sleep", side_effect=finish_turn),
        ):
            result = handoff.prepare(self.args())
            self.assertEqual(result["status"], "queued")
            self.assertFalse(self.fixture.destination_home.exists())
            with self.assertRaisesRegex(ValueError, "already pending"):
                handoff.prepare(self.args())
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(handoff.run_job(result["job_id"]), 0)
        job = json.loads(handoff.job_path(result["job_id"]).read_text())
        self.assertEqual(job["status"], "complete")
        self.assertIn(
            "final saved turn",
            Path(job["receipt"]["session"]["transcript"]).read_text(),
        )
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(all(call["pane"] == "11" * 16 for call in calls))
        launch = next(call["args"] for call in calls if "_run-handoff" in call["args"])
        self.assertNotIn("--window", launch)
        with self.assertRaisesRegex(ValueError, "cannot be replayed"):
            handoff.run_job(result["job_id"])

    def test_cancel_cannot_race_transfer_or_launch(self):
        with patch.object(handoff, "call", side_effect=self.routed):
            result = handoff.prepare(self.args())
        job = json.loads(handoff.job_path(result["job_id"]).read_text())
        handoff.save_job(job, expected="waiting", status="cancelled")
        with self.assertRaisesRegex(ValueError, "status changed"):
            handoff.save_job(job, expected="waiting", status="transferring")
        self.assertFalse(self.fixture.destination_home.exists())

    def test_clinch_launch_uses_original_project_and_exact_section_tab(self):
        request = self.fixture.request()
        manifest, _ = worker.unpack_bundle(request["bundle"], request["sha256"])
        self.assertEqual(manifest["session"]["session_id"], self.fixture.sid)
        request["app"] = "clinch"
        receipt = worker.import_session(request)
        self.assertEqual(receipt["terminal"]["app"], "clinch")
        calls = [json.loads(line)["args"] for line in self.log.read_text().splitlines()]
        create = next(args for args in calls if "--resume" in args)
        self.assertNotIn("--window", create)
        section = next(args for args in calls if "section" in args and "create" in args)
        self.assertIn("tab-exact", section)
        self.assertIn("Transferred", section)

    def test_existing_section_uses_opaque_id_and_inactive_project_is_recoverable(self):
        bound = handoff.binding()
        tab = {"window": {"id": "w"}, "tab": {"id": "t"}}
        with patch.object(
            handoff,
            "control",
            side_effect=[
                {},
                {"sections": [{"name": "Transferred", "section_id": "opaque-group"}]},
                {},
            ],
        ) as control:
            self.assertIsNone(handoff.group_tab(bound, tab, "Title"))
        self.assertIn("opaque-group", control.call_args.args)
        with patch.object(
            handoff, "control", side_effect=ValueError("stale target")
        ) as control:
            self.assertIn("original project", handoff.group_tab(bound, tab, "Title"))
        self.assertEqual(control.call_count, 1)

    def test_inventory_preserves_receipt_lineage_for_saved_returned_pane(self):
        record = dict(
            self.fixture.record,
            ancestor_hashes={"path": ["ancestor"]},
            origin_pane="original-pane",
            project_name="project",
        )
        worker.write_receipt(
            worker.state_dir() / "receipts/fixture.json",
            {"status": "launched", "session": record},
        )
        rows = inventory.inventory(str(self.fixture.db))["sessions"]
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["ancestor_hashes"], {"path": ["ancestor"]})
        self.assertEqual(rows[0]["origin_pane"], "original-pane")

    def test_return_from_original_shell_uses_recorded_handoff_after_registry_clears(
        self,
    ):
        record = dict(self.fixture.record, origin_pane="11" * 16)
        job = {
            "id": "a" * 32,
            "created_at": 1,
            "direction": "to-devbox",
            "status": "complete",
            "host": "devbox",
            "session": record,
        }
        worker.write_receipt(handoff.job_path(job["id"]), job)
        args = cli.parser().parse_args(["from-devbox"])
        with patch.object(inventory, "inventory", return_value={"sessions": []}):
            selected = handoff.current_record(args)
        self.assertEqual(selected["session_id"], self.fixture.sid)

    def test_nested_cwd_keeps_orca_project_named_after_checkout(self):
        subdir = self.fixture.repo / "src"
        subdir.mkdir()
        (subdir / "tracked").write_text("content")
        self.fixture.git("add", ".")
        self.fixture.git("commit", "-m", "nested cwd")
        self.fixture.record["cwd"] = str(subdir)
        request = self.fixture.request()
        receipt = worker.import_session(request)
        calls = [
            json.loads(line) for line in self.fixture.calls.read_text().splitlines()
        ]
        self.assertEqual(calls[0][calls[0].index("--path") + 1], request["destination"])
        self.assertEqual(
            calls[1][calls[1].index("--worktree") + 1], "path:" + request["destination"]
        )
        self.assertEqual(
            receipt["session"]["cwd"], str(Path(request["destination"]) / "src")
        )

    def test_github_bootstrap_restores_unpublished_commits_and_supports_return(self):
        fixture = self.fixture
        remote = self.root / "github.git"
        fixture.git("clone", "--bare", str(fixture.repo), str(remote))
        url = "https://github.com/fixture/project.git"
        fixture.git("remote", "add", "origin", url)
        base = fixture.git("rev-parse", "HEAD").decode().strip()
        fixture.git("update-ref", "refs/remotes/origin/main", base)
        (fixture.repo / "file.txt").write_text("unpublished\n")
        fixture.git("commit", "-am", "unpublished")
        (fixture.repo / "file.txt").write_text("staged\n")
        fixture.git("add", ".")
        (fixture.repo / "file.txt").write_text("unstaged\n")
        fixture.record["github_bootstrap"] = True
        original_run = worker.run

        def local_github(argv, *args, **kwargs):
            if argv[0] == "git" and "fetch" in argv:
                argv = [
                    "git",
                    "-c",
                    "url." + remote.as_uri() + ".insteadOf=" + url,
                    *argv[1:],
                ]
            return original_run(argv, *args, **kwargs)

        with patch.object(worker, "run", side_effect=local_github):
            exported = fixture.export()
            self.assertEqual(exported["manifest"]["project"]["github"]["base"], base)
            request = fixture.request(exported)
            received = worker.import_session(request)
            cwd = received["session"]["cwd"]
            self.assertEqual(
                fixture.git("rev-parse", "HEAD", cwd=cwd),
                fixture.git("rev-parse", "HEAD"),
            )
            self.assertEqual(
                fixture.git("diff", "--cached", cwd=cwd),
                fixture.git("diff", "--cached"),
            )
            self.assertEqual(fixture.git("diff", cwd=cwd), fixture.git("diff"))
            self.assertEqual(
                fixture.git("config", "--get", "remote.origin.url", cwd=cwd)
                .decode()
                .strip(),
                url,
            )
            fixture.git("fsck", "--full", cwd=cwd)
            fixture.expire_launch()
            returning = worker.export_session(received["session"])
            self.assertEqual(returning["manifest"]["project"]["github"]["base"], base)

    def test_published_head_needs_no_uploaded_git_bundle_and_credentials_are_rejected(
        self,
    ):
        fixture = self.fixture
        fixture.git("remote", "add", "origin", "https://github.com/fixture/project.git")
        fixture.git(
            "update-ref",
            "refs/remotes/origin/main",
            fixture.git("rev-parse", "HEAD").decode().strip(),
        )
        fixture.record["github_bootstrap"] = True
        exported = fixture.export()
        self.assertNotIn("repo.bundle", exported["manifest"]["files"])
        fixture.git(
            "remote",
            "set-url",
            "origin",
            "https://token@github.com/fixture/project.git",
        )
        with self.assertRaisesRegex(ValueError, "credential-free"):
            worker.github_source(fixture.repo, "origin")

    def test_versioned_install_and_reinstall_keep_cli_working(self):
        installer = Path(__file__).resolve().parents[1] / "install-local"
        for _ in range(2):
            worker.run([sys.executable, str(installer)])
        command = self.fixture.home / ".local/bin/clinch-sessions"
        result = worker.run([str(command), "--help"]).decode()
        self.assertIn("to-devbox", result)
        self.assertIn("from-devbox", result)
        self.assertIn("transferred", result)


if __name__ == "__main__":
    unittest.main()
