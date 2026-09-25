"""Plan and execute explicit session handoffs, locally or over SSH."""

import argparse
import json
import re
import shlex
import sqlite3
import subprocess
import sys
from pathlib import Path

import session_worker as worker


def call(host, operation, **params):
    request = dict(params, operation=operation)
    if host == "local":
        return worker.dispatch(request)
    if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.@:-]*", host):
        raise ValueError("Use an SSH host alias or user@host for --from/--to.")
    directory = Path(__file__).resolve().parent
    source = (
        (directory / "session_inventory.py").read_bytes()
        + b"\n"
        + (directory / "session_worker.py").read_bytes()
    )
    # Code and data travel on stdin, never as interpolated shell input. The
    # receiver runs in memory and does not require Clinch to be installed.
    bootstrap = "import sys; exec(compile(sys.stdin.buffer.read(int(sys.stdin.buffer.readline())), '<clinch-sessions>', 'exec'))"
    payload = str(len(source)).encode() + b"\n" + source + json.dumps(request).encode()
    result = subprocess.run(
        [
            "ssh",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=15",
            "--",
            host,
            "python3 -c " + shlex.quote(bootstrap),
        ],
        input=payload,
        capture_output=True,
        check=False,
        timeout=900,
    )
    try:
        response = json.loads(result.stdout)
    except ValueError:
        raise ValueError(
            f"No valid session response from {host}. Check SSH access and Python 3.9+."
        ) from None
    if result.returncode or not response.get("ok"):
        raise ValueError(response.get("error", "Remote session operation failed."))
    return response["result"]


def add_source_options(parser):
    parser.add_argument(
        "--from",
        dest="source",
        default="local",
        metavar="HOST",
        help="Source SSH host, or local (default)",
    )
    parser.add_argument("--database", help="Explicit source Clinch warp.sqlite")
    parser.add_argument("--registry", help="Explicit source agent-resume registry")
    parser.add_argument("--agent", choices=("claude", "codex"))
    parser.add_argument("--cwd", help="Source session working directory")
    parser.add_argument("--agent-home", help="Source CODEX_HOME or CLAUDE_CONFIG_DIR")
    parser.add_argument(
        "--transcript", help="Explicit source transcript inside --agent-home"
    )
    parser.add_argument(
        "--json", action="store_true", help="Emit metadata as JSON, without prompt text"
    )


def parser():
    root = argparse.ArgumentParser(
        prog="clinch sessions",
        description="Discover saved Claude/Codex conversations, open them in Orca, or move them between hosts.",
    )
    commands = root.add_subparsers(dest="command", required=True)
    from session_handoff import add_commands

    add_commands(commands)
    listing = commands.add_parser(
        "list", help="List saved Clinch sessions across all projects"
    )
    add_source_options(listing)
    opening = commands.add_parser(
        "open-in", help="Resume stopped sessions in Orca on their current host"
    )
    opening.add_argument("app", choices=("orca", "clinch"))
    opening.add_argument("session", nargs="?", help="Session ID (or agent:ID)")
    opening.add_argument(
        "--all", action="store_true", help="Select all saved sessions across projects"
    )
    opening.add_argument(
        "--dry-run", action="store_true", help="Preview without writing or launching"
    )
    opening.add_argument("--orca-bin", help="Orca CLI executable on the source host")
    add_source_options(opening)
    moving = commands.add_parser(
        "move", help="Copy a stopped conversation and Git checkout, then resume in Orca"
    )
    moving.add_argument("session", help="Session ID (or agent:ID)")
    moving.add_argument("--to", required=True, dest="destination_host", metavar="HOST")
    moving.add_argument(
        "--dest", required=True, help="New absolute checkout root on the destination"
    )
    moving.add_argument("--open-in", choices=("orca",), default="orca")
    moving.add_argument(
        "--dest-agent-home", help="Destination CODEX_HOME or CLAUDE_CONFIG_DIR"
    )
    moving.add_argument(
        "--orca-bin", help="Orca CLI executable on the destination host"
    )
    moving.add_argument(
        "--include",
        action="append",
        default=[],
        metavar="PATH",
        help="Also transfer this ignored file/directory relative to the Git root",
    )
    moving.add_argument(
        "--dry-run",
        action="store_true",
        help="Preview both hosts without writing or launching",
    )
    add_source_options(moving)
    return root


def select(args, data):
    selected = getattr(args, "session", None)
    agent = args.agent
    if selected and ":" in selected:
        prefix, selected = selected.split(":", 1)
        if prefix not in ("claude", "codex") or (agent and agent != prefix):
            raise ValueError("Session provider does not match --agent.")
        agent = prefix
    records = [
        r
        for r in data["sessions"]
        if (not agent or r["agent"] == agent)
        and (not selected or r["session_id"] == selected)
        and (not args.cwd or r.get("cwd") == args.cwd)
    ]
    if selected and not records and agent:
        records = [
            {
                "agent": agent,
                "session_id": selected,
                "cwd": args.cwd,
                "registry": args.registry,
                "title": None,
                "section": None,
            }
        ]
    if args.command != "list" and not getattr(args, "all", False) and len(records) != 1:
        raise ValueError(
            "Select one session ID, disambiguating with --agent and --cwd; or use --all for open-in."
        )
    for record in records:
        if args.agent_home:
            record["agent_home"] = args.agent_home
        if args.transcript:
            record["transcript"] = args.transcript
        if getattr(args, "include", None):
            record["include"] = args.include
    return records


def execute(args):
    if args.command == "open-in" and (bool(args.session) == bool(args.all)):
        raise ValueError("Use either a session ID or --all.")
    data = call(
        args.source, "inventory", database=args.database, registry=args.registry
    )
    records = select(args, data)
    if args.command == "list":
        return dict(data, sessions=records), 0
    if args.command == "open-in":
        results = []
        for record in records:
            result = {
                "agent": record["agent"],
                "session_id": record["session_id"],
                "title": record.get("title"),
                "section": record.get("section"),
            }
            try:
                inspected = call(args.source, "inspect", session=record)
                result.update(
                    cwd=inspected["cwd"], active_pids=inspected["active_pids"]
                )
                if inspected["active_pids"]:
                    result.update(
                        status="blocked",
                        reason="Exit the source agent before opening this session in Orca.",
                    )
                elif args.dry_run:
                    result["status"] = "ready"
                else:
                    result["terminal"] = call(
                        args.source,
                        "open",
                        session=inspected,
                        orca=args.orca_bin,
                        app=args.app,
                    )
                    result["status"] = "opened"
            except (ValueError, OSError, subprocess.SubprocessError) as error:
                result.update(status="error", reason=str(error))
            results.append(result)
        report = {
            "dry_run": args.dry_run,
            "sessions": results,
            "skipped": data["skipped"],
            "sections": data["sections"],
            "note": "Section names become tab-title prefixes; live terminals and layouts are not moved.",
        }
        return report, int(any(r["status"] in ("error", "blocked") for r in results))
    record = records[0]
    plan = call(args.source, "plan-move", session=record)
    destination = call(
        args.destination_host,
        "check-destination",
        destination=args.dest,
        agent=record["agent"],
        orca=args.orca_bin,
    )
    report = {
        "dry_run": args.dry_run,
        "source": args.source,
        "destination_host": args.destination_host,
        **destination,
        **plan,
        "note": "Source files are retained. Ignored files require --include; credentials and machine settings are not copied.",
        "warning": "Historical absolute paths are unchanged. Copied attachments may need reopening under the destination paths recorded in the receipt.",
    }
    if plan["session"]["active_pids"]:
        report.update(
            status="blocked", reason="Exit the source agent before moving this session."
        )
        return report, 1
    if args.dry_run:
        report["status"] = "ready"
        return report, 0
    print(report["warning"], file=sys.stderr)
    exported = call(args.source, "export", session=plan["session"])
    # Recheck the stopped source before starting any destination process.
    current = call(args.source, "plan-move", session=plan["session"])
    if (
        current["session"]["active_pids"]
        or current["session"]["transcript_sha256"]
        != exported["manifest"]["session"]["transcript_sha256"]
        or current["project"]["fingerprint"]
        != exported["manifest"]["project"]["fingerprint"]
    ):
        raise ValueError(
            "Source changed after export; no destination session was launched."
        )
    receipt = call(
        args.destination_host,
        "import",
        bundle=exported["bundle"],
        sha256=exported["sha256"],
        destination=destination["destination"],
        agent_home=args.dest_agent_home,
        orca=args.orca_bin,
    )
    return {
        "status": "launched",
        "destination_host": args.destination_host,
        "destination": destination["destination"],
        "receipt": receipt,
        "note": report["note"],
    }, 0


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        if args.command in (
            "to-devbox",
            "from-devbox",
            "transferred",
            "cancel",
            "organize",
            "_run-handoff",
        ):
            from session_handoff import execute as handoff

            return handoff(args)
        report, code = execute(args)
    except (
        ValueError,
        OSError,
        subprocess.SubprocessError,
        KeyError,
        sqlite3.Error,
    ) as error:
        report, code = {"status": "error", "error": str(error)}, 1
    if args.json:
        print(json.dumps(report, indent=2))
    elif "error" in report:
        print("Error: " + report["error"], file=sys.stderr)
    elif "sessions" in report:
        for row in report["sessions"]:
            print(
                "{} {}:{}  {}  {}".format(
                    row.get("status", "saved"),
                    row["agent"],
                    row["session_id"],
                    row.get("cwd", ""),
                    row.get("reason") or row.get("title") or "",
                )
            )
        print(
            "{} session(s); {} pane(s) without a resume ID.".format(
                len(report["sessions"]), len(report.get("skipped", []))
            )
        )
    else:
        print(
            "{}: {} -> {}:{}".format(
                report["status"],
                getattr(args, "session", ""),
                args.destination_host,
                report["destination"],
            )
        )
        if report.get("reason"):
            print(report["reason"])
        if "project" in report:
            print(
                "{} artifacts, {} untracked files; ignored files present: {}".format(
                    report["artifact_count"],
                    report["untracked_count"],
                    report["project"]["ignored_files_present"],
                )
            )
        print(report["note"])
        if report.get("warning"):
            print(report["warning"])
    return code
