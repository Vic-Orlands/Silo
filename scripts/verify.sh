#!/usr/bin/env sh
set -eu

cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check

if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit
else
  echo "cargo-audit is not installed; skipped dependency advisory scan." >&2
fi
