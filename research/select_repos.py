"""Pick the Study 1 mining set without any API or credentials.

Repositories are ranked by how much ROS 2 configuration they contain, measured on the Study 2
shallow clones (plain `git clone`): launch files + parameter YAML + URDF/xacro + package.xml.
This favours robot bringup/config repositories (where configuration fixes happen) over
general libraries that merely appear in rosdistro (e.g. header-only C++ libraries).

Selection = top-N corpus repositories by this count  ∪  training/repos.txt.
Writes data/config_counts_<distro>.json and data/s1_repos_configrank_<distro>.txt.

The published Study 1 used the frozen list data/s1_repos.txt (chosen once by GitHub star count;
reproducing the study needs only that file). This script is the credential-free way to build a
new mining set, e.g. for another distribution.

Usage: python select_repos.py [--distro jazzy] [--top 185]   (run s2_field/field.py first)
"""

import argparse
import json

from common import CACHE, CONFIG_RE, DATA, REPO


def config_count(path):
    n = 0
    for f in path.rglob("*"):
        if ".git" in f.parts or not f.is_file():
            continue
        rel = f.relative_to(path).as_posix()
        if CONFIG_RE.search(rel) and not rel.endswith(("CMakeLists.txt", "setup.py", "setup.cfg")):
            n += 1
    return n


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--distro", default="jazzy")
    ap.add_argument("--top", type=int, default=185)
    a = ap.parse_args()
    corpus = json.loads((DATA / f"corpus_{a.distro}.json").read_text())["repos"]
    counts = {}
    for r in corpus:
        p = CACHE / "head" / r["repo"].replace("/", "__")
        if p.is_dir():
            counts[r["repo"]] = config_count(p)
    (DATA / f"config_counts_{a.distro}.json").write_text(json.dumps(counts, indent=1))
    ranked = sorted(counts, key=lambda n: (-counts[n], n.lower()))[: a.top]
    extra = [l.strip() for l in (REPO / "training" / "repos.txt").read_text().splitlines()
             if l.strip() and not l.startswith("#")]
    seen, sel = set(), []
    for n in ranked + extra:
        if n.lower() not in seen:
            seen.add(n.lower())
            sel.append(n)
    # data/s1_repos.txt is the frozen list the published Study 1 labels belong to; never overwrite it.
    out = DATA / f"s1_repos_configrank_{a.distro}.txt"
    out.write_text("\n".join(sel) + "\n")
    print(f"config counts for {len(counts)}/{len(corpus)}; selected {len(sel)} -> {out.name}")


if __name__ == "__main__":
    main()
