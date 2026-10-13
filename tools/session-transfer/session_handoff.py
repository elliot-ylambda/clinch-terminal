"""Current-pane shortcuts and visible, stopped-agent handoff jobs for Clinch."""

import fcntl
import json
import os
import re
import subprocess
import sys
import time
import uuid
from pathlib import Path

import session_inventory as inventory
import session_worker as worker


def call(host, operation, **params):
    from session_cli import call as request

    return request(host, operation, **params)


def binding():
    wrapper = os.environ.get("CLINCH_CONTROL_WRAPPER", "")
    pid = os.environ.get("CLINCH_CONTROL_PID", "")
    pane = os.environ.get("WARP_TERMINAL_SESSION_UUID", "")
    if (
        not Path(wrapper).is_file()
        or not os.access(wrapper, os.X_OK)
        or not pid.isdigit()
    ):
        raise ValueError(
            "Run this command in a current Clinch terminal with local control enabled"
        )
    if not re.fullmatch(r"[0-9a-fA-F-]{32,36}", pane):
        raise ValueError("Clinch did not supply the originating terminal identity")
    value = {"wrapper": wrapper, "pid": pid, "pane": pane}
    control(value, "app", "ping")
    return value


def control(bound, *args):
    env = dict(os.environ, WARP_TERMINAL_SESSION_UUID=bound["pane"])
    raw = worker.run(
        [
            bound["wrapper"],
            "ctrl",
            "--output-format",
            "json",
            *args,
            "--pid",
            bound["pid"],
        ],
        env=env,
    )
    result = json.loads(raw)
    if result.get("ok") is False:
        raise ValueError(
            result.get("error", {}).get("message", "Clinch control failed")
        )
    return result


def create_tab(bound, cwd, argv):
    env = dict(os.environ, WARP_TERMINAL_SESSION_UUID=bound["pane"])
    raw = worker.run(
        [
            bound["wrapper"],
            "ctrl",
            "--output-format",
            "json",
            "tab",
            "create",
            "--pid",
            bound["pid"],
            "--cwd",
            cwd,
            "--",
            *argv,
        ],
        env=env,
    )
    result = json.loads(raw)
    if not result.get("created"):
        raise ValueError("Clinch did not confirm creation of the handoff tab")
    return result


def group_tab(bound, tab, title):
    """Exact tab IDs make old app project-scoping limitations fail closed."""
    target = ["--window", tab["window"]["id"], "--tab", tab["tab"]["id"]]
    try:
        control(bound, "tab", "rename", title, *target)
        # Always create from this exact tab. A different active project rejects
        # the opaque tab ID before modifying any section. Do not guess by title.
        result = control(bound, "section", "list", "--window", tab["window"]["id"])
        matches = [
            s for s in result.get("sections", []) if s.get("name") == "Transferred"
        ]
        if len(matches) == 1:
            control(bound, "section", "tab", "add", matches[0]["section_id"], *target)
        else:
            control(bound, "section", "create", "Transferred", *target)
        return None
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        return (
            "Tab created; select its original project and run the organize command to group it. "
            + str(error)
        )


def launch_clinch(record, argv):
    bound = binding()
    if record.get("origin_pane"):
        bound["pane"] = record["origin_pane"]
    tab = create_tab(bound, record["cwd"], argv)
    title = "From DevBox · " + (record.get("title") or record["session_id"][:8])
    warning = group_tab(bound, tab, title)
    return {"app": "clinch", "tab": tab, "warning": warning}


def current_record(args):
    database = args.database
    if not database:
        wrapper = os.environ.get("CLINCH_CONTROL_WRAPPER", "")
        app = (
            "sh.clinch.ClinchDev"
            if "/Clinch Dev.app/" in wrapper
            else "sh.clinch.Clinch"
        )
        candidate = Path.home() / "Library/Application Support" / app / "warp.sqlite"
        if candidate.exists():
            database = str(candidate)
    data = inventory.inventory(database, args.registry)
    selected = args.session
    if selected == "current":
        pane = os.environ.get("WARP_TERMINAL_SESSION_UUID", "").replace("-", "").lower()
        records = [
            r
            for r in data["sessions"]
            if r.get("pane_id") == pane or pane in r.get("other_panes", [])
        ]
        thread = os.environ.get("CODEX_THREAD_ID")
        if not records and thread:
            records = [
                {
                    "agent": "codex",
                    "session_id": thread,
                    "cwd": os.getcwd(),
                    "title": None,
                    "section": None,
                }
            ]
        if not records and args.command == "from-devbox":
            previous = [
                j["session"]
                for j in jobs()
                if j["direction"] == "to-devbox"
                and j["status"] == "complete"
                and j["host"] == args.host
                and j["session"].get("origin_pane", "").replace("-", "").lower() == pane
            ]
            identities = {(r["agent"], r["session_id"]) for r in previous}
            if len(identities) == 1:
                records = [previous[0]]
        if len(records) != 1:
            raise ValueError(
                "Cannot identify this pane's agent; supply claude:SESSION_ID or codex:SESSION_ID"
            )
        record = dict(records[0])
        if thread and (record["agent"], record["session_id"]) != ("codex", thread):
            raise ValueError(
                "Current thread and pane registry disagree; retry after session capture updates"
            )
    else:
        if not re.fullmatch(r"(claude|codex):[A-Za-z0-9-]+", selected):
            raise ValueError(
                "Use current or an exact claude:SESSION_ID / codex:SESSION_ID"
            )
        agent, sid = selected.split(":", 1)
        records = [
            r for r in data["sessions"] if (r["agent"], r["session_id"]) == (agent, sid)
        ]
        if args.cwd:
            records = [r for r in records if r.get("cwd") == args.cwd]
        if len(records) > 1:
            raise ValueError("Several checkouts match; disambiguate with --cwd")
        record = (
            dict(records[0])
            if records
            else {"agent": agent, "session_id": sid, "cwd": args.cwd}
        )
    if args.cwd:
        record["cwd"] = args.cwd
    if args.agent_home:
        record["agent_home"] = args.agent_home
    return record


def job_path(identifier):
    if not re.fullmatch(r"[0-9a-f]{32}", identifier):
        raise ValueError("Invalid handoff job ID")
    return worker.state_dir() / "jobs" / (identifier + ".json")


def jobs():
    result = []
    for path in (worker.state_dir() / "jobs").glob("*.json"):
        try:
            result.append(json.loads(path.read_text()))
        except (OSError, ValueError):
            continue
    return sorted(result, key=lambda job: job["created_at"], reverse=True)


def save_job(job, expected=None, **changes):
    path = job_path(job["id"])
    with path.with_suffix(".state-lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        latest = json.loads(path.read_text())
        if expected is not None and latest["status"] != expected:
            raise ValueError(
                "Handoff status changed; inspect the task before proceeding"
            )
        latest.update(changes, updated_at=time.time())
        worker.write_receipt(path, latest)
        job.update(latest)


def returning_record(args):
    local = current_record(args)
    sid = (local["agent"], local["session_id"])
    outbound = [
        j
        for j in jobs()
        if j["direction"] == "to-devbox"
        and j["host"] == args.host
        and (j["session"]["agent"], j["session"]["session_id"]) == sid
        and j["status"] == "complete"
    ]
    if outbound:
        record = dict(outbound[0]["receipt"]["session"])
    else:
        records = [
            r
            for r in call(args.host, "inventory")["sessions"]
            if (r["agent"], r["session_id"]) == sid
        ]
        if args.remote_cwd:
            records = [r for r in records if r.get("cwd") == args.remote_cwd]
        if len(records) != 1:
            raise ValueError(
                "Choose a remote conversation from `clinch sessions transferred --host "
                + args.host
                + "`; use its provider:ID and --remote-cwd if needed"
            )
        record = dict(records[0])
    return record


def prepare(args):
    bound = binding()
    to_remote = args.command == "to-devbox"
    record = current_record(args) if to_remote else returning_record(args)
    source, destination_host = (
        ("local", args.host) if to_remote else (args.host, "local")
    )
    record["include"] = args.include or record.get("include", [])
    record["github_bootstrap"] = True
    record["git_remote"] = args.git_remote
    record.setdefault("origin_pane", bound["pane"])
    plan = call(source, "plan-move", session=record)
    record = plan["session"]
    record.setdefault("origin_cwd", record["cwd"])
    github = plan["project"].get("github")
    project = record.get("project_name") or (
        github["url"].rsplit("/", 1)[-1].removesuffix(".git")
        if github
        else Path(plan["project"]["root"]).name
    )
    project = re.sub(r"[^A-Za-z0-9_.-]", "-", project)[:80] or "project"
    record["project_name"] = project
    record["section"] = "From Mac" if to_remote else "From DevBox"
    identifier = uuid.uuid4().hex
    destination_home = call(destination_host, "home")
    destination = args.dest or str(
        Path(destination_home)
        / ".clinch/transfers"
        / project
        / identifier[:12]
        / project
    )
    app = "orca" if to_remote else "clinch"
    call(
        destination_host,
        "check-destination",
        destination=destination,
        agent=record["agent"],
        app=app,
    )
    job = {
        "id": identifier,
        "created_at": time.time(),
        "status": "waiting",
        "direction": args.command,
        "host": args.host,
        "source": source,
        "destination_host": destination_host,
        "destination": destination,
        "session": record,
        "binding": bound,
        "app": app,
        "project": project,
        "git_bootstrap": github,
        "source_project": plan["project"]["root"],
    }
    if args.dry_run:
        return {
            "status": "preview",
            "job": job,
            "active_pids": record["active_pids"],
            "note": "Applying opens a handoff tab. Exit the source agent; the task waits for its final saved conversation.",
        }
    for previous in jobs():
        if (
            previous["status"] in ("waiting", "transferring")
            and previous["direction"] == args.command
            and previous["host"] == args.host
            and previous["session"]["session_id"] == record["session_id"]
        ):
            raise ValueError(
                "A handoff is already pending: "
                + previous["id"]
                + ". Inspect it with transferred or cancel it first"
            )
    worker.write_new(job_path(identifier), json.dumps(job).encode())
    try:
        tab = create_tab(
            bound,
            os.getcwd(),
            [
                sys.executable,
                str(Path(__file__).with_name("clinch-sessions")),
                "_run-handoff",
                identifier,
            ],
        )
        warning = group_tab(
            bound, tab, ("To DevBox · " if to_remote else "From DevBox · ") + project
        )
        save_job(job, tab=tab, grouping_warning=warning)
    except Exception as error:  # noqa: BLE001 -- retain uncertain tab-creation state
        # Creation may have succeeded before a response was lost. Keep a receipt
        # and require inspection instead of automatically opening another tab.
        return {"status": "launch_uncertain", "job_id": identifier, "error": str(error)}
    return {
        "status": "queued",
        "job_id": identifier,
        "tab": tab,
        "warning": warning,
        "destination": destination,
        "note": "Exit the source Claude/Codex with /exit or Ctrl-D. The visible handoff tab will copy its final saved state. Keep the Mac awake until Complete.",
    }


def run_job(identifier):
    path = job_path(identifier)
    with path.with_suffix(".lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError("This handoff task is already running") from None
        job = json.loads(path.read_text())
        if job["status"] != "waiting":
            raise ValueError(
                "This task cannot be replayed; inspect its recorded status"
            )
        print(
            "Waiting for the source agent to exit. Finish the turn, then use /exit or Ctrl-D.\nKeep this task open and your Mac awake. Ctrl-C cancels before transfer.",
            flush=True,
        )
        deadline, last_hash = time.monotonic() + 1800, None
        try:
            while time.monotonic() < deadline:
                if json.loads(path.read_text())["status"] == "cancelled":
                    print("Cancelled.", flush=True)
                    return 0
                record = call(job["source"], "inspect", session=job["session"])
                destination_active = []
                if job["app"] == "clinch":
                    destination_active = worker.active_owners(
                        record, record.get("origin_cwd", job["source_project"])
                    )
                if not record["active_pids"] and not destination_active:
                    if last_hash == record["transcript_sha256"]:
                        break
                    last_hash = record["transcript_sha256"]
                else:
                    last_hash = None
                time.sleep(2)
            else:
                raise ValueError(
                    "Timed out waiting for the source agent; no transfer started"
                )
            save_job(job, expected="waiting", status="transferring")
            print(
                "Copying the saved conversation and working changes. GitHub history downloads on the destination. Historical absolute attachment paths remain unchanged.",
                flush=True,
            )
            exported = call(job["source"], "export", session=record)
            current = call(job["source"], "plan-move", session=record)
            if (
                current["session"]["active_pids"]
                or current["session"]["transcript_sha256"]
                != exported["manifest"]["session"]["transcript_sha256"]
                or current["project"]["fingerprint"]
                != exported["manifest"]["project"]["fingerprint"]
            ):
                raise ValueError(
                    "Source changed during handoff; destination launch was stopped"
                )
            for key, value in (
                ("CLINCH_CONTROL_WRAPPER", "wrapper"),
                ("CLINCH_CONTROL_PID", "pid"),
                ("WARP_TERMINAL_SESSION_UUID", "pane"),
            ):
                os.environ[key] = job["binding"][value]
            receipt = call(
                job["destination_host"],
                "import",
                bundle=exported["bundle"],
                sha256=exported["sha256"],
                destination=job["destination"],
                app=job["app"],
            )
            save_job(job, status="complete", receipt=receipt)
            print(
                "Complete: "
                + job["project"]
                + " → "
                + job["destination_host"]
                + "\nCheckout: "
                + job["destination"],
                flush=True,
            )
            if job["app"] == "orca":
                print(
                    "On your phone: Orca → your paired DevBox → "
                    + job["project"]
                    + " → [From Mac] "
                    + (record.get("title") or record["session_id"][:8])
                    + ". The Mac can now sleep.",
                    flush=True,
                )
            else:
                print("Continue in the new Clinch agent tab.", flush=True)
            return 0
        except (Exception, KeyboardInterrupt) as error:  # noqa: BLE001 -- persist failure before the visible worker exits
            if json.loads(path.read_text())["status"] == "cancelled":
                print("Cancelled.", flush=True)
                return 0
            save_job(
                job,
                status="cancelled"
                if isinstance(error, KeyboardInterrupt) and job["status"] == "waiting"
                else "failed",
                error=str(error)
                or "Interrupted; inspect destination receipts before retrying",
            )
            print("Handoff stopped: " + job["error"], flush=True)
            return 1


def add_commands(commands):
    for name in ("to-devbox", "from-devbox"):
        parser = commands.add_parser(
            name,
            help="Queue the current conversation's "
            + name
            + " handoff in a Clinch tab",
        )
        parser.add_argument("session", nargs="?", default="current")
        parser.add_argument("--host", default="devbox")
        parser.add_argument("--dest")
        parser.add_argument("--cwd")
        parser.add_argument("--remote-cwd")
        parser.add_argument("--database")
        parser.add_argument("--registry")
        parser.add_argument("--agent-home")
        parser.add_argument(
            "--git-remote",
            help="GitHub remote used to download published history; defaults to clinch or origin",
        )
        parser.add_argument("--include", action="append", default=[])
        parser.add_argument("--dry-run", action="store_true")
        parser.add_argument("--json", action="store_true")
    listing = commands.add_parser(
        "transferred",
        help="List local handoff jobs or imported conversations on a host",
    )
    listing.add_argument("--host")
    listing.add_argument("--json", action="store_true")
    cancel = commands.add_parser(
        "cancel", help="Cancel a handoff that is still waiting for the source agent"
    )
    cancel.add_argument("job")
    cancel.add_argument("--json", action="store_true")
    organize = commands.add_parser(
        "organize",
        help="Group a handoff tab after selecting its original Clinch project",
    )
    organize.add_argument("job")
    organize.add_argument("--json", action="store_true")
    runner = commands.add_parser("_run-handoff", help=__import__("argparse").SUPPRESS)
    runner.add_argument("job")
    runner.add_argument("--json", action="store_true")


def execute(args):
    if args.command in ("to-devbox", "from-devbox"):
        result = prepare(args)
    elif args.command == "_run-handoff":
        return run_job(args.job)
    elif args.command == "cancel":
        job = json.loads(job_path(args.job).read_text())
        if job["status"] != "waiting":
            raise ValueError(
                "Only a waiting task can be cancelled; inspect transfers already in progress"
            )
        save_job(job, expected="waiting", status="cancelled")
        result = {"status": "cancelled", "job_id": args.job}
    elif args.command == "organize":
        job = json.loads(job_path(args.job).read_text())
        tab = job.get("receipt", {}).get("terminal", {}).get("tab") or job.get("tab")
        if not tab:
            raise ValueError("This job has no recorded Clinch tab")
        warning = group_tab(binding(), tab, "Transferred · " + job["project"])
        result = {
            "status": "needs_attention" if warning else "organized",
            "warning": warning,
        }
    elif args.host:
        result = {
            "sessions": call(args.host, "inventory")["sessions"],
            "host": args.host,
        }
    else:
        result = {
            "transfers": [
                {
                    k: j.get(k)
                    for k in (
                        "id",
                        "status",
                        "direction",
                        "host",
                        "project",
                        "destination",
                        "error",
                        "updated_at",
                    )
                }
                | {
                    "session": j["session"]["agent"] + ":" + j["session"]["session_id"],
                    "title": j["session"].get("title"),
                }
                for j in jobs()
            ]
        }
    print(json.dumps(result, indent=2))
    return int(result.get("status") in ("failed", "launch_uncertain"))
