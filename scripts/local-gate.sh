#!/usr/bin/env bash
# Local replica of the android-apk CI gates (see .github/workflows/android-apk.yml).
#
# Run this BEFORE pushing: every gate here has failed a CI run at least once
# (budget, per-file rustfmt, clippy -D warnings, Rust tests, FFI tests, Dart).
# The Rust tests use the same FQSIGN_LIB wiring as CI — the emu artifact is the
# only signing path now, and a missing artifact fails the mock-upstream
# contract test (x-argus assertion).
#
# Usage: scripts/local-gate.sh [--skip-dart] [--skip-ffi]
set -euo pipefail
cd "$(dirname "$0")/.."

# The workspace SDK location; the CI installs Flutter on PATH already.
if ! command -v flutter >/dev/null 2>&1 && [ -x /data/flutter/bin/flutter ]; then
  export PATH="/data/flutter/bin:$PATH"
fi

skip_dart=0
skip_ffi=0
for arg in "$@"; do
  case "$arg" in
    --skip-dart) skip_dart=1 ;;
    --skip-ffi) skip_ffi=1 ;;
    *) echo "unknown flag: $arg" >&2; exit 2 ;;
  esac
done

step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

step "1/7 Rust formatting (per-file, generated bindings excluded)"
mapfile -t sources < <(
  git ls-files -- 'rust/**/*.rs' 'rust/*.rs' | grep -v 'frb_generated\.rs$'
)
failed=0
for source in "${sources[@]}"; do
  if ! rustfmt --check --edition 2021 --config skip_children=true "$source"; then
    printf 'Misformatted: %s\n' "$source" >&2
    failed=1
  fi
done
[ "$failed" -eq 0 ] || { echo "FAIL: rustfmt"; exit 1; }

step "2/7 Rust lints (clippy -D warnings, all targets)"
cargo clippy --manifest-path rust/Cargo.toml --locked --all-targets -- -D warnings

step "3/7 Rust tests (FQSIGN_LIB -> emu signer)"
artifact="$PWD/rust/vendor/libfqsign-linux-x86_64.so"
if [ ! -f "$artifact" ]; then
  echo "missing $artifact — download it first:" >&2
  echo "  gh release download v0.2.1 -R dgscyg/rustnidbg -p libfqsign-linux-x86_64.so --clobber -D rust/vendor" >&2
  exit 1
fi
FQSIGN_LIB="$artifact" cargo test --manifest-path rust/Cargo.toml --locked

step "4/7 Size budget"
bash scripts/check_line_budget.sh

if [ "$skip_ffi" -eq 0 ]; then
  step "5/7 Host Rust core + FFI integration tests"
  cargo build --manifest-path rust/Cargo.toml
  bash scripts/run_rust_host_tests.sh --skip-build
fi

if [ "$skip_dart" -eq 0 ]; then
  step "6/7 Dart analyze"
  flutter analyze --no-pub
  step "7/7 Full Dart test suite"
  flutter test
fi

echo
echo "All local gates passed — safe to push."