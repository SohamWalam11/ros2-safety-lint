"""Study 3: cross-distribution parameter drift (Humble -> Jazzy -> Kilted).

Two sources of "declared parameters per node, per distribution":
  1. rosfix's knowledge packs (nodes of Nav2, slam_toolbox, robot_localization, ...);
  2. generate_parameter_library definition files of ros-controls/ros2_controllers at each
     distribution branch (controllers are plugins, so they are not in the packs).

For each node present in two consecutive distributions we report parameters removed, added,
type-changed, and likely renames (a removed and an added name of the same node with high
string similarity or the same last segment). Output: data/s3_drift.json, data/s3_drift.md.

Usage: python s3_drift/drift.py
"""

import difflib
import gzip
import json
import sys
from pathlib import Path

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import CACHE, DATA, REPO, run  # noqa: E402

DISTROS = ["humble", "jazzy", "kilted"]
CONTROLLER_BRANCH = {"humble": "humble", "jazzy": "jazzy", "kilted": "kilted"}


def pack_params():
    """{distro: {"pkg/node": {param: type}}} from the knowledge packs (executables only)."""
    out = {}
    for d in DISTROS:
        p = json.load(gzip.open(REPO / "crates/ros2-safety-lint/packs" / f"{d}.json.gz"))
        nodes = {}
        for pkg, v in p["packages"].items():
            for exe, e in v.get("executables", {}).items():
                ps = {}
                for q in e["params"]:
                    (kind, name), = q["name"].items()
                    ps[name if kind == "Exact" else f"*{name}"] = q["ty"]
                if ps:
                    nodes[f"{pkg}/{exe}"] = ps
        out[d] = nodes
    return out


def flatten_gpl(tree, prefix=""):
    """Flatten a generate_parameter_library definition into {dotted.name: type}."""
    out = {}
    if not isinstance(tree, dict):
        return out
    for k, v in tree.items():
        if not isinstance(v, dict):
            continue
        seg = f"<{k[6:]}>" if k.startswith("__map_") else k  # __map_joints -> <joints>
        name = f"{prefix}.{seg}" if prefix else seg
        if "type" in v and not isinstance(v["type"], dict):
            out[name] = str(v["type"])
        else:
            out.update(flatten_gpl(v, name))
    return out


def controller_params():
    out = {}
    for d in DISTROS:
        dest = CACHE / "s3" / f"ros2_controllers-{d}"
        if not dest.exists():
            dest.parent.mkdir(parents=True, exist_ok=True)
            run(["git", "clone", "--quiet", "--depth", "1", "--branch", CONTROLLER_BRANCH[d],
                 "https://github.com/ros-controls/ros2_controllers.git", str(dest)], timeout=600)
        nodes = {}
        for f in dest.rglob("*.yaml"):
            if "test" in f.parts or ".git" in f.parts:
                continue
            try:
                doc = yaml.safe_load(f.read_text(encoding="utf-8"))
            except Exception:
                continue
            if not isinstance(doc, dict) or len(doc) != 1:
                continue
            (ns, body), = doc.items()
            flat = flatten_gpl(body)
            if len(flat) >= 2:  # a gpl file: every leaf has a `type`
                pkg = f.relative_to(dest).parts[0]
                nodes[f"{pkg}/{ns}"] = flat
        out[d] = nodes
    return out


def compare(a, b):
    rem, add = sorted(set(a) - set(b)), sorted(set(b) - set(a))
    retyped = sorted(k for k in set(a) & set(b) if a[k] != b[k] and "Unknown" not in (a[k], b[k]))
    renames, used = [], set()
    for r in rem:
        best, score = None, 0.0
        for n in add:
            if n in used:
                continue
            s = difflib.SequenceMatcher(None, r, n).ratio()
            if r.split(".")[-1] == n.split(".")[-1] and r != n:
                s = max(s, 0.9)
            if s > score:
                best, score = n, s
        if best and score >= 0.8:
            renames.append([r, best, round(score, 2)])
            used.add(best)
    renamed_from = {x[0] for x in renames}
    return {"removed": [r for r in rem if r not in renamed_from],
            "added": [n for n in add if n not in used], "renamed": renames,
            "retyped": [[k, a[k], b[k]] for k in retyped], "n_before": len(a), "n_after": len(b)}


PACK_REPOS = [  # mirrors scripts/build_packs.sh (kilted uses the real kilted branches when present)
    ("ros-navigation/navigation2", "humble", "jazzy", "kilted"),
    ("ros-controls/ros2_controllers", "humble", "jazzy", "kilted"),
    ("ros-controls/ros2_control", "humble", "jazzy", "kilted"),
    ("ros/robot_state_publisher", "humble", "jazzy", "kilted"),
    ("ros/joint_state_publisher", "ros2", "ros2", "ros2"),
    ("ros2/geometry2", "humble", "jazzy", "kilted"),
    ("ros2/demos", "humble", "jazzy", "kilted"),
    ("SteveMacenski/slam_toolbox", "humble", "jazzy", "jazzy"),
    ("cra-ros-pkg/robot_localization", "humble-devel", "jazzy-devel", "jazzy-devel"),
]
SRC_EXT = {".cpp", ".hpp", ".h", ".cc", ".py", ".yaml"}


def source_text(distro):
    """{package: (non-test source text, test-only source text)} of the pack repos at `distro`.

    Packages are found by package.xml; a file belongs to the deepest package containing it.
    """
    import re
    idx = DISTROS.index(distro)
    texts = {}
    for repo, *branches in PACK_REPOS:
        dest = CACHE / "s3src" / distro / repo.split("/")[1]
        if not dest.exists():
            dest.parent.mkdir(parents=True, exist_ok=True)
            run(["git", "clone", "--quiet", "--depth", "1", "--branch", branches[idx],
                 f"https://github.com/{repo}.git", str(dest)], timeout=900)
        pkgs = {}
        for px in dest.rglob("package.xml"):
            m = re.search(r"<name>\s*([^<\s]+)\s*</name>", px.read_text(encoding="utf-8", errors="replace"))
            if m:
                pkgs[px.parent] = m.group(1)
        for f in dest.rglob("*"):
            if f.suffix not in SRC_EXT or ".git" in f.parts or not f.is_file():
                continue
            owner = next((pkgs[p] for p in [f.parent, *f.parents] if p in pkgs), None)
            if not owner:
                continue
            t = f.read_text(encoding="utf-8", errors="replace")
            main, test = texts.setdefault(owner, ([], []))
            (test if any(p in ("test", "tests") for p in f.relative_to(dest).parts) else main).append(t)
    return {k: ("\n".join(a), "\n".join(b)) for k, (a, b) in texts.items()}


def verify(report):
    """Label every removed / renamed-from parameter with source evidence.

    real          : literal present in the older non-test source, absent from the newer one
    test_artifact : literal only in the older *test* sources (extraction picked up tests)
    still_present : literal still in the newer non-test source (extraction missed it; not a removal)
    unknown       : literal not found anywhere (built dynamically; cannot confirm)
    Controller parameters come from gpl definition files and are checked the same way.
    """
    texts = {d: source_text(d) for d in DISTROS}
    for key, r in report.items():
        d0, d1 = key.split(":")[1].split("->")
        for node, x in r["changes"].items():
            pkg = node.split("/")[0]
            old_main, old_test = texts[d0].get(pkg, ("", ""))
            new_main = texts[d1].get(pkg, ("", ""))[0]
            # Nav2 nodes inherit parameters declared in shared libraries (nav2_util, nav2_core,
            # nav2_costmap_2d): search those packages too.
            for lib in ("nav2_util", "nav2_core", "nav2_costmap_2d", "nav2_ros_common"):
                if pkg.startswith(("nav2_", "opennav_")) and lib != pkg:
                    old_main += texts[d0].get(lib, ("", ""))[0]
                    old_test += texts[d0].get(lib, ("", ""))[1]
                    new_main += texts[d1].get(lib, ("", ""))[0]
            labels = {}
            for name in x["removed"] + [a for a, _, _ in x["renamed"]]:
                leaf = name.lstrip("*").lstrip(".").lstrip("/")
                if "<" in leaf:  # map parameter: check the last fixed segment
                    leaf = leaf.split(">")[-1].lstrip(".") or leaf
                last = leaf.split(".")[-1]  # gpl definitions nest dotted names as YAML maps
                lit = [f'"{leaf}"', f"'{leaf}'", f"{leaf}:"]
                if "." in leaf and key.startswith("ros2_controllers"):
                    lit.append(f"{last}:")
                in_new = any(l in new_main for l in lit)
                in_old = any(l in old_main for l in lit)
                in_test = any(l in old_test for l in lit)
                labels[name] = ("still_present" if in_new else "real" if in_old
                                else "test_artifact" if in_test else "unknown")
            x["verification"] = labels
    return report


def main():
    sources = {"packs": pack_params(), "ros2_controllers": controller_params()}
    report = {}
    for src, per in sources.items():
        for d0, d1 in zip(DISTROS, DISTROS[1:]):
            common_nodes = sorted(set(per[d0]) & set(per[d1]))
            rows = {n: compare(per[d0][n], per[d1][n]) for n in common_nodes}
            rows = {n: r for n, r in rows.items() if r["removed"] or r["renamed"] or r["retyped"] or r["added"]}
            report[f"{src}:{d0}->{d1}"] = {
                "nodes_compared": len(common_nodes),
                "nodes_only_before": sorted(set(per[d0]) - set(per[d1])),
                "nodes_only_after": sorted(set(per[d1]) - set(per[d0])),
                "params_before": sum(len(per[d0][n]) for n in common_nodes),
                "changes": rows,
            }
    report = verify(report)
    (DATA / "s3_drift.json").write_text(json.dumps(report, indent=1), encoding="utf-8")

    def count(c, lab):
        return sum(1 for x in c for v in x["verification"].values() if v == lab)

    md = ["# Study 3: parameter drift between distributions", "",
          "Raw = pack/definition diff. Removed and renamed names are then checked against upstream",
          "source (see `verify`): only `real` ones are breaking changes.", "",
          "| Source | Transition | Nodes | Params before | Removed+renamed (raw) | real | still present | test artifact | unknown | Retyped | Added |",
          "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for key, r in report.items():
        src, tr = key.split(":")
        c = list(r["changes"].values())
        raw = sum(len(x["removed"]) + len(x["renamed"]) for x in c)
        md.append(f"| {src} | {tr} | {r['nodes_compared']} | {r['params_before']} | {raw} | "
                  f"{count(c, 'real')} | {count(c, 'still_present')} | {count(c, 'test_artifact')} | "
                  f"{count(c, 'unknown')} | {sum(len(x['retyped']) for x in c)} | "
                  f"{sum(len(x['added']) for x in c)} |")
    md += ["", "## Breaking changes per node (removed / renamed / retyped)", ""]
    for key, r in report.items():
        for n, x in r["changes"].items():
            if not (x["removed"] or x["renamed"] or x["retyped"]):
                continue
            v = x["verification"]
            if not any(v[a] == "real" for a in v) and not x["retyped"]:
                continue
            md.append(f"### {key} `{n}`")
            for a, b, s in x["renamed"]:
                if v[a] == "real":
                    md.append(f"- renamed `{a}` -> `{b}` (similarity {s})")
            for a in x["removed"]:
                if v[a] == "real":
                    md.append(f"- removed `{a}`")
            for a, t0, t1 in x["retyped"]:
                md.append(f"- type `{a}`: {t0} -> {t1}")
            md.append("")
    (DATA / "s3_drift.md").write_text("\n".join(md), encoding="utf-8")
    print("\n".join(md[:12]))


if __name__ == "__main__":
    main()
