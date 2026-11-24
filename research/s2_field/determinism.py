"""Determinism check: analyse every corpus repository N times; outputs must be identical.

A static checker used in CI with baselines must give byte-identical findings for an identical
tree. Rust's HashMap iteration order is randomised per process, so any finding message chosen
from a HashMap iteration can differ between runs (found in Study 1: T001 on crazyswarm2).

Usage: python s2_field/determinism.py [--runs 3] [--workers 8]  -> data/s2_determinism.json
"""

import argparse
import json
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import CACHE, DATA, rosfix_json  # noqa: E402


def check(path, runs):
    outs = []
    for _ in range(runs):
        rep, _, err = rosfix_json(path, ["--distro", "jazzy"])
        if not rep:
            return {"repo": path.name, "error": err}
        outs.append(sorted((d["rule"], d["fingerprint"], d["message"]) for d in rep["diagnostics"]))
    base = set(map(tuple, outs[0]))
    diff = set()
    for o in outs[1:]:
        diff |= base ^ set(map(tuple, o))
    return {"repo": path.name.replace("__", "/"), "stable": not diff,
            "unstable_rules": sorted({d[0] for d in diff}), "examples": sorted(diff)[:4]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--workers", type=int, default=8)
    a = ap.parse_args()
    repos = sorted(p for p in (CACHE / "head").iterdir() if p.is_dir())
    with ThreadPoolExecutor(a.workers) as ex:
        res = list(ex.map(lambda p: check(p, a.runs), repos))
    bad = [r for r in res if r.get("stable") is False]
    out = {"runs": a.runs, "repos": len(res), "unstable": len(bad), "results": bad,
           "errors": [r for r in res if "error" in r]}
    (DATA / "s2_determinism.json").write_text(json.dumps(out, indent=1))
    print(f"{len(res)} repos x {a.runs} runs: {len(bad)} unstable; rules: "
          f"{sorted({x for r in bad for x in r['unstable_rules']})}")


if __name__ == "__main__":
    main()
