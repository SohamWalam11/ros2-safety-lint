"""Study 1, stage 3: combine blind labels with the rosfix replay and compute the results.

Population P = all candidate commits. Set A = every candidate where some rosfix finding
disappeared (fully labelled). Set B = uniform random sample of P \\ A (labelled).

  genuine        : rater-1 label misconfig_fix == "yes"
  related        : a disappeared finding lies in a file the commit touched, or in the same
                   package directory as a touched file (automatic proxy for "the finding is the
                   bug that was fixed"); `verified` overrides it when data/s1_labels/verify.json
                   exists (manual check of every genuine detection)
  detected       : genuine and related (Set A only, by construction)
  precision      : of Set A candidates with a related finding, the share that are genuine fixes
  recall         : detected / estimated genuine fixes in P, where genuine fixes in P \\ A are
                   estimated as (genuine rate in B) x |P \\ A|; 95% CI by bootstrap over B
Reported overall, for statically detectable fixes (`static == yes`), and per category.
Agreement: Cohen's kappa between rater 1 and rater 2 on misconfig_fix (yes vs not yes).

Usage: python s1_mined/score.py   -> data/s1_results.json, data/s1_results.md
"""

import collections
import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import DATA, load_jsonl  # noqa: E402

LAB = DATA / "s1_labels"


def load_labels(pattern):
    out = {}
    for f in sorted(LAB.glob(pattern)):
        for x in json.loads(f.read_text(encoding="utf-8")):
            out[x["id"]] = x
    return out


def pkg_dir(path):
    parts = (path or "").split("/")
    return parts[0] if len(parts) > 1 else ""


def related(c, key):
    touched = set(c["files"])
    pkgs = {pkg_dir(f) for f in touched}
    return [f for f in c[key] if f["file"] in touched or (pkg_dir(f["file"]) and pkg_dir(f["file"]) in pkgs)]


def kappa(pairs):
    n = len(pairs)
    if not n:
        return None
    po = sum(a == b for a, b in pairs) / n
    ca, cb = collections.Counter(a for a, _ in pairs), collections.Counter(b for _, b in pairs)
    pe = sum(ca[k] * cb[k] for k in set(ca) | set(cb)) / n / n
    return (po - pe) / (1 - pe) if pe < 1 else 1.0


def main():
    cands = {r["id"]: r for r in load_jsonl(DATA / "s1_candidates.jsonl")}
    sample = json.loads((LAB / "sample.json").read_text())
    r1, r2 = load_labels("labels_batch_*.json"), load_labels("labels_r2_*.json")
    verify = json.loads((LAB / "verify.json").read_text()) if (LAB / "verify.json").exists() else {}
    A = [i for i in sample["set_a"] if i in r1]
    B = [i for i in sample["set_b"] if i in r1]
    n_rest = sample["rest"]
    gen = lambda i: r1[i]["misconfig_fix"] == "yes"  # noqa: E731
    stat = lambda i: r1[i].get("static") == "yes"  # noqa: E731

    def is_detected(i, key="disappeared"):
        if not gen(i):
            return False
        if key == "disappeared" and i in verify:
            return bool(verify[i]["is_the_bug"])
        return bool(related(cands[i], key))

    def recall(filter_fn, key="disappeared", boot=2000):
        a_gen = [i for i in A if gen(i) and filter_fn(i)]
        det = sum(is_detected(i, key) for i in a_gen)
        b_rate = (sum(gen(i) and filter_fn(i) for i in B) / len(B)) if B else 0.0
        est = len(a_gen) + b_rate * n_rest
        rng = random.Random(7)
        bs = []
        for _ in range(boot):
            s = [rng.choice(B) for _ in B]
            e = len(a_gen) + sum(gen(i) and filter_fn(i) for i in s) / len(s) * n_rest
            bs.append(det / e if e else 0.0)
        bs.sort()
        return {"detected": det, "genuine_in_A": len(a_gen), "genuine_rate_B": round(b_rate, 4),
                "est_genuine_total": round(est, 1), "recall": round(det / est, 4) if est else None,
                "ci95": [round(bs[int(0.025 * boot)], 4), round(bs[int(0.975 * boot)], 4)]}

    flagged_rel = [i for i in A if related(cands[i], "disappeared")]
    prec = sum(gen(i) for i in flagged_rel) / len(flagged_rel) if flagged_rel else None
    per_rule = collections.defaultdict(lambda: [0, 0])
    for i in flagged_rel:
        for f in related(cands[i], "disappeared"):
            per_rule[f["rule"]][0] += 1
            per_rule[f["rule"]][1] += gen(i)

    # weighted category distribution of genuine fixes in P (A exact, B scaled)
    w_b = n_rest / len(B) if B else 0
    cat = collections.Counter()
    fail = collections.Counter()
    for i in A:
        if gen(i):
            cat[r1[i]["category"]] += 1
            fail[r1[i]["failure"]] += 1
    for i in B:
        if gen(i):
            cat[r1[i]["category"]] += w_b
            fail[r1[i]["failure"]] += w_b
    det_cat = collections.Counter(r1[i]["category"] for i in A if is_detected(i))

    both = [i for i in r2 if i in r1]
    yes = lambda x: "yes" if x["misconfig_fix"] == "yes" else "no"  # noqa: E731
    res = {
        "population": len(cands), "set_A": len(sample["set_a"]), "set_B": len(sample["set_b"]),
        "labelled_A": len(A), "labelled_B": len(B),
        "agreement": {"n": len(both), "kappa_misconfig_fix": kappa([(yes(r1[i]), yes(r2[i])) for i in both]),
                      "raw_agreement": round(sum(yes(r1[i]) == yes(r2[i]) for i in both) / len(both), 4) if both else None,
                      "kappa_category_on_agreed_yes": kappa([(r1[i]["category"], r2[i]["category"]) for i in both
                                                             if yes(r1[i]) == yes(r2[i]) == "yes"])},
        "precision_flagged_related": {"n": len(flagged_rel), "genuine": sum(gen(i) for i in flagged_rel),
                                      "precision": round(prec, 4) if prec is not None else None},
        "per_rule_flagged": {k: {"flagged": v[0], "genuine": v[1]} for k, v in sorted(per_rule.items())},
        "recall_all": recall(lambda i: True),
        "recall_static": recall(stat),
        # rosfix does not model CMake build logic: commits touching only CMakeLists.txt are out of scope
        "recall_excl_cmake_only": recall(lambda i: not all(f.endswith("CMakeLists.txt") for f in cands[i]["files"])),
        "recall_silent": recall(lambda i: r1[i].get("failure") == "silent"),
        "recall_by_category": {c: recall(lambda i, c=c: r1[i]["category"] == c, boot=500)
                               for c in sorted({r1[i]["category"] for i in A + B if gen(i)})},
        "recall_all_nopack": recall(lambda i: True, "disappeared_nopack"),
        "recall_static_nopack": recall(stat, "disappeared_nopack"),
        "genuine_by_category_weighted": {k: round(v, 1) for k, v in cat.most_common()},
        "genuine_by_failure_weighted": {k: round(v, 1) for k, v in fail.most_common()},
        "detected_by_category": dict(det_cat.most_common()),
        "verified_detections": len(verify),
    }
    (DATA / "s1_results.json").write_text(json.dumps(res, indent=1))

    def pct(x):
        if not x or x["recall"] is None:
            return "n/a"
        return f"{x['recall']:.1%} [{x['ci95'][0]:.1%}, {x['ci95'][1]:.1%}]"

    def rr(x):
        return f"{x['recall']:.1%} [{x['ci95'][0]:.1%}, {x['ci95'][1]:.1%}] ({x['detected']} / ~{x['est_genuine_total']:.0f})"

    md = ["# Study 1: recall on real configuration fixes", "",
          f"Population: {res['population']} candidate commits; Set A (flagged) {res['set_A']}, Set B (random) {res['set_B']}.",
          f"Inter-rater agreement on `misconfig_fix` (n={res['agreement']['n']}): kappa = "
          f"{res['agreement']['kappa_misconfig_fix'] and round(res['agreement']['kappa_misconfig_fix'], 3)}, raw = {res['agreement']['raw_agreement']}.",
          "", "| Metric | Value |", "|---|---|",
          f"| Precision of flagged commits (finding in touched package disappeared) | {res['precision_flagged_related']['genuine']}/{res['precision_flagged_related']['n']} = {prec and f'{prec:.1%}'} |",
          f"| Recall, all genuine fixes | {rr(res['recall_all'])} |",
          f"| Recall, statically detectable fixes | {rr(res['recall_static'])} |",
          f"| Recall, excluding CMakeLists-only commits | {rr(res['recall_excl_cmake_only'])} |",
          f"| Recall, silent failures only | {rr(res['recall_silent'])} |",
          f"| Recall without knowledge packs, all | {rr(res['recall_all_nopack'])} |",
          f"| Recall without knowledge packs, static | {rr(res['recall_static_nopack'])} |",
          "", "## Genuine fixes by ISSTA category (weighted estimate for the population) and detections", "",
          "| Category | Est. genuine fixes | Detected | Recall [95% CI] |", "|---|---:|---:|---|"]
    for k, v in cat.most_common():
        rc = res["recall_by_category"].get(k)
        md.append(f"| {k} | {v:.0f} | {det_cat.get(k, 0)} | {pct(rc)} |")
    md += ["", "## How the pre-fix mistake shows up (weighted)", "", "| Failure | Est. count |", "|---|---:|"]
    md += [f"| {k} | {v:.0f} |" for k, v in fail.most_common()]
    md += ["", "## Flagged commits per rule", "", "| Rule | Flagged | Genuine fix |", "|---|---:|---:|"]
    md += [f"| {k} | {v['flagged']} | {v['genuine']} |" for k, v in res["per_rule_flagged"].items()]
    (DATA / "s1_results.md").write_text("\n".join(md), encoding="utf-8")
    print("\n".join(md))


if __name__ == "__main__":
    main()
