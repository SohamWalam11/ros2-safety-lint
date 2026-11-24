"""Study 1, stage 1: mine candidate configuration-fix commits and replay rosfix on them.

Candidate selection is independent of rosfix: a non-merge commit is a candidate when every file
it touches is a configuration file rosfix reads (common.CONFIG_RE), it touches 1..8 files, and
its subject looks like a fix (FIX_RE) and not like a release/merge (SKIP_RE).

Replay: rosfix runs on the parent tree and on the commit tree. Findings are matched by
fingerprint (rule|file|message, no line numbers). Per candidate we store the findings that
disappeared, the findings that appeared, and the commit diff (for blind labelling).

Usage: python s1_mined/mine.py --repos-from data/s1_repos.txt [--per-repo 40] [--workers 6]
Output: data/s1_candidates.jsonl (one line per candidate, resumable: finished repos are skipped)
"""

import argparse
import re
import sys
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import CONFIG_RE, DATA, append_jsonl, clone, findings, load_jsonl, rosfix_json, run  # noqa: E402

FIX_RE = re.compile(
    r"\b(fix\w*|typo\w*|wrong|missing|correct\w*|broken|remap\w*|param\w*|namespace\w*|frame\w*|"
    r"plugin\w*|depend\w*|qos|sim_?time|topic\w*|joint\w*|controller\w*|install\w*|launch\w*)\b", re.I)
SKIP_RE = re.compile(r"\b(merge|release|bump|changelog|version|backport|revert|pre-commit|lint\w*|format\w*|style)\b"
                     r"|^\s*\d+\.\d+\.\d+\s*$", re.I)
MAX_DIFF = 8000
OUT = DATA / "s1_candidates.jsonl"
DONE = DATA / "s1_done.txt"


def candidates(repo, limit):
    log = run(["git", "log", "--no-merges", "--format=@@%H%x09%P%x09%cI%x09%s", "--name-only", "HEAD"],
              cwd=repo, timeout=600).stdout
    out = []
    for block in log.split("@@")[1:]:
        head, *files = [l for l in block.splitlines() if l.strip()]
        sha, parents, date, subject = (head.split("\t") + ["", "", ""])[:4]
        if " " in parents or not parents:  # root commit or merge
            continue
        if not files or len(files) > 8 or not all(CONFIG_RE.search(f) for f in files):
            continue
        if not FIX_RE.search(subject) or SKIP_RE.search(subject):
            continue
        out.append({"sha": sha, "parent": parents, "date": date, "subject": subject, "files": files})
        if len(out) >= limit:
            break
    return out


def distro_for(date):
    """The ROS 2 LTS/regular distribution current at the commit date (for knowledge packs)."""
    if date < "2024-05-23":
        return "humble"
    if date < "2025-05-23":
        return "jazzy"
    return "kilted"


def replay(repo, distro):
    """Run rosfix twice: without knowledge packs, and with the packs of `distro`."""
    plain, t, err = rosfix_json(repo)
    packed, t2, err2 = rosfix_json(repo, ["--distro", distro])
    return findings(plain), findings(packed), t + t2, err or err2


def diff_findings(fb, fa):
    a, b = {f["fp"] for f in fa}, {f["fp"] for f in fb}
    return [f for f in fb if f["fp"] not in a], [f for f in fa if f["fp"] not in b]


def checkout(wt, rev):
    r = run(["git", "checkout", "--quiet", "--force", "--detach", rev], cwd=wt, timeout=900)
    run(["git", "clean", "-fdxq"], cwd=wt)
    return r.returncode == 0


def mine_repo(full, per_repo):
    repo = clone(full, history=True)
    if not repo:
        return full, [], "clone failed"
    cands = candidates(repo, per_repo)
    rows = []
    for c in cands:
        diff = run(["git", "show", "--format=%B", "--unified=2", c["sha"]], cwd=repo, timeout=300).stdout
        if not checkout(repo, c["parent"]):
            continue
        distro = distro_for(c["date"])
        pb, kb, t_before, err_b = replay(repo, distro)
        if not checkout(repo, c["sha"]):
            continue
        pa, ka, t_after, err_a = replay(repo, distro)
        gone, new = diff_findings(kb, ka)
        gone_plain, new_plain = diff_findings(pb, pa)
        rows.append({
            "id": f"{full}@{c['sha'][:10]}", "repo": full, **c, "distro": distro,
            "diff": diff[:MAX_DIFF], "diff_truncated": len(diff) > MAX_DIFF,
            "n_before": len(kb), "n_after": len(ka),
            "disappeared": gone, "appeared": new,
            "disappeared_nopack": gone_plain, "appeared_nopack": new_plain,
            "secs": [round(t_before, 3), round(t_after, 3)],
            "error": err_b or err_a or None,
        })
    return full, rows, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repos-from", required=True)
    ap.add_argument("--per-repo", type=int, default=40)
    ap.add_argument("--workers", type=int, default=6)
    a = ap.parse_args()
    repos = [l.strip() for l in Path(a.repos_from).read_text().splitlines()
             if l.strip() and not l.startswith("#")]
    done = set(DONE.read_text().split()) if DONE.exists() else set()
    todo = [r for r in repos if r not in done]
    print(f"{len(todo)} repos to mine ({len(done)} done)", file=sys.stderr)
    with ProcessPoolExecutor(a.workers) as ex:
        futs = {ex.submit(mine_repo, r, a.per_repo): r for r in todo}
        for i, f in enumerate(as_completed(futs), 1):
            r = futs[f]
            try:
                full, rows, err = f.result()
            except Exception as e:  # keep going; record nothing for this repo
                print(f"[{i}/{len(todo)}] {r}: crashed {e}", file=sys.stderr)
                continue
            append_jsonl(OUT, rows)
            with open(DONE, "a") as d:
                d.write(full + "\n")
            print(f"[{i}/{len(todo)}] {full}: {len(rows)} candidates {err or ''}", file=sys.stderr)
    print(f"total candidates: {len(load_jsonl(OUT))}", file=sys.stderr)


if __name__ == "__main__":
    main()
