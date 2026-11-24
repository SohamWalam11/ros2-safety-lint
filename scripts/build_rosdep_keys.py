#!/usr/bin/env python3
"""Builds the embedded rosdep key lists (crates/ros2-safety-lint/packs/keys/<distro>.txt.gz).

A key is valid for `rosdep install` if it is a package released in the distribution
(rosdistro <distro>/distribution.yaml) or a system dependency key in rosdep's base, python
or ruby files. Run once per release cycle; needs network access and PyYAML.

Usage: python scripts/build_rosdep_keys.py [--rosdistro <local clone>]
"""

import argparse
import gzip
import io
import pathlib
import urllib.request

import yaml

RAW = "https://raw.githubusercontent.com/ros/rosdistro/master/"
DISTROS = ["humble", "jazzy", "kilted"]
SYSTEM_FILES = ["rosdep/base.yaml", "rosdep/python.yaml", "rosdep/ruby.yaml"]


def load(base, rel):
    if base:
        return yaml.safe_load((pathlib.Path(base) / rel).read_text(encoding="utf-8"))
    with urllib.request.urlopen(RAW + rel, timeout=60) as r:
        return yaml.safe_load(io.TextIOWrapper(r, encoding="utf-8"))


def released_packages(dist):
    names = set()
    for repo, entry in (dist.get("repositories") or {}).items():
        rel = (entry or {}).get("release")
        if not rel:
            continue
        pkgs = rel.get("packages")
        names.update(pkgs if pkgs else [repo])
    return names


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rosdistro", help="local clone of ros/rosdistro (default: fetch from GitHub)")
    args = ap.parse_args()
    system = set()
    for f in SYSTEM_FILES:
        system.update((load(args.rosdistro, f) or {}).keys())
    out_dir = pathlib.Path(__file__).resolve().parent.parent / "crates/ros2-safety-lint/packs/keys"
    out_dir.mkdir(parents=True, exist_ok=True)
    for d in DISTROS:
        keys = sorted(released_packages(load(args.rosdistro, f"{d}/distribution.yaml")) | system)
        data = ("\n".join(keys) + "\n").encode()
        (out_dir / f"{d}.txt.gz").write_bytes(gzip.compress(data, mtime=0))
        print(f"{d}: {len(keys)} keys ({len(data) // 1024} KiB raw)")


if __name__ == "__main__":
    main()
