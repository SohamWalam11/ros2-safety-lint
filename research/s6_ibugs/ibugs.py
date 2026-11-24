"""Study 6: rosfix on the iBugs dataset (interaction bugs in ROS 2 Humble projects).

Dataset: Chen et al., "An Empirical Study of Interaction Bugs in ROS-based Software",
arXiv 2507.10235 (2025); 121 bugs from ten ROS 2 projects, published at
https://anonymous.4open.science/r/Understanding-Interaction-Bugs-in-Robotic-Systems-FC8E
(`Interaction Bug Set.csv`, copied to research/data/s6_ibugs.csv).

Bugs whose root cause a file-reading tool could detect in principle are replayed: the fix pull
request is resolved through the GitHub CLI (the PR itself, or the PRs that closed the issue),
rosfix runs on the merge commit's first parent and on the merge commit, and the bug counts as
detected when a finding in a package the fix touched disappears (the Study 1 criterion).

    python -I research/s6_ibugs/ibugs.py            # writes data/s6_ibugs_rows.jsonl, s6_ibugs.md
"""

import collections
import csv
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import DATA, clone, run  # noqa: E402
from s1_mined.mine import checkout, diff_findings, distro_for, replay  # noqa: E402

# Root causes (iBugs taxonomy) that concern configuration or wiring a static checker can read.
STATIC_CAUSES = {
    "Naming mismatch",
    "Namespace mishandling",
    "Incorrect establishment",
    "Parameter mishandling",
    "Improper callback scheduling settings",
    "Incorrect hardware interfaces",
}
SOURCE_RE = re.compile(r"\.(cpp|hpp|h|cc|py|ya?ml|xml|launch|urdf|xacro|txt|cfg)$")


def gh_json(args):
    r = run(["gh", *args], timeout=120)
    if r.returncode:
        return None
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        return None


def fix_prs(full, kind, num):
    """Merged pull requests that fix the bug: the PR itself, or the ones that closed the issue."""
    if kind == "pull":
        nums = [num]
    else:
        v = gh_json(["issue", "view", str(num), "--repo", full, "--json", "closedByPullRequestsReferences"])
        nums = [p["number"] for p in (v or {}).get("closedByPullRequestsReferences", [])
                if p.get("repository", {}).get("nameWithOwner", full).lower() == full.lower()]
    out = []
    for n in nums:
        v = gh_json(["pr", "view", str(n), "--repo", full, "--json", "mergeCommit,mergedAt,files,state"])
        if v and v.get("state") == "MERGED" and v.get("mergeCommit"):
            out.append({"pr": n, "sha": v["mergeCommit"]["oid"], "date": (v.get("mergedAt") or "")[:10],
                        "files": [f["path"] for f in v.get("files", [])]})
    return out


def package_of(path, wt):
    """The package directory (nearest ancestor with package.xml) of a repository path."""
    p = Path(path).parent
    while True:
        if (wt / p / "package.xml").exists():
            return str(p).replace("\\", "/")
        if p == p.parent or str(p) in ("", "."):
            return ""
        p = p.parent


def main():
    rows_path = DATA / "s6_ibugs_rows.jsonl"
    bugs = list(csv.DictReader(open(DATA / "s6_ibugs.csv", encoding="utf-8-sig", errors="replace")))
    eligible = [b for b in bugs if b["Root Cause"].strip() in STATIC_CAUSES]
    print(f"{len(bugs)} bugs, {len(eligible)} with a statically detectable root cause", file=sys.stderr)
    rows = []
    for b in eligible:
        m = re.search(r"github\.com/([^/]+/[^/]+)/(issues|pull)/(\d+)", b["Bug Link"])
        row = {"id": b["Bug ID"], "link": b["Bug Link"], "project": b["Project"], "cause": b["Root Cause"].strip()}
        if not m:
            rows.append({**row, "status": "unparsable link"})
            continue
        full, kind, num = m.group(1), m.group(2), int(m.group(3))
        prs = fix_prs(full, kind, num)
        if not prs:
            rows.append({**row, "status": "no merged fix PR linked"})
            continue
        fix = prs[0]
        row.update({"repo": full, "fix_pr": fix["pr"], "fix_sha": fix["sha"], "files": fix["files"]})
        row["reads_files"] = any(SOURCE_RE.search(f) for f in fix["files"])
        wt = clone(full, history=True)
        if not wt:
            rows.append({**row, "status": "clone failed"})
            continue
        parent = run(["git", "rev-parse", f"{fix['sha']}^1"], cwd=wt).stdout.strip()
        if not parent or not checkout(wt, parent):
            rows.append({**row, "status": "checkout failed"})
            continue
        touched = {package_of(f, wt) for f in fix["files"]}
        distro = distro_for(fix["date"] or "2024-01-01")
        _, before, _, err_b = replay(wt, distro)
        if not checkout(wt, fix["sha"]):
            rows.append({**row, "status": "checkout failed"})
            continue
        _, after, _, err_a = replay(wt, distro)
        gone, _ = diff_findings(before, after)
        hits = [g for g in gone if g["file"] in fix["files"] or any(t and g["file"].startswith(t + "/") for t in touched)]
        rows.append({**row, "status": "replayed", "distro": distro, "error": err_b or err_a or None,
                     "detected": bool(hits), "disappeared": hits})
        print(f"{b['Bug ID']:>4} {full:<45} {row['cause']:<35} detected={bool(hits)}", file=sys.stderr)
    rows_path.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows), encoding="utf-8")

    st = collections.Counter(r["status"] for r in rows)
    rep = [r for r in rows if r["status"] == "replayed"]
    det = [r for r in rep if r["detected"]]
    by = collections.defaultdict(lambda: [0, 0, 0])
    for r in rows:
        by[r["cause"]][0] += 1
    for r in rep:
        by[r["cause"]][1] += 1
        by[r["cause"]][2] += r["detected"]
    lines = [
        "# Study 6: rosfix on iBugs (interaction bugs in ROS 2)",
        "",
        f"Dataset: 121 bugs (Chen et al., arXiv 2507.10235). {len(rows)} have a root cause a static",
        f"checker could detect in principle. Replayed: {len(rep)} (fix PR resolved and checked out).",
        f"Detected (a finding in a fix-touched package disappears with the fix): {len(det)} of {len(rep)}.",
        "",
        "| Root cause | Bugs | Replayed | Detected |",
        "|---|---:|---:|---:|",
        *[f"| {c} | {v[0]} | {v[1]} | {v[2]} |" for c, v in sorted(by.items())],
        "",
        "Not replayed: " + ", ".join(f"{k} {v}" for k, v in st.items() if k != "replayed"),
        "",
        "## Detected bugs",
        "",
        *[f"- #{r['id']} {r['link']} ({r['cause']}): " + "; ".join(f"{h['rule']} {h['file']}: {h['message'][:120]}" for h in r["disappeared"][:3]) for r in det],
    ]
    (DATA / "s6_ibugs.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
