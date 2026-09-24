#!/usr/bin/env bash
# Build a NautilusTrader wheel for this machine from the current checkout, for
# trying unreleased changes before they are published.
#
# The wheel is built the way release wheels are (the `make build-wheel` recipe:
# release profile, locked dependencies), for the Python version strategy
# repositories run on rather than the one this checkout's own environment uses,
# and copied to OUT_DIR. A provenance file beside it
# records the commit, whether the worktree had uncommitted changes, and the wheel's
# sha256, so a consumer can say exactly which source it is running.
#
# Usage: scripts/guild/build-local-wheel.sh OUT_DIR
#        PYTHON_VERSION=3.12 scripts/guild/build-local-wheel.sh OUT_DIR   (the default)
set -euo pipefail

out_dir="${1:?usage: $0 OUT_DIR}"
python_version="${PYTHON_VERSION:-3.12}"
abi="cp${python_version/./}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo"

commit="$(git rev-parse HEAD)"
dirty=false
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  dirty=true
  echo "[build-local-wheel] worktree has uncommitted changes; the provenance will say so" >&2
fi

# Wheels of earlier builds would be picked up by the glob below.
stamp="$(mktemp -d)"
trap 'rm -rf "$stamp"' EXIT
touch "$stamp/start"

interpreter="$(uv python find "$python_version")"
# `make build-wheel` builds for whichever Python this checkout's environment has,
# which need not be the one wanted here, so its recipe is run with --interpreter.
make sync
(cd python && VIRTUAL_ENV= CARGO_TARGET_DIR="$repo/target" uv run --no-sync \
  maturin build --release --locked --out ../dist --interpreter "$interpreter")

wheel="$(find dist -maxdepth 1 -name 'nautilus_trader-*.whl' -newer "$stamp/start" | head -n 1)"
if [ -z "$wheel" ]; then
  echo "[build-local-wheel] the build produced no new wheel in dist/" >&2
  exit 1
fi
case "$(basename "$wheel")" in
  *"-$abi-$abi-"*) ;;
  *) echo "[build-local-wheel] $(basename "$wheel") is not built for Python $python_version" >&2; exit 1 ;;
esac

mkdir -p "$out_dir"
cp "$wheel" "$out_dir/"
name="$(basename "$wheel")"
sha="$(shasum -a 256 "$out_dir/$name" | cut -d' ' -f1)"
cat > "$out_dir/$name.provenance.json" <<JSON
{
  "wheel": "$name",
  "sha256": "$sha",
  "source": "$repo",
  "commit": "$commit",
  "dirty": $dirty,
  "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
echo "[build-local-wheel] $out_dir/$name"
echo "[build-local-wheel] sha256 $sha, commit $commit, dirty $dirty"
