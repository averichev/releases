#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
npm --prefix frontend ci
npm --prefix frontend run build
cargo build --locked --release --target-dir target
