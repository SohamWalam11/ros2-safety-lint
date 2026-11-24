"""Build the study corpus: every GitHub source repository indexed in a rosdistro distribution.

Usage: python research/corpus.py [--distro jazzy]
Writes research/data/corpus_<distro>.json: [{"name", "repo", "branch", "status"}], sorted.
The rosdistro commit used is recorded so the corpus is reproducible.
"""

import argparse
import json
import urllib.request

import yaml

from common import DATA, run, slug

# No API, no token: the commit comes from `git ls-remote`, the file from a public raw URL.
RAW = "https://raw.githubusercontent.com/ros/rosdistro/{sha}/{distro}/distribution.yaml"


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": "rosfix-research"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def rosdistro_head():
    out = run(["git", "ls-remote", "https://github.com/ros/rosdistro.git", "refs/heads/master"]).stdout
    return out.split()[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--distro", default="jazzy")
    a = ap.parse_args()
    sha = rosdistro_head()
    dist = yaml.safe_load(get(RAW.format(sha=sha, distro=a.distro)))
    rows, seen = [], set()
    for name, entry in dist["repositories"].items():
        src = entry.get("source") or {}
        full = slug(src.get("url", "")) if "github.com" in src.get("url", "") else None
        if not full or full.lower() in seen:
            continue
        seen.add(full.lower())
        rows.append({"name": name, "repo": full, "branch": src.get("version"),
                     "status": entry.get("status", "unknown")})
    rows.sort(key=lambda r: r["repo"].lower())
    DATA.mkdir(exist_ok=True)
    out = DATA / f"corpus_{a.distro}.json"
    out.write_text(json.dumps({"distro": a.distro, "rosdistro_commit": sha, "repos": rows},
                              indent=1), encoding="utf-8")
    print(f"{len(rows)} repositories -> {out}")


if __name__ == "__main__":
    main()
