#!/usr/bin/env bash
# Publish the rosfix fix ranker (model repo) and knowledge packs (dataset repo)
# to Hugging Face. Run after `hf auth login` (or with HF_TOKEN set).
#
#   HF_USER=<your-hf-username> scripts/publish_hf.sh            # publish both
#   HF_USER=<your-hf-username> scripts/publish_hf.sh --dry-run  # stage only
#
# Repos are created on first upload. Re-running uploads a new commit with
# only the changed files.
set -euo pipefail

: "${HF_USER:?set HF_USER to your Hugging Face username or org}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/crates/ros2-safety-lint"
VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$CRATE/Cargo.toml" | head -1)"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/model" "$STAGE/dataset/packs" "$STAGE/dataset/keys" "$STAGE/dataset/scripts"

cp "$ROOT/huggingface/model/README.md" "$STAGE/model/"
cp "$CRATE/src/assist/learned_weights.rs" "$CRATE/src/assist/learned.rs" "$STAGE/model/"

cp "$ROOT/huggingface/dataset/README.md" "$STAGE/dataset/"
cp "$CRATE"/packs/*.json.gz "$STAGE/dataset/packs/"
cp "$CRATE"/packs/keys/*.txt.gz "$STAGE/dataset/keys/"
cp "$ROOT/scripts/build_packs.sh" "$ROOT/scripts/build_rosdep_keys.py" "$STAGE/dataset/scripts/"

for f in LICENSE-MIT LICENSE-APACHE; do
  cp "$ROOT/$f" "$STAGE/model/"
  cp "$ROOT/$f" "$STAGE/dataset/"
done

if [ "${1:-}" = "--dry-run" ]; then
  (cd "$STAGE" && find . -type f | sort)
  exit 0
fi

hf auth whoami >/dev/null || { echo "not logged in: run 'hf auth login' first" >&2; exit 1; }

hf upload "$HF_USER/rosfix-assist" "$STAGE/model" . \
  --repo-type model --commit-message "rosfix $VERSION"
hf upload "$HF_USER/rosfix-knowledge-packs" "$STAGE/dataset" . \
  --repo-type dataset --commit-message "rosfix $VERSION"

echo "https://huggingface.co/$HF_USER/rosfix-assist"
echo "https://huggingface.co/datasets/$HF_USER/rosfix-knowledge-packs"
