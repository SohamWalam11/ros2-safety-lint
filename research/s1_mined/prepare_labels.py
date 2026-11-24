"""Study 1, stage 2: select the items to label and write blind labelling batches.

Set A = every candidate where a rosfix finding disappeared (any configuration, packs or not).
Set B = seeded random sample of the remaining candidates.
A second rater independently labels a seeded 20% subset of A ∪ B (agreement / Cohen's kappa).

Batches contain only what a human reviewer would see on GitHub: repo, message, files, diff.
No rosfix output. Writes data/s1_labels/sample.json and data/s1_labels/batch_NN.jsonl.

Usage: python s1_mined/prepare_labels.py [--sample 300] [--batch 47] [--seed 20261120]
"""

import argparse
import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import DATA, load_jsonl  # noqa: E402

MAX_DIFF = 2500


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sample", type=int, default=300)
    ap.add_argument("--batch", type=int, default=47)
    ap.add_argument("--seed", type=int, default=20261120)
    a = ap.parse_args()
    rows = load_jsonl(DATA / "s1_candidates.jsonl")
    flagged = [r for r in rows if r["disappeared"] or r["disappeared_nopack"]]
    rest = [r for r in rows if not (r["disappeared"] or r["disappeared_nopack"])]
    rng = random.Random(a.seed)
    set_a = [r["id"] for r in flagged]
    set_b = [r["id"] for r in rng.sample(rest, min(a.sample, len(rest)))]
    items = set_a + set_b
    rng.shuffle(items)  # raters cannot tell set A from set B by position
    second = sorted(rng.sample(items, len(items) // 5))
    out = DATA / "s1_labels"
    out.mkdir(exist_ok=True)
    (out / "sample.json").write_text(json.dumps({
        "seed": a.seed, "population": len(rows), "flagged": len(flagged), "rest": len(rest),
        "set_a": set_a, "set_b": set_b, "second_rater": second}, indent=1))
    by_id = {r["id"]: r for r in rows}

    def blind(i):
        r = by_id[i]
        d = r["diff"]
        return {"id": i, "repo": r["repo"], "files": r["files"],
                "diff": d[:MAX_DIFF] + ("\n[... diff truncated ...]" if len(d) > MAX_DIFF or r["diff_truncated"] else "")}

    n = 0
    for k in range(0, len(items), a.batch):
        (out / f"batch_{n:02d}.jsonl").write_text(
            "\n".join(json.dumps(blind(i), ensure_ascii=False) for i in items[k:k + a.batch]) + "\n",
            encoding="utf-8")
        n += 1
    for k in range(0, len(second), a.batch):
        (out / f"batch_r2_{k // a.batch:02d}.jsonl").write_text(
            "\n".join(json.dumps(blind(i), ensure_ascii=False) for i in second[k:k + a.batch]) + "\n",
            encoding="utf-8")
    print(f"A={len(set_a)} B={len(set_b)} items={len(items)} batches={n} second-rater={len(second)}")


if __name__ == "__main__":
    main()
