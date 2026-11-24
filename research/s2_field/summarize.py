"""Study 2: summarise the ecosystem run and draw a stratified finding sample for triage.

Prevalence: per rule, number of findings and of repositories with at least one.
Cost: wall time per repository (median, p95, max) vs. number of packages.
Triage sample: up to --per-rule findings per rule (seeded), with a source excerpt, written to
data/s2_triage/sample.jsonl for manual true/false-positive review (precision per rule).

Usage: python s2_field/summarize.py [--distro jazzy] [--per-rule 8]
"""

import argparse
import collections
import json
import random
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import CACHE, DATA, load_jsonl  # noqa: E402


def excerpt(repo, file, line, ctx=6):
    p = CACHE / "head" / repo.replace("/", "__") / (file or "")
    try:
        lines = p.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return ""
    lo, hi = max(0, (line or 1) - 1 - ctx), min(len(lines), (line or 1) + ctx)
    return "\n".join(f"{i + 1:5d}  {lines[i]}" for i in range(lo, hi))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--distro", default="jazzy")
    ap.add_argument("--per-rule", type=int, default=8)
    ap.add_argument("--seed", type=int, default=20261120)
    a = ap.parse_args()
    rows = load_jsonl(DATA / f"s2_results_{a.distro}.jsonl")
    ok = [r for r in rows if "findings" in r]
    by_rule, repos_rule, sev = collections.Counter(), collections.defaultdict(set), {}
    for r in ok:
        for f in r["findings"]:
            by_rule[f["rule"]] += 1
            repos_rule[f["rule"]].add(r["repo"])
            sev[f["rule"]] = f["severity"]
    secs = sorted(r["secs"] for r in ok)
    p95 = secs[int(0.95 * (len(secs) - 1))]
    with_any = sum(1 for r in ok if r["findings"])
    with_err = sum(1 for r in ok if any(f["severity"] == "error" for f in r["findings"]))
    summary = {
        "repos": len(rows), "analysed": len(ok), "failed": [(r["repo"], r.get("error")) for r in rows if "findings" not in r],
        "repos_with_findings": with_any, "repos_with_errors": with_err,
        "findings": sum(by_rule.values()),
        "per_rule": {k: {"severity": sev[k], "findings": v, "repos": len(repos_rule[k])} for k, v in by_rule.most_common()},
        "secs": {"median": statistics.median(secs), "p95": p95, "max": secs[-1], "total": round(sum(secs), 1)},
        "version": next((r.get("version") for r in ok if r.get("version")), None),
    }
    (DATA / f"s2_summary_{a.distro}.json").write_text(json.dumps(summary, indent=1))

    rng = random.Random(a.seed)
    pool = collections.defaultdict(list)
    for r in ok:
        for f in r["findings"]:
            pool[f["rule"]].append((r["repo"], r["commit"], f))
    out_dir = DATA / "s2_triage"
    out_dir.mkdir(exist_ok=True)
    sample = []
    for rule, items in sorted(pool.items()):
        # at most 2 findings per repository per rule, so one noisy repo cannot fill a stratum
        rng.shuffle(items)
        per_repo, picked = collections.Counter(), []
        for repo, sha, f in items:
            if per_repo[repo] < 2:
                picked.append((repo, sha, f))
                per_repo[repo] += 1
            if len(picked) >= a.per_rule:
                break
        for repo, sha, f in picked:
            sample.append({"tid": f"{rule}-{len(sample):03d}", "repo": repo, "commit": sha, **f,
                           "excerpt": excerpt(repo, f["file"], f["line"])})
    (out_dir / "sample.jsonl").write_text("\n".join(json.dumps(s, ensure_ascii=False) for s in sample) + "\n",
                                          encoding="utf-8")

    md = [f"# Study 2: rosfix on every {a.distro} repository in rosdistro", "",
          f"{summary['analysed']}/{summary['repos']} repositories analysed "
          f"({len(summary['failed'])} failed). {with_any} have at least one finding, {with_err} at least one error-severity finding. "
          f"Total findings: {summary['findings']}.", "",
          f"Wall time per repository: median {summary['secs']['median']:.2f} s, p95 {p95:.2f} s, max {secs[-1]:.2f} s "
          f"(total {summary['secs']['total']:.0f} s, single thread).", "",
          "| Rule | Severity | Findings | Repositories |", "|---|---|---:|---:|"]
    md += [f"| {k} | {v['severity']} | {v['findings']} | {v['repos']} |" for k, v in summary["per_rule"].items()]
    md += ["", f"Triage sample: {len(sample)} findings (≤{a.per_rule} per rule, ≤2 per repository per rule) -> data/s2_triage/sample.jsonl"]
    (DATA / f"s2_summary_{a.distro}.md").write_text("\n".join(md), encoding="utf-8")
    print("\n".join(md))


if __name__ == "__main__":
    main()
