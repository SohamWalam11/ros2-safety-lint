#!/usr/bin/env bash
# Rebuilds the embedded knowledge packs from upstream source at each distro's branch.
# Run rarely (once per ROS distro cycle); the result is committed under
# crates/ros2-safety-lint/packs/. Requires git and a release build of rosfix-pack.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${WORK:-$ROOT/.packs-src}"
PACK_BIN="${PACK_BIN:-$ROOT/target/release/rosfix-pack}"
mkdir -p "$WORK"

# repo  humble-branch  jazzy-branch  kilted-branch   ("-" = not available)
REPOS=(
  "ros-navigation/navigation2      humble        jazzy         kilted"
  "ros-controls/ros2_controllers   humble        jazzy         kilted"
  "ros-controls/ros2_control       humble        jazzy         kilted"
  "ros/robot_state_publisher       humble        jazzy         kilted"
  "ros/joint_state_publisher       ros2          ros2          ros2"
  "ros2/geometry2                  humble        jazzy         kilted"
  "ros2/demos                      humble        jazzy         kilted"
  "SteveMacenski/slam_toolbox      humble        jazzy         kilted"
  "cra-ros-pkg/robot_localization  humble-devel  jazzy-devel   kilted-devel"
)

for idx in 1 2 3; do
  distro=$(echo "humble jazzy kilted" | cut -d' ' -f$idx)
  dirs=()
  for line in "${REPOS[@]}"; do
    read -r repo b1 b2 b3 <<<"$line"
    branch=$(echo "$b1 $b2 $b3" | cut -d' ' -f$idx)
    [ "$branch" = "-" ] && continue
    dest="$WORK/$distro/$(basename "$repo")"
    if [ ! -d "$dest" ]; then
      git clone --quiet --depth 1 --branch "$branch" "https://github.com/$repo.git" "$dest" \
        || { echo "skip $repo@$branch"; continue; }
    fi
    dirs+=("$dest")
  done
  "$PACK_BIN" --distro "$distro" --out "$ROOT/crates/ros2-safety-lint/packs/$distro.json.gz" "${dirs[@]}"
done
