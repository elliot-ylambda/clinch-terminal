"""Real Git fixtures exercise cleanup's destructive boundary; no user checkout is removed."""

import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


def load_script(name):
    loader = importlib.machinery.SourceFileLoader(name, str(Path(__file__).resolve().parents[1] / name))
    spec = importlib.util.spec_from_loader(name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


cleanup = load_script("clean-worktrees")
watcher = load_script("watch-clinch-resources")


class CleanupTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="clinch-maintenance-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.repo = self.root / "repo"
        self.managed = self.root / "managed"
        self.managed.mkdir()
        self.repo.mkdir()
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.test")
        self.git("config", "commit.gpgsign", "false")
        (self.repo / "tracked").write_text("original\n")
        (self.repo / ".gitignore").write_text("target/\n.env\n")
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")
        self.base = self.git("rev-parse", "HEAD").stdout.decode().strip()
        self.checkout = self.managed / "done with spaces"
        self.git("worktree", "add", "-b", "done", str(self.checkout))

    def git(self, *args):
        return cleanup.git(self.repo, *args)

    def inspect(self, checkout=None, directories=(), process_error=None):
        checkout = checkout or self.checkout
        record = next(record for record in cleanup.worktrees(self.repo) if record["worktree"] == str(checkout))
        return cleanup.inspect(self.repo, record, self.repo, self.managed, self.base, directories, process_error)

    def invoke(self, *args):
        output = io.StringIO()
        with patch.object(sys, "argv", ["clean-worktrees", "--repo", str(self.repo), "--managed-root", str(self.managed), *args]), \
                patch.object(cleanup, "live_directories", return_value=[(99999, self.root)]), \
                contextlib.redirect_stdout(output):
            status = cleanup.main()
        return status, output.getvalue()

    def test_preview_preserves_checkout_and_branch(self):
        status, output = self.invoke("--json")
        self.assertEqual(status, 0)
        entries = json.loads(output)["worktrees"]
        self.assertFalse(entries[0]["eligible"])
        self.assertTrue(entries[1]["eligible"])
        self.assertTrue(self.checkout.is_dir())
        self.git("show-ref", "--verify", "refs/heads/done")

    def test_explicit_removal_includes_ignored_builds_but_preserves_branch(self):
        (self.checkout / "target").mkdir()
        (self.checkout / "target/artifact").write_text("regenerable")
        (self.checkout / ".env").write_text("fixture, not a secret")
        status, output = self.invoke("--remove", str(self.checkout))
        self.assertEqual(status, 0)
        self.assertIn('"target/"', output)
        self.assertIn('".env"', output)
        self.assertTrue(self.checkout.exists())
        status, _ = self.invoke("--remove", str(self.checkout), "--apply")
        self.assertEqual(status, 0)
        self.assertFalse(self.checkout.exists())
        self.git("show-ref", "--verify", "refs/heads/done")

    def test_dirty_and_untracked_are_protected(self):
        for filename in ["tracked", "untracked"]:
            with self.subTest(filename=filename):
                (self.checkout / filename).write_text("unfinished work")
                with self.assertRaisesRegex(RuntimeError, "protected"):
                    self.invoke("--remove", str(self.checkout), "--apply")
                self.assertTrue(self.checkout.exists())
                cleanup.git(self.checkout, "reset", "--hard", "HEAD")
                if filename == "untracked":
                    (self.checkout / filename).unlink()

    def test_unmerged_locked_and_detached_are_protected(self):
        cleanup.git(self.checkout, "commit", "--allow-empty", "-qm", "unfinished")
        self.assertIn("commits not merged into base", self.inspect()["blocked_by"])
        self.git("worktree", "lock", str(self.checkout))
        self.assertIn("locked", self.inspect()["blocked_by"])
        self.git("worktree", "unlock", str(self.checkout))
        cleanup.git(self.checkout, "checkout", "--detach", "HEAD")
        self.assertIn("detached HEAD or bare repository", self.inspect()["blocked_by"])

    def test_hidden_tracked_modifications_are_protected(self):
        for flag in ["assume-unchanged", "skip-worktree"]:
            with self.subTest(flag=flag):
                cleanup.git(self.checkout, "update-index", "--" + flag, "tracked")
                (self.checkout / "tracked").write_text("hidden unfinished work")
                self.assertEqual(cleanup.git(self.checkout, "status", "--porcelain").stdout, b"")
                with self.assertRaisesRegex(RuntimeError, "protected"):
                    self.invoke("--remove", str(self.checkout), "--apply")
                self.assertEqual((self.checkout / "tracked").read_text(), "hidden unfinished work")
                cleanup.git(self.checkout, "update-index", "--no-" + flag, "tracked")
                cleanup.git(self.checkout, "reset", "--hard", "HEAD")

    def test_current_primary_and_external_checkouts_are_protected(self):
        self.assertIn("primary checkout", self.inspect(self.repo)["blocked_by"])
        with patch.object(Path, "cwd", return_value=self.checkout / "subdir"):
            self.assertIn("current working directory", self.inspect()["blocked_by"])
        outside = self.root / "outside"
        self.git("worktree", "add", "-b", "outside", str(outside))
        self.assertIn("outside the managed worktree root", self.inspect(outside)["blocked_by"])

    def test_live_cwd_and_failed_process_check_are_protected(self):
        entry = self.inspect(directories=[(123, self.checkout / "subdir")])
        self.assertIn("in use by process(es): 123", entry["blocked_by"])
        self.assertFalse(self.inspect(process_error="lsof unavailable")["eligible"])

    def test_escaped_paths_are_inventoried_but_protected(self):
        for i, name in enumerate(["line\nwith\ttab", "literal\\nbackslash", "café"]):
            unusual = self.managed / name
            self.git("worktree", "add", "-b", f"unusual-{i}", str(unusual))
            entry = self.inspect(unusual)
            self.assertEqual(entry["path"], str(unusual))
            self.assertIn("path contains characters that process inspection may escape", entry["blocked_by"])
            with self.assertRaisesRegex(RuntimeError, "protected"):
                self.invoke("--remove", str(unusual), "--apply")
            self.assertTrue(unusual.exists())

    def test_paths_are_not_shell_interpreted(self):
        unusual = self.managed / "with spaces and $(false)"
        self.git("worktree", "add", "-b", "unusual", str(unusual))
        status, _ = self.invoke("--remove", str(unusual), "--apply")
        self.assertEqual(status, 0)
        self.assertFalse(unusual.exists())

    def test_symlink_target_is_rejected(self):
        alias = self.managed / "alias"
        alias.symlink_to(self.checkout)
        with self.assertRaisesRegex(RuntimeError, "exact registered"):
            self.invoke("--remove", str(alias), "--apply")
        # Also reject a registered checkout replaced with a symlink.
        moved = self.root / "moved"
        self.checkout.rename(moved)
        self.checkout.symlink_to(moved)
        self.assertIn("path contains a symlink", self.inspect()["blocked_by"])

    def test_revalidation_prevents_new_activity_and_dirty_files(self):
        original = cleanup.live_directories
        calls = 0

        def new_activity():
            nonlocal calls
            calls += 1
            if calls == 2:
                (self.checkout / "tracked").write_text("changed after preview")
            return [(99999, self.root)]

        with patch.object(sys, "argv", ["clean-worktrees", "--repo", str(self.repo), "--managed-root", str(self.managed), "--remove", str(self.checkout), "--apply"]), \
                patch.object(cleanup, "live_directories", side_effect=new_activity), \
                contextlib.redirect_stdout(io.StringIO()), \
                self.assertRaisesRegex(RuntimeError, "Worktree changed"):
            cleanup.main()
        self.assertTrue(self.checkout.exists())
        self.assertIs(cleanup.live_directories, original)

    def test_failed_or_newly_busy_process_inspection_blocks_apply(self):
        for results in [[OSError("lsof unavailable")], [[(99999, self.root)], [(123, self.checkout)]]]:
            with patch.object(sys, "argv", ["clean-worktrees", "--repo", str(self.repo), "--managed-root", str(self.managed), "--remove", str(self.checkout), "--apply"]), \
                    patch.object(cleanup, "live_directories", side_effect=results), \
                    contextlib.redirect_stdout(io.StringIO()), \
                    self.assertRaises(RuntimeError):
                cleanup.main()
            self.assertTrue(self.checkout.exists())

    def test_stale_registration_is_not_removed(self):
        import shutil
        shutil.rmtree(self.checkout)
        status, output = self.invoke("--remove", str(self.checkout))
        self.assertEqual(status, 1)
        self.assertIn("missing/stale registration", output)
        self.assertEqual(len(cleanup.worktrees(self.repo)), 2)

    def test_lsof_nul_records_preserve_spaces(self):
        raw = b"p123\0\nfcwd\0n/tmp/with spaces\0\np456\0\nfcwd\0n/\0\n"
        with patch.object(cleanup, "run", return_value=subprocess.CompletedProcess([], 0, raw)):
            self.assertEqual(cleanup.live_directories(), [(123, Path("/tmp/with spaces").resolve()), (456, Path("/"))])
        with patch.object(cleanup, "run", return_value=subprocess.CompletedProcess([], 0, b"")):
            with self.assertRaisesRegex(RuntimeError, "no process directories"):
                cleanup.live_directories()


class ResourceTests(unittest.TestCase):
    def test_descendants_exclude_unrelated_processes_and_handle_cycles(self):
        self.assertEqual(watcher.descendants(10, {11: 10, 12: 11, 13: 1, 10: 12}), {11, 12})

    def test_cpu_uses_interval_and_rejects_reused_pid(self):
        previous, current = watcher.RusageInfo(), watcher.RusageInfo()
        previous.proc_start_abstime = current.proc_start_abstime = 7
        previous.user_time = 1_000_000_000
        current.user_time, current.system_time = 2_000_000_000, 500_000_000
        self.assertEqual(watcher.cpu_percent(current, previous, 5, 1), 30)
        self.assertAlmostEqual(watcher.cpu_percent(current, previous, 5, 125 / 3), 1250)
        self.assertIsNone(watcher.cpu_percent(current, None, 5, 1))
        current.proc_start_abstime = 8
        self.assertIsNone(watcher.cpu_percent(current, previous, 5, 1))


if __name__ == "__main__":
    unittest.main()
