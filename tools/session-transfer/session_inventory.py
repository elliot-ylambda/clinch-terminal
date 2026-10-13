"""Read saved panes across every Clinch project without reading prompt text."""

import json
import os
import re
import shlex
import sqlite3
from pathlib import Path


def parse_restore(command):
    try:
        words = shlex.split(command or "")
    except ValueError:
        return None
    if (
        len(words) < 3
        or words[0] not in ("clinch_agent_resume_launch", "warp_agent_resume_launch")
        or words[1] not in ("claude", "codex")
        or not re.fullmatch(r"[A-Za-z0-9-]+", words[2])
    ):
        return None
    return {"agent": words[1], "session_id": words[2]}


def database_path(explicit=None):
    if explicit:
        return Path(explicit).expanduser().resolve()
    base = Path.home() / "Library/Application Support"
    candidates = [
        base / app / "warp.sqlite"
        for app in ("sh.clinch.Clinch", "sh.clinch.ClinchDev")
    ]
    candidates = [p for p in candidates if p.is_file()]
    if len(candidates) > 1:
        raise ValueError("Several Clinch databases exist; select one with --database.")
    return candidates[0] if candidates else None


def registry_path(database=None, explicit=None):
    if explicit or os.environ.get("WARP_AGENT_RESUME_DIR"):
        return Path(explicit or os.environ["WARP_AGENT_RESUME_DIR"]).expanduser()
    config = ".clinch-local" if database and "ClinchDev" in str(database) else ".warp"
    return Path.home() / config / "agent-resume"


def read_registry(directory):
    entries = {}
    if directory.is_dir():
        for path in directory.glob("*.json"):
            if not re.fullmatch(r"[0-9a-fA-F-]+", path.stem):
                continue
            try:
                item = json.loads(path.read_text())
                if isinstance(item, dict):
                    entries[path.stem.replace("-", "").lower()] = item
            except (OSError, ValueError):
                continue
    return entries


def inventory(database=None, registry=None):
    database = database_path(database)
    registry = registry_path(database, registry)
    live = read_registry(registry)
    sessions, skipped, sections = [], [], []
    if database:
        # mode=ro retains the current WAL; immutable=1 would miss live saves.
        with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as db:
            db.row_factory = sqlite3.Row
            db.execute("BEGIN")
            sections = [
                dict(row)
                for row in db.execute("SELECT * FROM tab_groups ORDER BY window_id,id")
            ]
            rows = db.execute("""
                SELECT w.id AS project_id, w.project_index, t.id AS tab_id,
                       t.custom_title AS tab_title, t.pinned, t.color,
                       g.name AS section, g.id AS section_id,
                       n.id AS pane_node_id, n.parent_pane_node_id, n.flex,
                       l.custom_vertical_tabs_title AS pane_title,
                       p.uuid, p.cwd, p.on_restore_command
                FROM windows w JOIN tabs t ON t.window_id=w.id
                JOIN pane_nodes n ON n.tab_id=t.id
                JOIN pane_leaves l ON l.pane_node_id=n.id
                JOIN terminal_panes p ON p.id=n.id
                LEFT JOIN tab_groups g ON g.id=t.tab_group_id AND g.window_id=w.id
                ORDER BY w.project_index,w.id,t.id,n.id
            """)
            for row in rows:
                row = dict(row)
                pane = bytes(row.pop("uuid")).hex()
                stored = row.pop("on_restore_command")
                entry = live.get(pane, {})
                # A present registry record is authoritative, even if its command
                # was cleared; don't resurrect the stale persisted conversation.
                restore = parse_restore(entry.get("command") if entry else stored)
                title = row.pop("pane_title") or row.pop("tab_title", None)
                row.pop("tab_title", None)
                item = dict(
                    row, pane_id=pane, title=title, cwd=entry.get("cwd") or row["cwd"]
                )
                if restore:
                    item.update(restore, registry=str(registry))
                    sessions.append(item)
                else:
                    skipped.append(
                        dict(item, reason="No recorded Claude/Codex resume ID")
                    )
    else:
        for pane, entry in sorted(live.items()):
            restore = parse_restore(entry.get("command"))
            if restore:
                sessions.append(
                    dict(
                        restore,
                        pane_id=pane,
                        cwd=entry.get("cwd"),
                        title=None,
                        section=None,
                        registry=str(registry),
                    )
                )
    # Imported sessions remain discoverable on a headless host without Clinch.
    receipt_dir = Path.home() / ".clinch/session-transfer/receipts"
    for path in sorted(receipt_dir.glob("*.json")):
        try:
            receipt = json.loads(path.read_text())
            item = receipt["session"]
            if receipt["status"] in (
                "imported",
                "launching",
                "launched",
                "launch_failed",
            ):
                sessions.append(item)
        except (OSError, ValueError, KeyError, TypeError):
            continue
    unique = {}
    for item in sessions:
        key = (item["agent"], item["session_id"], item.get("cwd"))
        if key in unique:
            unique[key].setdefault("other_panes", []).append(item.get("pane_id"))
            # Receipts carry lineage that a newly saved Clinch pane does not.
            for field in (
                "ancestor_hashes",
                "origin_pane",
                "origin_cwd",
                "project_name",
                "checkout_root",
                "agent_home",
                "transcript",
            ):
                if field in item:
                    unique[key][field] = item[field]
        else:
            unique[key] = item
    return {
        "schema_version": 1,
        "sessions": list(unique.values()),
        "skipped": skipped,
        "sections": sections,
        "database": str(database) if database else None,
    }
