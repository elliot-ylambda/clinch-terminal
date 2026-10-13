import contextlib
import io
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import session_cli as cli
import session_inventory as inventory
import session_worker as worker


class SessionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="clinch-sessions-test-")
        self.root = Path(self.temp.name).resolve()
        self.home = self.root / "home"
        self.home.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.calls = self.root / "orca-calls.jsonl"
        self.env = patch.dict(
            os.environ,
            {
                "HOME": str(self.home),
                "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                "ORCA_PAIRING_CODE": "should-not-route-remotely",
                "ORCA_ENVIRONMENT": "wrong-host",
                "WARP_AGENT_RESUME_DIR": str(self.home / ".warp/agent-resume"),
                "CODEX_HOME": str(self.home / ".codex"),
                "CLAUDE_CONFIG_DIR": str(self.home / ".claude"),
                "PYTHONDONTWRITEBYTECODE": "1",
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": os.devnull,
            },
        )
        self.env.start()
        self.repo = self.root / "source repo"
        self.repo.mkdir()
        self.git("init", "-b", "main")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "user.name", "Fixture")
        (self.repo / "file.txt").write_text("base\n")
        (self.repo / ".gitignore").write_text(".env\ncache/\n")
        self.git("add", ".")
        self.git("commit", "-m", "fixture")
        self.sid = "aaaaaaaa-1111-2222-3333-444444444444"
        self.transcript = self.home / ".claude/projects/source" / (self.sid + ".jsonl")
        self.transcript.parent.mkdir(parents=True)
        self.transcript.write_text(
            json.dumps(
                {
                    "sessionId": self.sid,
                    "type": "user",
                    "cwd": str(self.repo),
                    "message": {"role": "user", "content": "PRIVATE PROMPT"},
                }
            )
            + "\n"
        )
        self.record = {
            "agent": "claude",
            "session_id": self.sid,
            "cwd": str(self.repo),
            "title": "Quotes ' and $(not-a-command)",
            "section": "Backburner",
        }
        self.destination_home = self.root / "destination-agent-home"
        self.orca = self.bin / "orca"
        self.orca.write_text(
            "#!"
            + sys.executable
            + "\n"
            + "import os,sys,json\n"
            + "assert 'ORCA_PAIRING_CODE' not in os.environ\n"
            + "assert 'ORCA_ENVIRONMENT' not in os.environ\n"
            + "with open("
            + repr(str(self.calls))
            + ", 'a') as f: f.write(json.dumps(sys.argv[1:])+'\\n')\n"
            + "print(json.dumps({'handle':'term_fixture'}))\n"
        )
        self.orca.chmod(0o755)
        (self.bin / "claude").write_text("#!/bin/sh\nexit 0\n")
        (self.bin / "claude").chmod(0o755)
        self.db = self.root / "saved.sqlite"
        with sqlite3.connect(self.db) as db:
            db.executescript("""
                CREATE TABLE windows(id,project_index);
                CREATE TABLE tabs(id,window_id,custom_title,pinned,color,tab_group_id);
                CREATE TABLE tab_groups(id,window_id,name,color,collapsed,pinned);
                CREATE TABLE pane_nodes(id,tab_id,parent_pane_node_id,flex);
                CREATE TABLE pane_leaves(pane_node_id,custom_vertical_tabs_title);
                CREATE TABLE terminal_panes(id,uuid,cwd,on_restore_command);
                INSERT INTO windows VALUES(1,0),(2,1);
                INSERT INTO tab_groups VALUES(1,1,'Backburner','red',1,0);
                INSERT INTO tabs VALUES(1,1,'Tab',0,NULL,1),(2,2,'Shell',0,NULL,NULL);
                INSERT INTO pane_nodes VALUES(1,1,NULL,1),(2,2,NULL,1);
                INSERT INTO pane_leaves VALUES(1,'Visible title'),(2,NULL);
            """)
            db.execute(
                "INSERT INTO terminal_panes VALUES(?,?,?,?)",
                (
                    1,
                    bytes.fromhex("11" * 16),
                    str(self.repo),
                    "clinch_agent_resume_launch claude " + self.sid,
                ),
            )
            db.execute(
                "INSERT INTO terminal_panes VALUES(?,?,?,NULL)",
                (2, bytes.fromhex("22" * 16), str(self.repo)),
            )

    def tearDown(self):
        self.env.stop()
        self.temp.cleanup()

    def git(self, *args, cwd=None):
        return worker.run(["git", *args], cwd or self.repo)

    def export(self):
        return worker.export_session(self.record)

    def request(self, exported=None):
        exported = exported or self.export()
        return {
            "bundle": exported["bundle"],
            "sha256": exported["sha256"],
            "destination": str(self.root / "destination repo"),
            "agent_home": str(self.destination_home),
            "orca": str(self.orca),
        }

    def test_inventory_includes_inactive_projects_and_section_metadata(self):
        data = inventory.inventory(str(self.db))
        self.assertEqual(len(data["sessions"]), 1)
        self.assertEqual(data["sessions"][0]["title"], "Visible title")
        self.assertEqual(data["sessions"][0]["section"], "Backburner")
        self.assertEqual(data["skipped"][0]["project_id"], 2)
        self.assertNotIn("PRIVATE PROMPT", json.dumps(data))

    def test_inventory_reads_wal_and_honors_registry(self):
        registry = Path(os.environ["WARP_AGENT_RESUME_DIR"])
        registry.mkdir(parents=True)
        (registry / ("11" * 16 + ".json")).write_text(
            json.dumps({"command": "cleared"})
        )
        self.assertEqual(inventory.inventory(str(self.db))["sessions"], [])
        with sqlite3.connect(self.db) as writer:
            writer.execute("PRAGMA journal_mode=WAL")
            writer.execute("UPDATE tabs SET custom_title='Updated' WHERE id=2")
            writer.commit()
            self.assertEqual(
                inventory.inventory(str(self.db))["skipped"][1]["title"], "Updated"
            )

    def test_ambiguous_databases_require_selection(self):
        for name in ("sh.clinch.Clinch", "sh.clinch.ClinchDev"):
            path = self.home / "Library/Application Support" / name / "warp.sqlite"
            path.parent.mkdir(parents=True)
            path.touch()
        with self.assertRaisesRegex(ValueError, "Several Clinch"):
            inventory.inventory()

    def test_dry_run_writes_nothing_and_does_not_launch(self):
        before = {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = cli.main(
                [
                    "move",
                    "claude:" + self.sid,
                    "--database",
                    str(self.db),
                    "--to",
                    "local",
                    "--dest",
                    str(self.root / "target"),
                    "--dry-run",
                    "--json",
                ]
            )
        self.assertEqual(code, 0, output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["status"], "ready")
        self.assertNotIn("PRIVATE PROMPT", output.getvalue())
        after = {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        self.assertEqual(before, after)

    def test_open_all_reports_running_session_without_launch(self):
        with (
            patch.object(worker, "active_owners", return_value=[123]),
            contextlib.redirect_stdout(io.StringIO()) as out,
        ):
            code = cli.main(
                ["open-in", "orca", "--all", "--database", str(self.db), "--json"]
            )
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(out.getvalue())["sessions"][0]["status"], "blocked")
        self.assertFalse(self.calls.exists())

    def test_open_preserves_cwd_and_quotes_resume_arguments(self):
        worker.dispatch(
            {"operation": "open", "session": self.record, "orca": str(self.orca)}
        )
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertEqual(calls[0][:4], ["repo", "add", "--path", str(self.repo)])
        self.assertEqual(
            calls[1][calls[1].index("--title") + 1],
            "[Backburner] Quotes ' and $(not-a-command)",
        )
        import shlex

        command = shlex.split(calls[1][calls[1].index("--command") + 1])
        self.assertEqual(command[-2:], ["--resume", str(self.transcript)])

    def test_move_preserves_git_states_artifacts_and_source(self):
        (self.repo / "file.txt").write_text("staged\n")
        self.git("add", "file.txt")
        (self.repo / "file.txt").write_text("unstaged\n")
        (self.repo / "untracked file").write_bytes(b"\x00binary\xff")
        (self.repo / ".env").write_text("EXPLICITLY_SELECTED=fixture\n")
        sidecar = self.transcript.with_suffix("") / "subagents/agent-test.jsonl"
        sidecar.parent.mkdir(parents=True)
        sidecar.write_text('{"type":"assistant"}\n')
        self.record["include"] = [".env"]
        exported = self.export()
        request = self.request(exported)
        receipt = worker.import_session(request)
        destination = Path(request["destination"])
        self.assertEqual(receipt["status"], "launched")
        self.assertEqual(
            self.git("diff", "--cached", "--binary"),
            self.git("diff", "--cached", "--binary", cwd=destination),
        )
        self.assertEqual(
            self.git("diff", "--binary"), self.git("diff", "--binary", cwd=destination)
        )
        self.assertEqual(
            (destination / "untracked file").read_bytes(), b"\x00binary\xff"
        )
        self.assertEqual(
            (destination / ".env").read_bytes(), (self.repo / ".env").read_bytes()
        )
        self.assertEqual(
            (
                self.destination_home / sidecar.relative_to(self.home / ".claude")
            ).read_bytes(),
            sidecar.read_bytes(),
        )
        self.assertTrue(self.transcript.exists())
        self.assertTrue(self.repo.exists())
        self.assertNotIn("PRIVATE PROMPT", json.dumps(receipt))

    def test_move_preserves_default_branch_and_detached_head(self):
        self.git("branch", "-m", "master")
        for branch in ("master", "HEAD"):
            with self.subTest(branch=branch):
                if branch == "HEAD":
                    self.git("checkout", "--detach")
                request = self.request()
                destination = self.root / ("checkout-" + branch)
                request["destination"] = str(destination)
                worker.import_session(request)
                self.assertEqual(
                    self.git(
                        "rev-parse", "--abbrev-ref", "HEAD", cwd=destination
                    ).strip(),
                    branch.encode(),
                )
                self.assertEqual(
                    self.git("rev-parse", "HEAD", cwd=destination),
                    self.git("rev-parse", "HEAD"),
                )
                self.expire_launch()

    def test_complete_cli_move_and_reverse_to_new_checkout(self):
        with contextlib.redirect_stdout(io.StringIO()) as out:
            code = cli.main(
                [
                    "move",
                    "claude:" + self.sid,
                    "--database",
                    str(self.db),
                    "--to",
                    "local",
                    "--dest",
                    str(self.root / "destination repo"),
                    "--dest-agent-home",
                    str(self.destination_home),
                    "--json",
                ]
            )
        self.assertEqual(code, 0, out.getvalue())
        imported = json.loads(out.getvalue())["receipt"]["session"]
        # Model a completed destination turn before handing the conversation back.
        path = Path(imported["transcript"])
        with path.open("a") as log:
            log.write(
                json.dumps(
                    {
                        "sessionId": self.sid,
                        "type": "assistant",
                        "cwd": imported["cwd"],
                        "message": {"content": "new reply"},
                    }
                )
                + "\n"
            )
        self.expire_launch()
        exported = worker.export_session(imported)
        request = self.request(exported)
        request["destination"] = str(self.root / "returned checkout")
        request["agent_home"] = str(self.home / ".claude")
        returned = worker.import_session(request)
        self.assertEqual(returned["status"], "launched")
        self.assertEqual(self.transcript.read_bytes(), path.read_bytes())
        self.assertTrue(
            list((worker.state_dir() / "backups").rglob(self.sid + ".jsonl"))
        )

    def expire_launch(self):
        for path in (worker.state_dir() / "launches").glob("*.json"):
            value = json.loads(path.read_text())
            value["time"] = 0
            path.write_text(json.dumps(value))

    def test_session_lock_excludes_different_destination_checkouts(self):
        request = self.request()
        with (
            worker.session_lock(self.record),
            self.assertRaisesRegex(ValueError, "handoff.*in progress"),
        ):
            worker.import_session(request)
        worker.import_session(request)
        request["destination"] = str(self.root / "another target")
        with self.assertRaisesRegex(ValueError, "pending or recent"):
            worker.import_session(request)

    def test_symlink_at_session_artifact_root_is_rejected(self):
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "credential").write_text("must not be exported")
        self.transcript.with_suffix("").symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink|Symlink"):
            self.export()

    def test_destination_artifact_race_is_rechecked_before_launch(self):
        request = self.request()
        original = worker.run

        def racing(argv, *args, **kwargs):
            result = original(argv, *args, **kwargs)
            if argv[:3] == ["git", "remote", "remove"]:
                target = self.destination_home / self.transcript.relative_to(
                    self.home / ".claude"
                )
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("concurrent history")
            return result

        with (
            patch.object(worker, "run", side_effect=racing),
            self.assertRaisesRegex(ValueError, "artifact changed"),
        ):
            worker.import_session(request)
        self.assertFalse(self.calls.exists())

    def test_ignored_files_are_not_implicitly_exported(self):
        (self.repo / ".env").write_text("NOT_SELECTED=fixture\n")
        exported = self.export()
        self.assertTrue(exported["manifest"]["project"]["ignored_files_present"])
        self.assertNotIn("files/.env", exported["manifest"]["files"])

    def test_existing_checkout_is_never_overwritten(self):
        request = self.request()
        Path(request["destination"]).mkdir()
        with self.assertRaisesRegex(ValueError, "never overwritten"):
            worker.import_session(request)
        self.assertFalse(self.calls.exists())

    def test_conflicting_history_stops_before_checkout_or_launch(self):
        request = self.request()
        other = self.destination_home / self.transcript.relative_to(
            self.home / ".claude"
        )
        other.parent.mkdir(parents=True)
        other.write_text("different history")
        with self.assertRaisesRegex(ValueError, "different session history"):
            worker.import_session(request)
        self.assertEqual(other.read_text(), "different history")
        self.assertFalse(Path(request["destination"]).exists())

    def test_launch_failure_retains_recovery_receipt_without_retrying(self):
        request = self.request()
        with (
            patch.object(worker, "launch", side_effect=ValueError("Orca unavailable")),
            self.assertRaisesRegex(ValueError, "Import retained"),
        ):
            worker.import_session(request)
        receipts = list(
            (self.home / ".clinch/session-transfer/receipts").glob("*.json")
        )
        self.assertEqual(json.loads(receipts[0].read_text())["status"], "launch_failed")
        self.assertTrue(Path(request["destination"]).is_dir())
        with self.assertRaises(ValueError):
            worker.import_session(request)

    def test_corrupt_archive_has_no_side_effects(self):
        request = self.request()
        request["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "checksum"):
            worker.import_session(request)
        self.assertFalse(Path(request["destination"]).exists())

    def test_path_traversal_and_symlink_artifacts_are_rejected(self):
        for name in ("../escape", "/absolute", "a/../../escape", "a\\escape", "a//b"):
            with self.assertRaises(ValueError):
                worker.safe_relative(name)
        (self.repo / "leak").symlink_to(self.transcript)
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.export()

    def test_changed_source_aborts_export(self):
        original = worker.project_snapshot
        calls = [0]

        def changing(*args, **kwargs):
            calls[0] += 1
            if calls[0] == 2:
                (self.repo / "file.txt").write_text("changed after snapshot\n")
            return original(*args, **kwargs)

        with (
            patch.object(worker, "project_snapshot", side_effect=changing),
            self.assertRaisesRegex(ValueError, "changed during export"),
        ):
            self.export()

    def test_partial_or_wrong_transcript_is_rejected(self):
        self.transcript.write_text('{"unfinished":')
        with self.assertRaisesRegex(ValueError, "incomplete"):
            worker.inspect_session(self.record)

    def test_codex_rollout_and_descendant_transfer(self):
        home = self.home / ".codex"
        root = home / "sessions/2026/09/24"
        root.mkdir(parents=True)
        child = "bbbbbbbb-1111-2222-3333-444444444444"
        for sid in (self.sid, child):
            rows = [
                {"type": "session_meta", "payload": {"id": sid, "cwd": str(self.repo)}},
                {
                    "type": "response_item",
                    "payload": {"type": "message", "role": "user"},
                },
            ]
            if sid == self.sid:
                rows.append(
                    {
                        "type": "event_msg",
                        "payload": {
                            "type": "collab_agent_spawn_end",
                            "new_thread_id": child,
                        },
                    }
                )
            (root / ("rollout-test-" + sid + ".jsonl")).write_text(
                "\n".join(map(json.dumps, rows))
            )
        record = worker.inspect_session(dict(self.record, agent="codex"))
        self.assertEqual(len(worker.session_files(record)), 2)
        (root / ("rollout-test-" + child + ".jsonl")).unlink()
        with self.assertRaisesRegex(ValueError, "found 0"):
            worker.session_files(record)

    @unittest.skipUnless(shutil.which("codex"), "Codex CLI is not installed")
    def test_real_codex_can_index_fixture_without_creating_a_turn(self):
        home = self.home / ".codex"
        path = (
            home
            / "sessions/2026/09/24"
            / ("rollout-2026-09-24T12-00-00-" + self.sid + ".jsonl")
        )
        path.parent.mkdir(parents=True)
        rows = [
            {
                "timestamp": "2026-09-24T12:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": self.sid,
                    "timestamp": "2026-09-24T12:00:00Z",
                    "cwd": str(self.repo),
                    "originator": "codex_cli_rs",
                    "cli_version": "0.154.0",
                    "source": "cli",
                    "model_provider": "openai",
                },
            },
            {
                "timestamp": "2026-09-24T12:00:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Fixture only"}],
                },
            },
        ]
        path.write_text("\n".join(map(json.dumps, rows)) + "\n")
        before = path.read_bytes()
        record = worker.inspect_session(dict(self.record, agent="codex"))
        worker.verify_codex(record)
        self.assertEqual(path.read_bytes(), before)

    def test_codex_probe_error_does_not_launch_orca(self):
        fake = self.bin / "codex"
        fake.write_text(
            "#!"
            + sys.executable
            + "\n"
            + "import sys,json\nfor line in sys.stdin:\n"
            + " r=json.loads(line)\n if 'id' not in r: continue\n"
            + " v={'result':{}} if r['method']=='initialize' else {'error':{'message':'missing'}}\n"
            + " print(json.dumps(dict(v,id=r['id'])),flush=True)\n"
        )
        fake.chmod(0o755)
        with self.assertRaisesRegex(ValueError, "could not read"):
            worker.launch(
                dict(self.record, agent="codex", agent_home=str(self.home / ".codex"))
            )
        self.assertFalse(self.calls.exists())

    def test_remote_bootstrap_uses_stdin_and_works_without_installed_clinch(self):
        real_run = subprocess.run
        captured = []

        def ssh(argv, **kwargs):
            if argv[0] == "ssh":
                captured.append(argv)
                import shlex

                return real_run(shlex.split(argv[-1]), **kwargs)
            return real_run(argv, **kwargs)

        with patch.object(cli.subprocess, "run", side_effect=ssh):
            result = cli.call("devbox", "inventory", database=str(self.db))
        self.assertEqual(result["sessions"][0]["session_id"], self.sid)
        self.assertNotIn(self.sid, " ".join(captured[0]))
        self.assertNotIn(str(self.db), " ".join(captured[0]))
        with self.assertRaises(ValueError):
            cli.call("-oProxyCommand=oops", "inventory")


if __name__ == "__main__":
    unittest.main()
