"""Study 3, part 2: do configurations on Jazzy branches still use parameters removed in Jazzy?

Takes every parameter labelled `real` (source-verified removal or rename) for humble->jazzy in
data/s3_drift.json and searches the YAML files of the Study 2 HEAD clones (the branch each
repository releases into Jazzy). A hit needs (a) the key on a YAML line and (b) a marker of
the owning node in the same file (controller type string, or the Nav2 server's section key),
so `use_stamped_vel` in an unrelated file does not count.

Usage: python s3_drift/usage.py  -> data/s3_usage.json, data/s3_usage.md
"""

import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import CACHE, DATA  # noqa: E402

MARKERS = {  # package -> regex that must appear in the same YAML file
    "diff_drive_controller": r"diff_drive_controller/DiffDriveController",
    "joint_trajectory_controller": r"joint_trajectory_controller/JointTrajectoryController",
    "mecanum_drive_controller": r"mecanum_drive_controller/MecanumDriveController",
    "steering_controllers_library": r"(ackermann|bicycle|tricycle)_steering_controller/",
    "nav2_controller": r"^\s*controller_server\s*:",
    "nav2_behaviors": r"^\s*behavior_server\s*:",
}


def main():
    drift = json.loads((DATA / "s3_drift.json").read_text())
    targets = []  # (package, node, param, kind, replacement)
    for key in ("packs:humble->jazzy", "ros2_controllers:humble->jazzy"):
        for node, x in drift[key]["changes"].items():
            v = x["verification"]
            pkg = node.split("/")[0]
            for p in x["removed"]:
                if v.get(p) == "real":
                    targets.append((pkg, node, p, "removed", None))
            for a, b, _ in x["renamed"]:
                if v.get(a) == "real":
                    targets.append((pkg, node, a, "renamed", b))
    targets = [t for t in targets if t[0] in MARKERS]
    hits = []
    head = CACHE / "head"
    for repo_dir in sorted(head.iterdir()):
        for f in repo_dir.rglob("*.y*ml"):
            if ".git" in f.parts or f.suffix not in (".yaml", ".yml"):
                continue
            try:
                text = f.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            for pkg, node, p, kind, repl in targets:
                leaf = p.split(".")[-1]
                if not re.search(MARKERS[pkg], text, re.M):
                    continue
                for n, line in enumerate(text.splitlines(), 1):
                    if re.match(rf"^\s*{re.escape(leaf)}\s*:", line) and not line.lstrip().startswith("#"):
                        hits.append({"repo": repo_dir.name.replace("__", "/"),
                                     "file": str(f.relative_to(repo_dir)).replace("\\", "/"), "line": n,
                                     "node": node, "param": p, "kind": kind, "replacement": repl,
                                     "text": line.strip()[:120]})
    repos = sorted({h["repo"] for h in hits})
    (DATA / "s3_usage.json").write_text(json.dumps({"targets": targets, "hits": hits}, indent=1))
    md = ["# Study 3: removed Humble parameters still set on Jazzy branches", "",
          f"Parameters checked (source-verified removals/renames with a node marker): {len(targets)}.",
          f"Repositories scanned: {sum(1 for _ in head.iterdir())}. Hits: {len(hits)} lines in "
          f"{len({(h['repo'], h['file']) for h in hits})} files of {len(repos)} repositories.", "",
          "| Repository | File:line | Node | Parameter | Change |", "|---|---|---|---|---|"]
    for h in hits:
        ch = f"renamed to `{h['replacement']}`" if h["kind"] == "renamed" else "removed"
        md.append(f"| {h['repo']} | `{h['file']}:{h['line']}` | {h['node']} | `{h['param']}` | {ch} |")
    (DATA / "s3_usage.md").write_text("\n".join(md), encoding="utf-8")
    print("\n".join(md[:5]))


if __name__ == "__main__":
    main()
