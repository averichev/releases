#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
npm --prefix frontend ci
npm --prefix frontend run build
npm --prefix frontend test
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
