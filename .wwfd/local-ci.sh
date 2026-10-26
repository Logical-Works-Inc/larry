#!/usr/bin/env bash
# .wwfd/local-ci.sh — Rocco's local mirror of the gates its own AGENTS.md declares.
#
# Why this exists: the spend guard refuses a push unless this repository has a
# fresh local-CI receipt or an open PR on the branch. Rocco had neither, and
# `keel` cannot gate it because there is no `.keel/lanes.toml` canonical plan
# here. This script is the guard's named alternative.
#
# Design contract, borrowed from the sibling repos that use this convention:
#   - The same commands AGENTS.md mandates, in the same order. No skips, no
#     weaker proxy, no relaxed limit. If AGENTS.md changes, this changes.
#   - `set -euo pipefail`: the first red lane aborts and no receipt is written.
#   - The receipt is written ONLY after every lane exits 0, and it names the
#     head it attested to, so it cannot silently vouch for a later commit.
#   - A gate that cannot run is a failure, not a skip.
#
# Deliberately absent: keel lanes, the signing key, mi-ratchet and the
# changed-file scoping. Rocco has no `.keel` plan, no signing key and no
# `scripts/ci/`, and inventing those here would be fabricating a gate rather
# than mirroring one.
#
# Lanes, in the order AGENTS.md lists them:
#   build        cargo build --bins --locked
#   tests        cargo test --workspace --all-targets --locked
#   clippy       cargo clippy --workspace --all-targets --locked -- -D warnings
#   fmt          cargo fmt --all -- --check
#   governance   python3 -m unittest discover -s tests
#   check        python3 tools/check_governance.py
#   deps         lgwks-deps check .
#   demo-cli     python3 tools/test_demo_cli.py
#   native-pty   python3 tools/test_native_cli.py
#   whitespace   git diff --check

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
REPO_ROOT="$(pwd)"
STATE_DIR="$HOME/wwfd/state"
RECEIPT="$STATE_DIR/local-ci-receipt.json"

step() { printf '\n\033[1mlocal-ci: %s\033[0m\n' "$*"; }

step 'cargo build --bins --locked'
cargo build --bins --locked

step 'cargo test --workspace --all-targets --locked'
cargo test --workspace --all-targets --locked

step 'cargo clippy --workspace --all-targets --locked -- -D warnings'
cargo clippy --workspace --all-targets --locked -- -D warnings

step 'cargo fmt --all -- --check'
cargo fmt --all -- --check

step 'python3 -m unittest discover -s tests'
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests

step 'python3 tools/check_governance.py'
PYTHONDONTWRITEBYTECODE=1 python3 tools/check_governance.py

step 'lgwks-deps check .'
lgwks-deps check .

step 'python3 tools/test_demo_cli.py'
PYTHONDONTWRITEBYTECODE=1 python3 tools/test_demo_cli.py

step 'python3 tools/test_native_cli.py  (real POSIX PTY journeys)'
PYTHONDONTWRITEBYTECODE=1 python3 tools/test_native_cli.py

step 'git diff --check'
git diff --check

# The PTY suite is the only lane that can pass by finding nothing. Assert it
# actually ran its journeys rather than reporting a vacuous green. Colour is
# forced off: Python 3.14's unittest wraps "ok" in ANSI escapes when the host
# sets FORCE_COLOR, and the count below would then read 0 against a green run.
step 'span check — the native PTY lane must have run both journeys'
journeys="$(
  NO_COLOR=1 PYTHON_COLORS=0 FORCE_COLOR= PYTHONDONTWRITEBYTECODE=1 python3 tools/test_native_cli.py 2>&1 |
    grep -c '\.\.\. ok$' || true
)"
if [[ "$journeys" -lt 2 ]]; then
  printf 'local-ci: the native PTY suite reported %s journeys, expected 2\n' "$journeys" >&2
  exit 1
fi

# Only reached when every lane above exited 0.
mkdir -p "$STATE_DIR"
printf '{"repo_root":"%s","branch":"%s","head_sha":"%s","attested_at":%s,"ci_script":"bash .wwfd/local-ci.sh"}\n' \
  "$REPO_ROOT" \
  "$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)" \
  "$(git rev-parse HEAD)" \
  "$(date +%s)" > "$RECEIPT"

step "OK — every declared lane passed; receipt written for $(git rev-parse --short HEAD)"
