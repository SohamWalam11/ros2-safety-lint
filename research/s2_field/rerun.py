"""Re-run Study 2 with another rosfix build on exactly the same clones, and diff the findings.

Re-analyses every repository of a previous Study 2 result file at the commit recorded there
(the cached clone must still be at that commit; repositories whose clone moved are reported
and skipped). Findings are matched by fingerprint, so a changed message does not count as a
change. Triage verdicts (data/s2_triage/verdicts_*.json) classify what disappeared or stayed.

Usage:
  ROSFIX_BIN=<new rosfix> python s2_field/rerun.py run  --base data/s2_results_jazzy.jsonl --out data/s2_rerun.jsonl
  python s2_field/rerun.py diff --base data/s2_results_jazzy.jsonl --new data/s2_rerun.jsonl --md data/s2_rerun.md
"""

import argparse
import json
import sys
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import CACHE, DATA, append_jsonl, findings, load_jsonl, rosfix_json, run  # noqa: E402


def analyse(row, distro):
    path = CACHE / "head" / row["repo"].replace("/", "__")
    out = {"repo": row["repo"], "commit": row.get("commit")}
    if not (path / ".git").exists():
        return {**out, "error": "clone missing"}
    sha = run(["git", "rev-parse", "HEAD"], cwd=path).stdout.strip()
    if row.get("commit") and sha != row["commit"]:
        return {**out, "error": f"clone moved to {sha}"}
    rep, secs, err = rosfix_json(path, ["--distro", distro])
    out.update({"secs": round(secs, 3), "error": err or None})
    if rep:
        out["findings"] = findings(rep)
        out["version"] = rep.get("version")
    return out


def cmd_run(a):
    base = [r for r in load_jsonl(Path(a.base)) if not r.get("error")]
    out = Path(a.out)
    done = {r["repo"] for r in load_jsonl(out)} if out.exists() else set()
    todo = [r for r in base if r["repo"] not in done]
    print(f"{len(todo)} to analyse ({len(done)} done)", file=sys.stderr)
    with ThreadPoolExecutor(a.workers) as ex:
        for i, row in enumerate(ex.map(lambda r: analyse(r, a.distro), todo), 1):
            append_jsonl(out, [row])
            if i % 50 == 0:
                print(f"[{i}/{len(todo)}]", file=sys.stderr)


def verdicts():
    """Fingerprint -> triage verdict (tp/fp/...), via the triage sample's tid -> fp mapping."""
    fp_of = {r["tid"]: r["fp"] for r in load_jsonl(DATA / "s2_triage" / "sample.jsonl")}
    v = {}
    for f in sorted((DATA / "s2_triage").glob("verdicts_*.json")):
        for item in json.loads(f.read_text(encoding="utf-8")):
            if item.get("tid") in fp_of:
                v[fp_of[item["tid"]]] = item.get("verdict")
    return v


def cmd_diff(a):
    base = {r["repo"]: r for r in load_jsonl(Path(a.base))}
    new = {r["repo"]: r for r in load_jsonl(Path(a.new))}
    ver = verdicts()
    skipped = [r for r in new.values() if r.get("error")]
    both = [k for k, r in new.items() if not r.get("error") and not base[k].get("error")]
    rules_b, rules_n = Counter(), Counter()
    gone, added = defaultdict(list), defaultdict(list)
    for repo in both:
        fb = {f["fp"]: f for f in base[repo].get("findings", [])}
        fn = {f["fp"]: f for f in new[repo].get("findings", [])}
        rules_b.update(f["rule"] for f in fb.values())
        rules_n.update(f["rule"] for f in fn.values())
        for fp in fb.keys() - fn.keys():
            gone[fb[fp]["rule"]].append((repo, fb[fp]))
        for fp in fn.keys() - fb.keys():
            added[fn[fp]["rule"]].append((repo, fn[fp]))
    tri_gone = Counter(ver[f["fp"]] for v in gone.values() for _, f in v if f["fp"] in ver)
    tri_kept = Counter(
        ver[f["fp"]] for repo in both for f in new[repo].get("findings", []) if f["fp"] in ver
    )
    lines = [
        f"# Study 2 re-run: {len(both)} repositories compared",
        "",
        f"Skipped: {len(skipped)} ({Counter(r['error'].split(' ')[0] for r in skipped)})",
        f"Findings: {sum(rules_b.values())} before, {sum(rules_n.values())} after; "
        f"{sum(map(len, gone.values()))} disappeared, {sum(map(len, added.values()))} new.",
        f"Triaged findings that disappeared: {dict(tri_gone)}; triaged findings still reported: {dict(tri_kept)}.",
        "",
        "| Rule | Before | After | Gone | New |",
        "|---|---:|---:|---:|---:|",
    ]
    for rule in sorted(set(rules_b) | set(rules_n)):
        lines.append(f"| {rule} | {rules_b[rule]} | {rules_n[rule]} | {len(gone[rule])} | {len(added[rule])} |")
    lines += ["", "## Disappeared findings with a triage verdict", ""]
    for rule, items in sorted(gone.items()):
        for repo, f in items:
            if f["fp"] in ver:
                lines.append(f"- {rule} [{ver[f['fp']]}] {repo} `{f['file']}:{f['line']}`: {f['message'][:140]}")
    lines += ["", "## New findings (up to 5 per rule)", ""]
    for rule, items in sorted(added.items()):
        for repo, f in items[:5]:
            lines.append(f"- {rule} {repo} `{f['file']}:{f['line']}`: {f['message'][:140]}")
    Path(a.md).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines[:12 + len(set(rules_b) | set(rules_n))]))


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--base", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--distro", default="jazzy")
    r.add_argument("--workers", type=int, default=4)
    d = sub.add_parser("diff")
    d.add_argument("--base", required=True)
    d.add_argument("--new", required=True)
    d.add_argument("--md", required=True)
    a = ap.parse_args()
    {"run": cmd_run, "diff": cmd_diff}[a.cmd](a)


if __name__ == "__main__":
    main()
