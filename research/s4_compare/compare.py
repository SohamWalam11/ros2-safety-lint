"""Study 4: rosfix vs ros2inspector (0.2.0) on the same inputs.

Other candidate tools could not be run here: lint_launch and HAROS2 need a ROS 2 installation
(launch / launch_ros Python packages), unavailable on this Windows host without Docker/WSL.

Part 1 (ecosystem): both tools on every Study 2 HEAD clone. Per repo: runtime, crash, number of
findings. Overlap on the one finding class both tools define the same way, "one topic, two
message types" (rosfix T003, ros2inspector interface_type_mismatch), compared by topic name.

Part 2 (real bugs): both tools on the parent and fix of every Study 1 commit labelled a genuine
misconfiguration fix. A tool "detects" a commit when one of its findings present on the parent
is gone on the fix and that finding mentions a file, package, node or name the commit touched
(the same proxy as Study 1; rosfix detections were then verified by hand: s1_labels/verify.json).

ros2inspector needs a colcon layout (<ws>/src/<pkgs>) and fails on paths containing spaces, so
each input is exposed as <WS_ROOT>/<id>/src -> junction to the clone (WS_ROOT has no spaces;
the cache must live on a space-free path too, see research/README.md).

Usage: python s4_compare/compare.py part1|part2 [--workers 6]
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import CACHE, DATA, load_jsonl, rosfix_json, run  # noqa: E402

R2I = Path(os.environ.get("R2I_BIN", CACHE / "venv-r2i" / "Scripts" / "ros2inspector.exe"))
WS_ROOT = Path(os.environ.get("R2I_WS", "D:/r2i/ws_cmp"))
TOPIC_RE = re.compile(r"Topic '([^']+)' has incompatible interface types")


def junction(link, target):
    link.parent.mkdir(parents=True, exist_ok=True)
    if link.exists():
        return
    if os.name == "nt":
        subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(target)], capture_output=True)
    else:
        link.symlink_to(target, target_is_directory=True)


def r2i(path, key):
    ws = WS_ROOT / key
    junction(ws / "src", path)
    t = time.perf_counter()
    r = run([str(R2I), "-q", "audit", ".", "--format", "json", "--no-cache"], cwd=ws, timeout=600)
    dt = time.perf_counter() - t
    try:
        rep = json.loads(r.stdout)
    except json.JSONDecodeError:
        return None, dt, (r.stderr or r.stdout).strip()[-300:]
    return rep, dt, ""


def part1(workers):
    s2 = {r["repo"]: r for r in load_jsonl(DATA / "s2_results_jazzy.jsonl") if "findings" in r}
    repos = sorted(p for p in (CACHE / "head").iterdir() if p.is_dir() and p.name.replace("__", "/") in s2)

    def one(p):
        name = p.name.replace("__", "/")
        rep, dt, err = r2i(p, "h_" + p.name)
        row = {"repo": name, "r2i_secs": round(dt, 3), "r2i_error": err or None,
               "rosfix_secs": s2[name]["secs"], "rosfix_findings": len(s2[name]["findings"]),
               "rosfix_T003_topics": sorted({m.group(1) for f in s2[name]["findings"] if f["rule"] == "T003"
                                             for m in [re.search(r"`(/[^`]+)`", f["message"])] if m})}
        if rep:
            row["r2i_findings"] = len(rep.get("findings", []))
            row["r2i_by_rule"] = rep.get("summary", {}).get("by_rule", {})
            row["r2i_complete"] = rep.get("completeness", {}).get("complete")
            row["r2i_type_topics"] = sorted({m.group(1) for f in rep.get("findings", [])
                                             for m in [TOPIC_RE.search(f.get("message", ""))] if m})
        return row

    with ThreadPoolExecutor(workers) as ex:
        rows = list(ex.map(one, repos))
    (DATA / "s4_part1.jsonl").write_text("\n".join(json.dumps(r) for r in rows) + "\n")
    ok = [r for r in rows if "r2i_findings" in r]
    both = sum(len(set(r["rosfix_T003_topics"]) & set(r["r2i_type_topics"])) for r in ok)
    only_rf = sum(len(set(r["rosfix_T003_topics"]) - set(r["r2i_type_topics"])) for r in ok)
    only_ri = sum(len(set(r["r2i_type_topics"]) - set(r["rosfix_T003_topics"])) for r in ok)
    med = lambda xs: sorted(xs)[len(xs) // 2] if xs else None  # noqa: E731
    summary = {"repos": len(rows), "r2i_ok": len(ok), "r2i_failed": len(rows) - len(ok),
               "r2i_with_findings": sum(1 for r in ok if r["r2i_findings"]),
               "r2i_findings": sum(r["r2i_findings"] for r in ok),
               "rosfix_findings": sum(r["rosfix_findings"] for r in rows),
               "median_secs": {"rosfix": med([r["rosfix_secs"] for r in rows]), "r2i": med([r["r2i_secs"] for r in ok])},
               "total_secs": {"rosfix": round(sum(r["rosfix_secs"] for r in rows), 1),
                              "r2i": round(sum(r["r2i_secs"] for r in rows), 1)},
               "type_conflict_topics": {"both": both, "rosfix_only": only_rf, "r2i_only": only_ri}}
    (DATA / "s4_part1_summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps(summary, indent=1))


def part2(workers):
    cands = {r["id"]: r for r in load_jsonl(DATA / "s1_candidates.jsonl")}
    labels = {}
    for f in (DATA / "s1_labels").glob("labels_batch_*.json"):
        for x in json.loads(f.read_text(encoding="utf-8")):
            labels[x["id"]] = x
    ids = sorted(i for i, x in labels.items() if x["misconfig_fix"] == "yes")
    by_repo = {}
    for i in ids:
        by_repo.setdefault(cands[i]["repo"], []).append(i)

    def touched_terms(c):
        terms = set()
        for f in c["files"]:
            parts = f.split("/")
            terms.update(p for p in parts if p)
            terms.add(Path(f).stem)
        for line in c["diff"].splitlines():  # identifiers on changed lines
            if line[:1] in "+-" and not line.startswith(("+++", "---")):
                terms.update(t for t in re.findall(r"[A-Za-z_/][\w/.-]{3,}", line))
        return {t for t in terms if len(t) > 3}

    def fp(f):
        return (f.get("rule_type"), f.get("message"))

    def repo_job(repo):
        clone = CACHE / "hist" / repo.replace("/", "__")
        rows = []
        for i in by_repo[repo]:
            c = cands[i]
            wt = WS_ROOT.parent / "wt_cmp" / i.replace("/", "__").replace("@", "_")
            outs = {}
            for side, rev in (("before", c["parent"]), ("after", c["sha"])):
                run(["git", "-C", str(clone), "worktree", "add", "-f", "--detach", str(wt), rev], timeout=900)
                rep, dt, err = r2i(wt, f"c_{wt.name}_{side}")
                outs[side] = (rep, dt, err)
                run(["git", "-C", str(clone), "worktree", "remove", "--force", str(wt)], timeout=300)
                link = WS_ROOT / f"c_{wt.name}_{side}" / "src"
                if link.exists():
                    os.rmdir(link)  # remove the junction only, never the target
            b, a = outs["before"][0], outs["after"][0]
            if not b or not a:
                rows.append({"id": i, "error": outs["before"][2] or outs["after"][2]})
                continue
            after = {fp(f) for f in a.get("findings", [])}
            gone = [f for f in b.get("findings", []) if fp(f) not in after]
            terms = touched_terms(c)
            rel = [f for f in gone if any(t in (f.get("message") or "") or t in " ".join(f.get("affected_entities") or [])
                                          for t in terms)]
            rows.append({"id": i, "category": labels[i]["category"], "gone": gone, "related": rel,
                         "secs": [round(outs["before"][1], 2), round(outs["after"][1], 2)]})
        return rows

    with ThreadPoolExecutor(workers) as ex:
        rows = [r for rs in ex.map(repo_job, sorted(by_repo)) for r in rs]
    (DATA / "s4_part2.jsonl").write_text("\n".join(json.dumps(r) for r in rows) + "\n")
    verify = json.loads((DATA / "s1_labels" / "verify.json").read_text())
    rf_det = {i for i, v in verify.items() if v["is_the_bug"]}
    ri_det = {r["id"] for r in rows if r.get("related")}
    summary = {"genuine_commits": len(ids), "r2i_errors": sum(1 for r in rows if "error" in r),
               "rosfix_detected_verified": len(rf_det & set(ids)),
               "r2i_detected_proxy": len(ri_det), "both": len(rf_det & ri_det),
               "r2i_only": sorted(ri_det - rf_det), "note": "r2i detections are proxy-based; verify r2i_only by hand"}
    (DATA / "s4_part2_summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps(summary, indent=1))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("part", choices=["part1", "part2"])
    ap.add_argument("--workers", type=int, default=6)
    a = ap.parse_args()
    (part1 if a.part == "part1" else part2)(a.workers)


if __name__ == "__main__":
    main()
