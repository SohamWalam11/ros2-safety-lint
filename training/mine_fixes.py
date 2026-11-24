#!/usr/bin/env python3
"""Mine real misconfiguration fixes from public ROS 2 repositories.

For each candidate commit that only touches configuration files (YAML, launch, package.xml,
CMakeLists.txt, setup.py/cfg, URDF/xacro), rosfix is run on the parent and on the commit:

  a finding with a repair site that exists *before* and disappears *after*, where the text
  the commit wrote at that spot is one of rosfix's options  →  one training example.

Only repositories under Apache-2.0, MIT or BSD licenses are used (read from the clone's LICENSE
file); provenance (repo, commit, license) is stored with every example. Needs only `git` and
network access to public repositories: no API, no token, no account. rosfix itself never
contacts these services.

Usage: python mine_fixes.py --repos repos.txt --out mined.jsonl [--max-commits 300]
"""

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

PERMISSIVE = {"apache-2.0", "mit", "bsd-2-clause", "bsd-3-clause"}
CONFIG_RE = re.compile(r"(\.(ya?ml|launch|xml|urdf|xacro|launch\.py)$)|(^|/)(package\.xml|CMakeLists\.txt|setup\.(py|cfg))$")
MSG_RE = re.compile(r"\b(fix|typo|wrong|missing|correct|rename|remap|param|namespace|frame|plugin|depend|install|joint|qos|use_sim_time)\b", re.I)
SKIP_MSG_RE = re.compile(r"\b(merge|release|bump|changelog|version)\b", re.I)


LICENSE_FILES = ("LICENSE", "LICENSE.txt", "LICENSE.md", "LICENCE", "COPYING")
LICENSE_SIGNS = [  # (spdx key, distinctive phrase), checked in order
    ("apache-2.0", "Apache License"),
    ("bsd-3-clause", "Neither the name"),
    ("bsd-2-clause", "Redistribution and use in source and binary forms"),
    ("mit", "Permission is hereby granted, free of charge"),
]


def detect_license(repo):
    """License key from the cloned repository's LICENSE file (no API, no token), or ''."""
    for name in LICENSE_FILES:
        p = Path(repo) / name
        if p.is_file():
            text = p.read_text(encoding="utf-8", errors="replace")
            for key, phrase in LICENSE_SIGNS:
                if phrase in text:
                    return key
            return "other"
    return ""


def run(cmd, cwd=None):
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, check=False)


def prompts(workspace):
    out = run(["rosfix", str(workspace), "--assist-cases", "--no-ros"]).stdout
    return [json.loads(l) for l in out.splitlines() if l.strip()]


def text_at(workspace, rel, line):
    p = Path(workspace) / rel
    try:
        return p.read_text(encoding="utf-8", errors="replace").splitlines()[line - 1]
    except (OSError, IndexError):
        return None


def mine_repo(full_name, max_commits, out):
    found = 0
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp) / "repo"
        if run(["git", "clone", "--quiet", "--filter=blob:none", f"https://github.com/{full_name}.git", str(repo)]).returncode:
            return 0
        lic = detect_license(repo)
        if lic not in PERMISSIVE:
            print(f"skip {full_name}: license {lic or 'unknown'}", file=sys.stderr)
            return 0
        log = run(["git", "log", f"-{max_commits * 5}", "--no-merges", "--format=%H%x09%s"], cwd=repo).stdout
        for entry in log.splitlines()[: max_commits * 5]:
            sha, _, subject = entry.partition("\t")
            if not MSG_RE.search(subject) or SKIP_MSG_RE.search(subject):
                continue
            files = run(["git", "show", "--name-only", "--format=", sha], cwd=repo).stdout.split()
            if not files or len(files) > 5 or not all(CONFIG_RE.search(f) for f in files):
                continue
            before, after = Path(tmp) / "before", Path(tmp) / "after"
            for d, rev in ((before, f"{sha}^"), (after, sha)):
                run(["git", "worktree", "remove", "--force", str(d)], cwd=repo)
                if run(["git", "worktree", "add", "--detach", str(d), rev], cwd=repo).returncode:
                    break
            else:
                after_keys = {(p["rule"], p["file"], p["line"]) for p in prompts(after)}
                for p in prompts(before):
                    if (p["rule"], p["file"], p["line"]) in after_keys or p["file"] not in files:
                        continue
                    fixed_line = text_at(after, p["file"], p["line"]) or ""
                    hits = [i for i, o in enumerate(p["options"]) if re.search(rf"(?<![\w]){re.escape(o)}(?![\w])", fixed_line)]
                    if len(hits) != 1:
                        continue
                    out.write(json.dumps({
                        "current": p["current"],
                        "options": p["options"],
                        "answer": hits[0],
                        "rule": p["rule"],
                        "op": "mined",
                        "repo": full_name.split("/")[-1],
                        "source": {"repo": full_name, "commit": sha, "license": lic},
                    }) + "\n")
                    found += 1
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repos", required=True, help="file with one owner/name per line")
    ap.add_argument("--out", required=True)
    ap.add_argument("--max-commits", type=int, default=300)
    args = ap.parse_args()
    repos = [l.strip() for l in Path(args.repos).read_text().splitlines() if l.strip() and not l.startswith("#")]
    total = 0
    with open(args.out, "a", encoding="utf-8") as out:
        for r in repos:
            n = mine_repo(r, args.max_commits, out)
            print(f"{r}: {n} examples", file=sys.stderr)
            total += n
    print(f"total: {total}", file=sys.stderr)


if __name__ == "__main__":
    main()
