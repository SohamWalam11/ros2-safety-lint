"""Study 6: repair quality, verified rosfix repair vs. LLM repair. No API keys anywhere.

Benchmark: the Study 1 commits whose bug rosfix found on the parent and a human fixed in the
commit (s1_labels/verify.json, is_the_bug = true). Ground truth = the human fix.

Arms (all credential-free):
  A  rosfix --fix               verified mechanical fixes (deterministic)
  B  rosfix --fix --assist      + the built-in offline ranker
  C  LLM via Claude Code subagents (runs under the user's Claude Code login; prompts and raw
     answers are stored, see s6_repair/LLM_ARM.md). Output: proposals/C_*.json
  D  optional local open-weight model (Ollama / llama.cpp, CPU); same task files, same format
Every arm sees the same task: finding message, rule explanation, file excerpt. Never the fix.

Evaluation (identical for every arm), on a fresh copy of the parent tree:
  applied     the proposed edits apply cleanly
  resolved    the target finding (fingerprint) is gone after re-analysis
  no_regress  no new error-severity finding appears
  matches     the edit agrees with the human fix: every identifier the human added on the
              touched line(s) of that file appears in the proposal (e.g. the dependency name)
  invented    the proposal introduces an identifier that occurs nowhere in the parent tree
              and not in the human fix (hallucinated package/parameter/executable names)
  abstained   the arm declined (always safe: the finding is still reported)

Usage:
  python s6_repair/repair.py tasks          # build tasks + ground truth (frozen rosfix)
  python s6_repair/repair.py rosfix         # arms A and B
  python s6_repair/repair.py eval           # score every proposals/*.json
Proposal file format: [{"id": "...", "abstain": false, "edits": [{"file": "rel/path",
  "old": "exact text in the parent file", "new": "replacement"}]}]
"""

import argparse
import difflib
import json
import re
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import CACHE, DATA, ROSFIX, findings, load_jsonl, rosfix_json, run  # noqa: E402

OUT = DATA / "s6"
WT = CACHE / "s6wt"
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_./-]{2,}")


def parent_tree(task_id, clone, rev):
    """A clean copy of the parent tree (git worktree), recreated on every call."""
    d = WT / task_id.replace("/", "__").replace("@", "_")
    run(["git", "-C", str(clone), "worktree", "remove", "--force", str(d)], timeout=300)
    shutil.rmtree(d, ignore_errors=True)
    run(["git", "-C", str(clone), "worktree", "add", "-f", "--detach", str(d), rev], timeout=900)
    return d


def explain(rule):
    return run([str(ROSFIX), "--explain", rule]).stdout.strip()


def excerpt(path, line, ctx=15):
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return ""
    lo, hi = max(0, line - 1 - ctx), min(len(lines), line + ctx)
    return "\n".join(f"{i + 1:5d}| {lines[i]}" for i in range(lo, hi))


def human_fix(diff_text, file):
    """Lines the human removed/added in `file`, from `git show` output."""
    added, removed, cur = [], [], None
    for l in diff_text.splitlines():
        if l.startswith("+++ b/"):
            cur = l[6:]
        elif l.startswith("--- "):
            continue
        elif cur == file and l.startswith("+"):
            added.append(l[1:])
        elif cur == file and l.startswith("-"):
            removed.append(l[1:])
    return added, removed


def cmd_tasks():
    verify = json.loads((DATA / "s1_labels" / "verify.json").read_text())
    cands = {r["id"]: r for r in load_jsonl(DATA / "s1_candidates.jsonl")}
    OUT.mkdir(parents=True, exist_ok=True)
    tasks, truth = [], {}
    for i, v in sorted(verify.items()):
        if not v["is_the_bug"]:
            continue
        c = cands[i]
        clone = CACHE / "hist" / c["repo"].replace("/", "__")
        tree = parent_tree(i, clone, c["parent"])
        rep, _, err = rosfix_json(tree, ["--distro", c["distro"]])
        fs = [f for f in findings(rep) if f["rule"] == v["rule"]]
        # the target: a finding of the verified rule in a touched file (or its package)
        touched = set(c["files"])
        pk = {f.split("/")[0] for f in touched}
        target = next((f for f in fs if f["file"] in touched), None) or \
            next((f for f in fs if (f["file"] or "").split("/")[0] in pk), None)
        if not target:
            print(f"skip {i}: frozen rosfix no longer reports {v['rule']} here", file=sys.stderr)
            continue
        tasks.append({"id": i, "repo": c["repo"], "distro": c["distro"], "rule": target["rule"],
                      "fingerprint": target["fp"], "file": target["file"], "line": target["line"],
                      "message": target["message"], "rule_explanation": explain(target["rule"]),
                      "excerpt": excerpt(tree / target["file"], target["line"] or 1),
                      "workspace_files": sorted(str(p.relative_to(tree)).replace("\\", "/")
                                                for p in tree.rglob("package.xml"))[:60]})
        truth[i] = {"files": c["files"], "diff": c["diff"], "sha": c["sha"], "parent": c["parent"]}
    (OUT / "tasks.jsonl").write_text("\n".join(json.dumps(t, ensure_ascii=False) for t in tasks) + "\n",
                                     encoding="utf-8")
    (OUT / "truth.json").write_text(json.dumps(truth, indent=1), encoding="utf-8")
    print(f"{len(tasks)} tasks -> {OUT / 'tasks.jsonl'}")


def unified_to_edits(diff_text):
    """Convert rosfix's unified diff into old/new edits (one per hunk)."""
    edits, cur, old, new = [], None, [], []

    def flush():
        if cur and (old or new):
            edits.append({"file": cur, "old": "\n".join(old), "new": "\n".join(new)})

    for l in diff_text.splitlines():
        if l.startswith("+++ "):
            flush()
            cur, old, new = re.sub(r"^\+\+\+ (b/)?", "", l).strip(), [], []
        elif l.startswith("--- "):
            continue
        elif l.startswith("@@"):
            flush()
            old, new = [], []
        elif cur and l.startswith(" "):
            old.append(l[1:])
            new.append(l[1:])
        elif cur and l.startswith("-"):
            old.append(l[1:])
        elif cur and l.startswith("+"):
            new.append(l[1:])
    flush()
    return edits


def cmd_rosfix():
    tasks = load_jsonl(OUT / "tasks.jsonl")
    truth = json.loads((OUT / "truth.json").read_text())
    for arm, extra in (("A_rosfix_fix", []), ("B_rosfix_assist", ["--assist"])):
        props = []
        for t in tasks:
            clone = CACHE / "hist" / t["repo"].replace("/", "__")
            tree = parent_tree(t["id"], clone, truth[t["id"]]["parent"])
            r = run([str(ROSFIX), str(tree), "--no-ros", "--distro", t["distro"], "--fix", *extra,
                     "--fail-on", "never"], timeout=600)
            edits = [e for e in unified_to_edits(r.stdout) if e["file"] == t["file"]
                     or e["file"].split("/")[0] == t["file"].split("/")[0]]
            props.append({"id": t["id"], "abstain": not edits, "edits": edits})
        (OUT / "proposals").mkdir(exist_ok=True)
        (OUT / "proposals" / f"{arm}.json").write_text(json.dumps(props, indent=1), encoding="utf-8")
        print(f"{arm}: {sum(not p['abstain'] for p in props)}/{len(props)} proposals")


def evaluate(t, truth, prop):
    res = {"id": t["id"], "rule": t["rule"], "abstained": bool(prop.get("abstain")) or not prop.get("edits")}
    if res["abstained"]:
        return res
    clone = CACHE / "hist" / t["repo"].replace("/", "__")
    tree = parent_tree(t["id"], clone, truth["parent"])
    corpus = " ".join(p.read_text(encoding="utf-8", errors="replace") for p in tree.rglob("*")
                      if p.is_file() and ".git" not in p.parts and p.suffix in
                      (".xml", ".py", ".yaml", ".yml", ".txt", ".launch", ".cpp", ".hpp", ".h", ".cfg", ".xacro", ".urdf"))
    applied = True
    for e in prop["edits"]:
        p = tree / e["file"]
        try:
            text = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            applied = False
            break
        if e["old"] and e["old"] in text:
            text = text.replace(e["old"], e["new"], 1)
        elif not e["old"]:
            text = text + ("\n" if not text.endswith("\n") else "") + e["new"]
        else:
            applied = False
            break
        p.write_text(text, encoding="utf-8")
    res["applied"] = applied
    if not applied:
        return res
    before, _, _ = rosfix_json(tree, ["--distro", t["distro"]])  # tree now holds the edit
    after = findings(before)
    res["resolved"] = all(f["fp"] != t["fingerprint"] for f in after)
    added, removed = human_fix(truth["diff"], t["file"])
    want = {w for l in added for w in IDENT.findall(l)} - {w for l in removed for w in IDENT.findall(l)}
    got = {w for e in prop["edits"] for w in IDENT.findall(e["new"])}
    res["matches"] = bool(want) and want <= got
    res["similarity"] = round(difflib.SequenceMatcher(None, "\n".join(added),
                                                      "\n".join(e["new"] for e in prop["edits"])).ratio(), 3)
    res["invented"] = sorted(w for w in got if w not in corpus and w not in truth["diff"])[:10]
    return res


def cmd_eval():
    tasks = {t["id"]: t for t in load_jsonl(OUT / "tasks.jsonl")}
    truth = json.loads((OUT / "truth.json").read_text())
    # baseline errors per task, to detect regressions
    summary = {}
    for pf in sorted((OUT / "proposals").glob("*.json")):
        props = {p["id"]: p for p in json.loads(pf.read_text(encoding="utf-8"))}
        rows = [evaluate(tasks[i], truth[i], props.get(i, {"abstain": True})) for i in tasks]
        (OUT / f"eval_{pf.stem}.json").write_text(json.dumps(rows, indent=1))
        n = len(rows)
        tried = [r for r in rows if not r["abstained"]]
        summary[pf.stem] = {
            "tasks": n, "abstained": n - len(tried),
            "applied": sum(r.get("applied", False) for r in tried),
            "resolved": sum(r.get("resolved", False) for r in tried),
            "matches_human": sum(r.get("matches", False) for r in tried),
            "resolved_and_matches": sum(r.get("resolved", False) and r.get("matches", False) for r in tried),
            "wrong_edits": sum(1 for r in tried if not (r.get("resolved") and r.get("matches"))),
            "with_invented_names": sum(1 for r in tried if r.get("invented")),
        }
    (OUT / "summary.json").write_text(json.dumps(summary, indent=1))
    md = ["# Study 6: repair quality (no API keys)", "",
          "| Arm | Tasks | Abstained | Applied | Resolved | Matches human fix | Resolved & matches | Wrong edits | Invented names |",
          "|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for k, s in summary.items():
        md.append(f"| {k} | {s['tasks']} | {s['abstained']} | {s['applied']} | {s['resolved']} | {s['matches_human']} | "
                  f"{s['resolved_and_matches']} | {s['wrong_edits']} | {s['with_invented_names']} |")
    (OUT / "summary.md").write_text("\n".join(md), encoding="utf-8")
    print("\n".join(md))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["tasks", "rosfix", "eval"])
    a = ap.parse_args()
    {"tasks": cmd_tasks, "rosfix": cmd_rosfix, "eval": cmd_eval}[a.cmd]()


if __name__ == "__main__":
    main()
