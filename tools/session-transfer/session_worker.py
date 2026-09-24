"""Local/SSH worker. Only explicit apply operations write files or launch agents."""

import base64
import contextlib
import fcntl
import hashlib
import io
import json
import os
import re
import selectors
import shlex
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path, PurePosixPath

try:
    from session_inventory import inventory, read_registry
except ImportError:
    # The SSH bootstrap prepends session_inventory.py to this module.
    if "inventory" not in globals():
        raise

MAX_BYTES = 128 * 1024 * 1024
MAX_FILES = 10000


def run(argv, cwd=None, env=None, timeout=60):
    env = dict(os.environ if env is None else env, GIT_OPTIONAL_LOCKS="0")
    result = subprocess.run(
        argv,
        cwd=cwd,
        env=env,
        capture_output=True,
        check=False,
        timeout=timeout,
    )
    if result.returncode:
        # Commands may include repository credentials; never echo argv or stderr.
        raise ValueError(f"{Path(argv[0]).name} failed (exit {result.returncode}).")
    return result.stdout


def digest(data):
    return hashlib.sha256(data).hexdigest()


def state_dir():
    return Path.home() / ".clinch/session-transfer"


def regular_bytes(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Expected a regular file: {path}")
    before = path.stat()
    if before.st_size > MAX_BYTES:
        raise ValueError(f"File exceeds the 128 MiB transfer limit: {path}")
    data = path.read_bytes()
    after = path.stat()
    if (before.st_ino, before.st_size, before.st_mtime_ns) != (
        after.st_ino,
        after.st_size,
        after.st_mtime_ns,
    ):
        raise ValueError(f"File changed while reading: {path}")
    return data


def safe_relative(value):
    path = PurePosixPath(value)
    if (
        not value
        or path.is_absolute()
        or ".." in path.parts
        or "\\" in value
        or "\x00" in value
        or str(path) != value
    ):
        raise ValueError("Invalid relative bundle path")
    return path


def beneath(root, relative):
    relative = safe_relative(relative)
    target = root.joinpath(*relative.parts)
    parent = target
    while parent != root:
        if parent.is_symlink():
            raise ValueError(f"Refusing symlink in destination: {parent}")
        parent = parent.parent
    if root.is_symlink():
        raise ValueError("Refusing symlink destination root")
    return target


def walk_files(root):
    count = 0
    if root.is_symlink():
        raise ValueError(f"Symlink at artifact root: {root}")
    if not root.exists():
        return
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in dirs:
            if (Path(directory) / name).is_symlink():
                raise ValueError(f"Symlink in session artifacts: {name}")
        for name in sorted(files):
            count += 1
            if count > MAX_FILES:
                raise ValueError(
                    "More than 10,000 session artifacts; narrow the selection."
                )
            yield Path(directory) / name


def provider_home(record):
    agent = record["agent"]
    override = record.get("agent_home")
    default = os.environ.get("CODEX_HOME" if agent == "codex" else "CLAUDE_CONFIG_DIR")
    return (
        Path(override or default or (Path.home() / ("." + agent)))
        .expanduser()
        .resolve()
    )


def find_transcript(record):
    agent, sid = record["agent"], record["session_id"]
    if agent not in ("claude", "codex") or not re.fullmatch(r"[A-Za-z0-9-]+", sid):
        raise ValueError("Invalid agent or session ID")
    home = provider_home(record)
    explicit = record.get("transcript")
    roots = (
        [home / "projects"]
        if agent == "claude"
        else [home / "sessions", home / "archived_sessions"]
    )
    if explicit:
        candidates = [Path(explicit).expanduser().resolve()]
    else:
        candidates = []
        for root in roots:
            for directory, dirs, names in os.walk(root):
                dirs[:] = [
                    d
                    for d in dirs
                    if not (Path(directory) / d).is_symlink()
                    and d not in ("subagents", "tool-results")
                ]
                for name in names:
                    if name == sid + ".jsonl" or name.endswith("-" + sid + ".jsonl"):
                        candidates.append(Path(directory) / name)
    if len(candidates) != 1:
        raise ValueError(
            f"Expected one transcript for {sid}; found {len(candidates)}. Use --transcript/--agent-home."
        )
    path = candidates[0]
    if not any(root in path.parents for root in roots):
        raise ValueError(
            "Transcript must be inside the selected provider's session store."
        )
    path = beneath(home, str(path.relative_to(home)))
    rows = []
    for line in regular_bytes(path).splitlines():
        if line.strip():
            try:
                rows.append(json.loads(line))
            except ValueError:
                raise ValueError(
                    "Transcript has an incomplete or invalid JSON record."
                ) from None
    if agent == "codex":
        meta = next(
            (r.get("payload", {}) for r in rows if r.get("type") == "session_meta"), {}
        )
        if meta.get("id") != sid:
            raise ValueError("Codex transcript ID does not match the selected session.")
        # Never pretend that a metadata-only paginated rollout is full history.
        if not any(r.get("type") == "response_item" for r in rows):
            raise ValueError(
                "Codex has no portable rollout history; this storage format is unsupported."
            )
        cwd = meta.get("cwd")
    else:
        if not any(
            r.get("sessionId") == sid and r.get("type") in ("user", "assistant")
            for r in rows
        ):
            raise ValueError(
                "Claude transcript has no conversation for the selected ID."
            )
        cwd = next((r.get("cwd") for r in reversed(rows) if r.get("cwd")), None)
    return path, rows, record.get("cwd") or cwd


def active_owners(record, cwd):
    owners = set()
    registry = Path(
        record.get("registry")
        or os.environ.get("WARP_AGENT_RESUME_DIR", Path.home() / ".warp/agent-resume")
    )
    for item in read_registry(registry).values():
        if record["session_id"] not in str(item.get("command", "")):
            continue
        pid = item.get("owner_pid")
        if str(pid).isdigit():
            result = subprocess.run(
                ["ps", "-p", str(pid), "-o", "comm="],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            if Path(result.stdout.decode().strip()).name == record["agent"]:
                owners.add(int(pid))
    result = run(["ps", "-axo", "pid=,comm=,args="])
    for line in result.decode(errors="replace").splitlines():
        fields = line.split(None, 2)
        if len(fields) == 3 and Path(fields[1]).name == record["agent"]:
            if record["session_id"] in fields[2]:
                owners.add(int(fields[0]))
            if sys.platform.startswith("linux"):
                try:
                    if Path(f"/proc/{fields[0]}/cwd").resolve() == Path(cwd).resolve():
                        owners.add(int(fields[0]))
                except OSError:
                    pass
    if sys.platform == "darwin" and shutil.which("lsof"):
        result = subprocess.run(
            ["lsof", "-a", "-c", record["agent"], "-d", "cwd", "-Fpn"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        pid = None
        for line in result.stdout.decode(errors="replace").splitlines():
            if line.startswith("p") and line[1:].isdigit():
                pid = int(line[1:])
            elif (
                line.startswith("n")
                and pid
                and Path(line[1:]).resolve() == Path(cwd).resolve()
            ):
                owners.add(pid)
    return sorted(owners)


def inspect_session(record):
    path, _, cwd = find_transcript(record)
    if not cwd or not Path(cwd).is_dir():
        raise ValueError("Session working directory is missing; supply --cwd.")
    record = dict(
        record,
        cwd=str(Path(cwd).resolve()),
        transcript=str(path),
        agent_home=str(provider_home(record)),
    )
    return dict(
        record,
        active_pids=active_owners(record, cwd),
        transcript_sha256=digest(regular_bytes(path)),
    )


def require_stopped(record):
    record = inspect_session(record)
    if record["active_pids"]:
        raise ValueError(
            "Source agent is running (PID {}). Exit it before transferring.".format(
                ", ".join(map(str, record["active_pids"]))
            )
        )
    return record


def orca_command(explicit=None):
    candidates = (
        [explicit]
        if explicit
        else [
            shutil.which("orca"),
            shutil.which("orca-ide"),
            "/Applications/Orca.app/Contents/Resources/bin/orca",
        ]
    )
    for candidate in candidates:
        if candidate and os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
    raise ValueError("Orca CLI not found on this host; use --orca-bin.")


def agent_env(record):
    env = {
        k: v
        for k, v in os.environ.items()
        if not k.startswith(("ORCA_", "WARP_", "CLINCH_"))
    }
    env["CODEX_HOME" if record["agent"] == "codex" else "CLAUDE_CONFIG_DIR"] = record[
        "agent_home"
    ]
    return env


def verify_codex(record):
    """Use Codex's own lazy index repair; never write its SQLite schema."""
    with subprocess.Popen(
        ["codex", "app-server"],
        cwd=record["cwd"],
        env=agent_env(record),
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    ) as process:
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        buffer = b""

        def send(value):
            process.stdin.write(json.dumps(value).encode() + b"\n")
            process.stdin.flush()

        def response(request_id):
            nonlocal buffer
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                while b"\n" in buffer:
                    line, buffer = buffer.split(b"\n", 1)
                    value = json.loads(line)
                    if value.get("id") == request_id:
                        if "error" in value:
                            raise ValueError(
                                "Codex could not read the imported session."
                            )
                        return value.get("result", {})
                if selector.select(max(0, deadline - time.monotonic())):
                    chunk = os.read(process.stdout.fileno(), 65536)
                    if not chunk:
                        break
                    buffer += chunk
                    if len(buffer) > MAX_BYTES:
                        break
            raise ValueError("Codex session verification timed out or exited.")

        try:
            send(
                {
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "clientInfo": {"name": "clinch_sessions", "version": "1"}
                    },
                }
            )
            response(1)
            send({"method": "initialized"})
            send(
                {
                    "id": 2,
                    "method": "thread/read",
                    "params": {"threadId": record["session_id"], "includeTurns": False},
                }
            )
            result = response(2)
            if result.get("thread", {}).get("id") != record["session_id"]:
                raise ValueError("Codex returned a different session ID.")
        finally:
            selector.close()
            process.stdin.close()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()


def launch(record, orca=None):
    binary = orca_command(orca)
    provider = shutil.which(record["agent"])
    if not provider:
        raise ValueError("{} CLI not found on destination.".format(record["agent"]))
    if record["agent"] == "codex":
        verify_codex(record)
        args = [
            "env",
            "CODEX_HOME=" + record["agent_home"],
            provider,
            "resume",
            record["session_id"],
            "-C",
            record["cwd"],
            "-c",
            'tui.resume_cwd="current"',
        ]
    else:
        args = [
            "env",
            "CLAUDE_CONFIG_DIR=" + record["agent_home"],
            provider,
            "--resume",
            record["transcript"],
        ]
    env = dict(os.environ)
    for key in ("ORCA_PAIRING_CODE", "ORCA_ENVIRONMENT"):
        env.pop(key, None)
    run([binary, "repo", "add", "--path", record["cwd"], "--json"], env=env)
    title = record.get("title") or "{} {}".format(
        record["agent"], record["session_id"][:8]
    )
    if record.get("section"):
        title = "[{}] {}".format(record["section"], title)
    raw = run(
        [
            binary,
            "terminal",
            "create",
            "--worktree",
            "path:" + record["cwd"],
            "--title",
            title,
            "--command",
            shlex.join(args),
            "--json",
        ],
        env=env,
    )
    return json.loads(raw)


def project_snapshot(cwd, includes=()):
    root = Path(
        run(["git", "rev-parse", "--show-toplevel"], cwd).decode().strip()
    ).resolve()
    head = run(["git", "rev-parse", "HEAD"], root).decode().strip()
    branch = run(["git", "rev-parse", "--abbrev-ref", "HEAD"], root).decode().strip()
    files = run(["git", "ls-files", "--stage", "-z"], root)
    if any(item.startswith(b"160000 ") for item in files.split(b"\0")):
        raise ValueError(
            "Submodules need a separate checkout transfer; move is not supported here."
        )
    attributes = run(
        ["git", "ls-files", "-z", "--", ".gitattributes", ":(glob)**/.gitattributes"],
        root,
    )
    for name in attributes.split(b"\0"):
        if name and b"filter=lfs" in regular_bytes(beneath(root, os.fsdecode(name))):
            raise ValueError("Git LFS repositories need a separate checkout transfer.")
    index = run(
        [
            "git",
            "diff",
            "--cached",
            "--binary",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
        ],
        root,
    )
    worktree = run(["git", "diff", "--binary", "--no-ext-diff", "--no-textconv"], root)
    names = run(["git", "ls-files", "--others", "--exclude-standard", "-z"], root)
    if len(names.split(b"\0")) > MAX_FILES:
        raise ValueError(
            "More than 10,000 working files; narrow the checkout before moving."
        )
    untracked = {}
    for name in names.split(b"\0"):
        if name:
            relative = os.fsdecode(name)
            target = beneath(root, relative)
            untracked[relative] = (
                regular_bytes(target),
                stat.S_IMODE(target.stat().st_mode),
            )
            if sum(len(data) for data, _ in untracked.values()) > MAX_BYTES:
                raise ValueError("Working files exceed the 128 MiB transfer limit.")
    for relative in includes:
        target = beneath(root, relative)
        if any(part.lower() == ".git" for part in PurePosixPath(relative).parts):
            raise ValueError("--include cannot copy Git internal files.")
        included = list(walk_files(target)) if target.is_dir() else [target]
        for path in included:
            name = str(path.relative_to(root))
            if not run(["git", "ls-files", "--", name], root):
                untracked[name] = (
                    regular_bytes(path),
                    stat.S_IMODE(path.stat().st_mode),
                )
    ignored = bool(
        run(
            [
                "git",
                "ls-files",
                "--directory",
                "--no-empty-directory",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z",
            ],
            root,
        )
    )
    if sum(len(data) for data, _ in untracked.values()) > MAX_BYTES:
        raise ValueError("Working files exceed the 128 MiB transfer limit.")
    fingerprint = digest(
        head.encode()
        + branch.encode()
        + index
        + worktree
        + b"".join(
            name.encode() + digest(data).encode() + str(mode).encode()
            for name, (data, mode) in sorted(untracked.items())
        )
    )
    return {
        "root": str(root),
        "head": head,
        "branch": branch,
        "cwd_relative": str(Path(cwd).resolve().relative_to(root)),
        "index": index,
        "worktree": worktree,
        "untracked": untracked,
        "ignored_files_present": ignored,
        "fingerprint": fingerprint,
    }


def session_files(record, visited=None):
    visited = set() if visited is None else visited
    if record["session_id"] in visited:
        raise ValueError("Cycle in Codex subagent history")
    visited.add(record["session_id"])
    path, rows, _ = find_transcript(record)
    home = Path(record["agent_home"])
    files = {str(path.relative_to(home)): regular_bytes(path)}
    if record["agent"] == "claude":
        roots = [path.with_suffix("")] + [
            home / name / record["session_id"]
            for name in ("file-history", "image-cache", "uploads")
        ]
        for root in roots:
            beneath(home, str(root.relative_to(home)))
            for child in walk_files(root):
                relative = str(child.relative_to(home))
                files[relative] = regular_bytes(beneath(home, relative))
        # Plans and paste-cache are shared stores: include only referenced files.
        text = json.dumps(rows)
        for folder in ("plans", "paste-cache"):
            for child in walk_files(beneath(home, folder)):
                if (
                    str(child) in text
                    or ("~/.claude/" + str(child.relative_to(home))) in text
                ):
                    relative = str(child.relative_to(home))
                    files[relative] = regular_bytes(beneath(home, relative))
    else:
        children = set()
        for row in rows:
            payload = row.get("payload", {})
            if payload.get("type") == "collab_agent_spawn_end":
                child = payload.get("new_thread_id") or payload.get(
                    "receiver_thread_id"
                )
                if not child:
                    raise ValueError(
                        "Unrecognized Codex subagent history; cannot transfer completely."
                    )
                children.add(child)
        if children:
            # Child sessions can themselves own running work and more children.
            for child in sorted(children):
                child_record = require_stopped(
                    dict(record, session_id=child, transcript=None)
                )
                files.update(session_files(child_record, visited))
    if sum(map(len, files.values())) > MAX_BYTES or len(files) > MAX_FILES:
        raise ValueError("Session artifacts exceed the transfer size limit.")
    return files


def export_session(record):
    record = require_stopped(record)
    project = project_snapshot(record["cwd"], record.get("include", []))
    artifacts = session_files(record)
    payloads = {"session/" + name: data for name, data in artifacts.items()}
    payloads.update(
        {"index.patch": project["index"], "worktree.patch": project["worktree"]}
    )
    for name, (data, _) in project["untracked"].items():
        payloads["files/" + name] = data
    with tempfile.TemporaryDirectory(prefix="clinch-session-export-") as temp:
        bundle = Path(temp) / "repo.bundle"
        run(["git", "bundle", "create", str(bundle), "HEAD"], project["root"])
        payloads["repo.bundle"] = regular_bytes(bundle)
    if sum(map(len, payloads.values())) > MAX_BYTES:
        raise ValueError(
            "Transfer exceeds 128 MiB; use an existing remote runtime for this repository."
        )
    if (
        project_snapshot(record["cwd"], record.get("include", []))["fingerprint"]
        != project["fingerprint"]
    ):
        raise ValueError("Project changed during export; retry after stopping writers.")
    if session_files(require_stopped(record)) != artifacts:
        raise ValueError(
            "Session changed during export; retry after exiting the source agent."
        )
    manifest = {
        "schema_version": 1,
        "session": record,
        "project": {
            k: v
            for k, v in project.items()
            if k not in ("index", "worktree", "untracked")
        },
        "modes": {
            "files/" + name: mode for name, (_, mode) in project["untracked"].items()
        },
        "files": {name: digest(data) for name, data in payloads.items()},
    }
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("manifest.json", json.dumps(manifest))
        for name, data in payloads.items():
            archive.writestr(name, data)
    return {
        "bundle": base64.b64encode(stream.getvalue()).decode(),
        "sha256": digest(stream.getvalue()),
        "manifest": manifest,
    }


def unpack_bundle(encoded, expected):
    raw = base64.b64decode(encoded, validate=True)
    if digest(raw) != expected or len(raw) > MAX_BYTES:
        raise ValueError("Transfer checksum or size mismatch")
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        infos = archive.infolist()
        names = [item.filename for item in infos]
        if (
            len(names) != len(set(names))
            or len(names) > MAX_FILES
            or sum(item.file_size for item in infos) > MAX_BYTES
        ):
            raise ValueError("Duplicate entries or oversized transfer")
        for name in names:
            safe_relative(name)
        manifest = json.loads(archive.read("manifest.json"))
        if manifest.get("schema_version") != 1:
            raise ValueError("Unsupported transfer manifest version")
        if set(names) != {"manifest.json"} | set(manifest["files"]):
            raise ValueError("Transfer file inventory mismatch")
        payloads = {name: archive.read(name) for name in manifest["files"]}
        for name, data in payloads.items():
            if digest(data) != manifest["files"][name]:
                raise ValueError(f"Corrupt transfer file: {name}")
    return manifest, payloads


def write_new(path, data, mode=0o600):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with path.open("xb") as output:
        os.chmod(path, mode)
        output.write(data)


def write_receipt(path, receipt):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as output:
        temp = Path(output.name)
        output.write(json.dumps(receipt, indent=2).encode())
    temp.replace(path)


@contextlib.contextmanager
def session_lock(record):
    agent, sid = record["agent"], record["session_id"]
    if agent not in ("claude", "codex") or not re.fullmatch(r"[A-Za-z0-9-]+", sid):
        raise ValueError("Invalid session lock identity")
    path = state_dir() / "locks" / (agent + "-" + sid + ".lock")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with path.open("a") as lock:
        os.chmod(path, 0o600)
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError(
                "Another handoff of this session is in progress."
            ) from None
        try:
            lease = state_dir() / "launches" / (agent + "-" + sid + ".json")
            if lease.exists():
                last = json.loads(lease.read_text())
                if last["status"] == "launching" or time.time() - last["time"] < 30:
                    raise ValueError(
                        f"A launch of this session is pending or recent; inspect {lease} before retrying."
                    )
            yield lease
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


def launch_once(record, orca, lease):
    write_receipt(
        lease, {"status": "launching", "time": time.time(), "session": record}
    )
    result = launch(record, orca)
    write_receipt(lease, {"status": "launched", "time": time.time(), "session": record})
    return result


def destination_check(destination, orca=None, agent=None):
    path = Path(destination).expanduser()
    if not path.is_absolute() or path.exists() or path.is_symlink():
        raise ValueError(
            "--dest must be a new absolute checkout path; existing paths are never overwritten."
        )
    # Resolve only the parent; a final symlink must never disguise a collision.
    path = path.parent.resolve() / path.name
    orca_command(orca)
    if agent and not shutil.which(agent):
        raise ValueError(f"{agent} CLI is missing on the destination.")
    if not shutil.which("git"):
        raise ValueError("Git is missing on the destination.")
    return str(path)


def import_session(request):
    manifest, payloads = unpack_bundle(request["bundle"], request["sha256"])
    with session_lock(manifest["session"]) as lease:
        return import_locked(request, manifest, payloads, lease)


def import_locked(request, manifest, payloads, lease):
    destination = Path(
        destination_check(
            request["destination"], request.get("orca"), manifest["session"]["agent"]
        )
    )
    source = manifest["session"]
    if source.get("agent") not in ("claude", "codex") or not re.fullmatch(
        r"[A-Za-z0-9-]+", source.get("session_id", "")
    ):
        raise ValueError("Invalid provider or session ID in transfer")
    home = (
        Path(request.get("agent_home") or provider_home({"agent": source["agent"]}))
        .expanduser()
        .resolve()
    )
    main_relative = str(
        Path(source["transcript"]).relative_to(Path(source["agent_home"]))
    )
    targets = {}
    ancestors = {
        name: list(hashes) for name, hashes in source.get("ancestor_hashes", {}).items()
    }
    previous = {}
    for name, data in payloads.items():
        if name.startswith("files/") and any(
            part.lower() == ".git" for part in PurePosixPath(name).parts
        ):
            raise ValueError("Transfer contains Git internal files")
        if name.startswith("session/"):
            relative = name[len("session/") :]
            allowed = (
                (
                    "projects",
                    "file-history",
                    "image-cache",
                    "uploads",
                    "plans",
                    "paste-cache",
                )
                if source["agent"] == "claude"
                else ("sessions", "archived_sessions")
            )
            if safe_relative(relative).parts[0] not in allowed:
                raise ValueError("Bundle includes a non-session provider file")
            target = beneath(home, relative)
            if target.exists():
                existing = regular_bytes(target)
                if existing != data:
                    if digest(existing) not in ancestors.get(relative, []) or (
                        target.suffix == ".jsonl" and not data.startswith(existing)
                    ):
                        raise ValueError(
                            "Destination has different session history; refusing divergent history."
                        )
                    if target.stat().st_nlink > 1:
                        raise ValueError(
                            "History has shared hardlinks; select a separate --dest-agent-home."
                        )
                    previous[target] = existing
            ancestors.setdefault(relative, [])
            if digest(data) not in ancestors[relative]:
                ancestors[relative].append(digest(data))
            targets[target] = data
    if home / main_relative not in targets:
        raise ValueError("Transfer is missing the selected transcript.")
    source_cwd = Path(source["cwd"])
    cwd_relative = manifest["project"]["cwd_relative"]
    if cwd_relative != ".":
        safe_relative(cwd_relative)
    record = dict(
        source,
        cwd=str(destination / cwd_relative),
        agent_home=str(home),
        transcript=str(home / main_relative),
        registry=None,
        ancestor_hashes=ancestors,
    )
    record.pop("active_pids", None)
    # Refuse to install onto a destination that already runs this conversation.
    if active_owners(record, record["cwd"]) or active_owners(record, str(source_cwd)):
        raise ValueError("This session is already running on the destination.")
    if (home / main_relative).exists():
        existing_record = dict(record, cwd=None)
        _, _, old_cwd = find_transcript(existing_record)
        if old_cwd and active_owners(existing_record, old_cwd):
            raise ValueError(
                "The destination's previous conversation is still running."
            )
    receipt_id = digest(
        (source["agent"] + source["session_id"] + str(destination)).encode()
    )[:24]
    receipt_path = state_dir() / "receipts" / (receipt_id + ".json")
    if receipt_path.exists():
        raise ValueError(
            f"A transfer receipt already exists; inspect it before retrying: {receipt_path}"
        )
    destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    receipt = {
        "schema_version": 1,
        "status": "preparing",
        "session": record,
        "source_cwd": str(source_cwd),
        "bundle_sha256": request["sha256"],
        "receipt": str(receipt_path),
        "path_mappings": {
            str(source_cwd): record["cwd"],
            source["agent_home"]: str(home),
        },
        "warning": "Historical absolute paths are unchanged; copied attachments may need reopening at their destination paths.",
    }
    write_new(receipt_path, json.dumps(receipt).encode())
    created_artifacts = []
    stage = Path(tempfile.mkdtemp(prefix=".clinch-import-", dir=destination.parent))
    installed = False
    try:
        bundle_path = stage / "repo.bundle"
        write_new(bundle_path, payloads["repo.bundle"])
        checkout = stage / "checkout"
        git_env = dict(
            os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull
        )
        run(
            [
                "git",
                "-c",
                "core.hooksPath=/dev/null",
                "clone",
                "--no-checkout",
                str(bundle_path),
                str(checkout),
            ],
            env=git_env,
        )
        run(
            [
                "git",
                "-c",
                "core.hooksPath=/dev/null",
                "checkout",
                "--detach",
                manifest["project"]["head"],
            ],
            checkout,
            env=git_env,
        )
        branch = manifest["project"]["branch"]
        if branch != "HEAD":
            run(["git", "check-ref-format", "--branch", branch], checkout, env=git_env)
            run(
                ["git", "-c", "core.hooksPath=/dev/null", "checkout", "-B", branch],
                checkout,
                env=git_env,
            )
        run(["git", "remote", "remove", "origin"], checkout, env=git_env)
        for name, indexed in (("index.patch", True), ("worktree.patch", False)):
            if payloads[name]:
                patch = stage / name
                write_new(patch, payloads[name])
                run(
                    ["git", "apply"] + (["--index"] if indexed else []) + [str(patch)],
                    checkout,
                    env=git_env,
                )
        for name, data in payloads.items():
            if name.startswith("files/"):
                mode = manifest["modes"].get(name, 0o600)
                write_new(beneath(checkout, name[6:]), data, mode & 0o777)
        if not (checkout / cwd_relative).is_dir():
            raise ValueError("Session directory was not restored in the checkout.")
        # Reserve the destination without overwriting a directory created concurrently.
        destination.mkdir(mode=0o700)
        try:
            for child in checkout.iterdir():
                child.rename(destination / child.name)
        except Exception:
            shutil.rmtree(destination)
            raise
        installed = True
        for target, data in targets.items():
            if target in previous:
                if regular_bytes(target) != previous[target]:
                    raise ValueError("Destination history changed during import.")
                backup = state_dir() / "backups" / receipt_id / target.relative_to(home)
                write_new(backup, previous[target])
                with tempfile.NamedTemporaryFile(
                    dir=target.parent, delete=False
                ) as output:
                    temporary = Path(output.name)
                    output.write(data)
                temporary.replace(target)
            elif target.exists():
                if regular_bytes(target) != data:
                    raise ValueError("Destination artifact changed during import.")
            else:
                write_new(target, data)
                created_artifacts.append(target)
        receipt["status"] = "imported"
        write_receipt(receipt_path, receipt)
        # The receipt precedes launch, so an uncertain response cannot invite an
        # automatic second agent process on retry.
        receipt["status"] = "launching"
        write_receipt(receipt_path, receipt)
        if active_owners(record, record["cwd"]):
            raise ValueError(
                "Session started during import; destination launch was stopped."
            )
        receipt["terminal"] = launch_once(record, request.get("orca"), lease)
        receipt["status"] = "launched"
        write_receipt(receipt_path, receipt)
        return receipt
    except Exception as error:
        if installed:
            receipt["status"] = "launch_failed"
            receipt["error"] = str(error)
            write_receipt(receipt_path, receipt)
            raise ValueError(
                f"Import retained at {destination}; inspect {receipt_path} before retrying. {error}"
            ) from error
        receipt_path.unlink(missing_ok=True)
        for path in created_artifacts:
            path.unlink(missing_ok=True)
        raise
    finally:
        shutil.rmtree(stage, ignore_errors=True)


def dispatch(request):
    operation = request["operation"]
    if operation == "inventory":
        return inventory(request.get("database"), request.get("registry"))
    if operation == "inspect":
        return inspect_session(request["session"])
    if operation == "plan-move":
        record = inspect_session(request["session"])
        project = project_snapshot(record["cwd"], record.get("include", []))
        artifacts = session_files(record)
        return {
            "session": record,
            "project": {
                k: v
                for k, v in project.items()
                if k not in ("index", "worktree", "untracked")
            },
            "artifact_count": len(artifacts),
            "artifact_bytes": sum(map(len, artifacts.values())),
            "untracked_count": len(project["untracked"]),
        }
    if operation == "check-destination":
        return {
            "destination": destination_check(
                request["destination"], request.get("orca"), request.get("agent")
            )
        }
    if operation == "export":
        with session_lock(request["session"]):
            return export_session(request["session"])
    if operation == "import":
        return import_session(request)
    if operation == "open":
        with session_lock(request["session"]) as lease:
            return launch_once(
                require_stopped(request["session"]), request.get("orca"), lease
            )
    raise ValueError("Unknown session worker operation")


if __name__ == "__main__":
    try:
        request = json.load(sys.stdin)
        print(json.dumps({"ok": True, "result": dispatch(request)}))
    except (
        ValueError,
        OSError,
        subprocess.SubprocessError,
        KeyError,
        TypeError,
        sqlite3.Error,
        zipfile.BadZipFile,
        RuntimeError,
    ) as error:
        print(json.dumps({"ok": False, "error": str(error)}))
        raise SystemExit(1)
