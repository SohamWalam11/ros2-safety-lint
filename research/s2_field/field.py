"""Study 2: rosfix on the whole rosdistro ecosystem (one shallow clone per repository).

For every corpus repository: shallow-clone the branch rosdistro builds for that distro, run
rosfix with that distro's knowledge packs (no installed ROS), and record findings, timings,
and indexing statistics (`--print-graph` node counts). Output: data/s2_results.jsonl.

Usage: python s2_field/field.py [--distro jazzy] [--workers 8] [--limit N]
"""

import argparse
import json
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import CACHE, DATA, append_jsonl, findings, load_jsonl, rosfix_json, run  # noqa: E402


def head_clone(repo, branch):
    dest = CACHE / "head" / repo.replace("/", "__")
    if (dest / ".git").exists():
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    args = ["git", "clone", "--quiet", "--depth", "1", "--no-tags"]
    r = run(args + (["--branch", branch] if branch else []) + [f"https://github.com/{repo}.git", str(dest)], timeout=900)
    if r.returncode and branch:  # branch renamed/removed: fall back to the default branch
        r = run(args + [f"https://github.com/{repo}.git", str(dest)], timeout=900)
    return dest if r.returncode == 0 else None


def analyse(entry, distro):
    repo = entry["repo"]
    path = head_clone(repo, entry.get("branch"))
    if not path:
        return {"repo": repo, "error": "clone failed"}
    sha = run(["git", "rev-parse", "HEAD"], cwd=path).stdout.strip()
    rep, secs, err = rosfix_json(path, ["--distro", distro])
    n_pkgs = sum(1 for _ in path.rglob("package.xml"))
    row = {"repo": repo, "commit": sha, "branch": entry.get("branch"), "packages": n_pkgs,
           "secs": round(secs, 3), "error": err or None}
    if rep:
        row["findings"] = findings(rep)
        row["agents_ran"] = [a["id"] for a in rep.get("agents", []) if a.get("ran")]
        row["version"] = rep.get("version")
    return row


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--distro", default="jazzy")
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--limit", type=int)
    a = ap.parse_args()
    corpus = json.loads((DATA / f"corpus_{a.distro}.json").read_text())["repos"][: a.limit]
    out = DATA / f"s2_results_{a.distro}.jsonl"
    done = {r["repo"] for r in load_jsonl(out)}
    todo = [e for e in corpus if e["repo"] not in done]
    print(f"{len(todo)} to analyse ({len(done)} done)", file=sys.stderr)
    with ThreadPoolExecutor(a.workers) as ex:  # work is in git/rosfix subprocesses
        futs = {ex.submit(analyse, e, a.distro): e["repo"] for e in todo}
        for i, f in enumerate(as_completed(futs), 1):
            row = f.result()
            append_jsonl(out, [row])
            print(f"[{i}/{len(todo)}] {row['repo']}: {len(row.get('findings', []))} findings "
                  f"{row.get('error') or ''}", file=sys.stderr)


if __name__ == "__main__":
    main()
