"""Shared helpers for the rosfix research studies (corpus, clone cache, running rosfix).

Nothing here is used by rosfix itself. All network access is plain `git clone` / HTTPS GET of
public data (rosdistro, GitHub). Clones live in research/.cache (git-ignored).
"""

import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
CACHE = Path(os.environ.get("ROSFIX_RESEARCH_CACHE", ROOT / ".cache"))
DATA = ROOT / "data"
EXE = ".exe" if os.name == "nt" else ""
ROSFIX = Path(os.environ.get("ROSFIX_BIN", REPO / "target" / "release" / f"rosfix{EXE}"))

# File kinds rosfix reads. A commit touching only these is a "configuration commit".
CONFIG_RE = re.compile(
    r"(\.(ya?ml|launch|urdf|xacro|sdf|srdf)$)|(\.launch\.(py|xml|ya?ml)$)|(_launch\.py$)"
    r"|(^|/)(package\.xml|CMakeLists\.txt|setup\.py|setup\.cfg)$"
    r"|(^|/)launch/[^/]+\.(py|xml)$|(^|/)(permissions|governance|policy)[^/]*\.xml$"
)


def run(cmd, cwd=None, timeout=None):
    try:
        return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, encoding="utf-8",
                              errors="replace", check=False, timeout=timeout)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(cmd, 124, "", "timeout")


def slug(url):
    """https://github.com/Owner/Name(.git) -> Owner/Name"""
    m = re.search(r"github\.com[/:]([^/]+)/([^/]+?)(\.git)?/?$", url)
    return f"{m.group(1)}/{m.group(2)}" if m else None


def clone(full_name, history=True):
    """Clone (or reuse) a repository. history=True: full history, blobs fetched lazily."""
    dest = CACHE / ("hist" if history else "head") / full_name.replace("/", "__")
    if (dest / ".git").exists() or (dest / "HEAD").exists():
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    args = ["git", "clone", "--quiet", "--no-tags"]
    args += ["--filter=blob:none"] if history else ["--depth", "1", "--recurse-submodules=no"]
    r = run(args + [f"https://github.com/{full_name}.git", str(dest)], timeout=900)
    if r.returncode:
        print(f"clone failed {full_name}: {r.stderr.strip()[:200]}", file=sys.stderr)
        return None
    return dest


def rosfix_json(path, extra=(), timeout=300):
    """Run rosfix on a directory. Returns (report dict or None, wall seconds, error text)."""
    t = time.perf_counter()
    r = run([str(ROSFIX), str(path), "--format", "json", "--no-ros", "--fail-on", "never",
             *extra], timeout=timeout)
    dt = time.perf_counter() - t
    if r.returncode not in (0, 1):
        return None, dt, (r.stderr or r.stdout).strip()[:500]
    try:
        return json.loads(r.stdout), dt, ""
    except json.JSONDecodeError as e:
        return None, dt, f"bad json: {e}"


def findings(report):
    """Compact, line-independent view of a report's diagnostics."""
    out = []
    for d in (report or {}).get("diagnostics", []):
        loc = d.get("location") or {}
        out.append({"fp": d["fingerprint"], "rule": d["rule"], "severity": d["severity"],
                    "file": loc.get("file"), "line": loc.get("line"), "message": d["message"],
                    "has_fix": d.get("fix") is not None})
    return out


def load_jsonl(p):
    p = Path(p)
    if not p.exists():
        return []
    return [json.loads(l) for l in p.read_text(encoding="utf-8").splitlines() if l.strip()]


def append_jsonl(p, rows):
    with open(p, "a", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
