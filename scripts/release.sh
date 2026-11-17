#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."

fail() {
  printf 'release: %s\n' "$*" >&2
  exit 1
}

[ "$(uname -s)" = Darwin ] || fail "Сборка трёх платформ требует macOS с Xcode Command Line Tools."

for tool in npm rustup zig cargo-zigbuild xcrun x86_64-w64-mingw32-gcc x86_64-w64-mingw32-ar tar zip shasum; do
  command -v "$tool" >/dev/null 2>&1 || fail "Не найдена команда $tool. Установка зависимостей описана в README.md."
done
xcrun --sdk macosx --show-sdk-path >/dev/null

# Use one rustup toolchain for Cargo, rustc and target libraries, even when
# Homebrew's Rust appears first in PATH.
toolchain=${RELEASE_TOOLCHAIN:-stable}
RUSTC=$(rustup which --toolchain "$toolchain" rustc)
PATH=$(dirname "$RUSTC"):$PATH
export RUSTC PATH
for target in aarch64-apple-darwin x86_64-unknown-linux-musl x86_64-pc-windows-gnu; do
  libdir=$(rustup run "$toolchain" rustc --print target-libdir --target "$target")
  [ -d "$libdir" ] || fail "Выполните rustup target add --toolchain $toolchain $target"
done

package_id=$(rustup run "$toolchain" cargo pkgid --locked)
version=${package_id##*@}
case "$version" in
  ''|*[!0-9A-Za-z.+-]*) fail "Не удалось определить версию package из Cargo.toml." ;;
esac

npm --prefix frontend ci
npm --prefix frontend run build

rustup run "$toolchain" cargo build --locked --release --bin veruna-releases \
  --target aarch64-apple-darwin --target-dir target
rustup run "$toolchain" cargo zigbuild --locked --release --bin veruna-releases \
  --target x86_64-unknown-linux-musl --target-dir target
CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc \
AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
  rustup run "$toolchain" cargo build --locked --release --bin veruna-releases \
    --target x86_64-pc-windows-gnu --target-dir target

# Package only after every build succeeds; keep previous archives on failure.
mkdir -p dist/releases
staging=$(mktemp -d "$PWD/dist/releases/.release.XXXXXX")
trap 'rm -rf "$staging"' EXIT
trap 'exit 1' HUP INT TERM

prefix=veruna-releases-$version
# Exclude AppleDouble sidecars and extended attributes from macOS tar.
COPYFILE_DISABLE=1 tar --no-xattrs -czf "$staging/$prefix-macos-aarch64.tar.gz" \
  -C target/aarch64-apple-darwin/release veruna-releases
COPYFILE_DISABLE=1 tar --no-xattrs -czf "$staging/$prefix-linux-x86_64.tar.gz" \
  -C target/x86_64-unknown-linux-musl/release veruna-releases
zip -q -j "$staging/$prefix-windows-x86_64.zip" \
  target/x86_64-pc-windows-gnu/release/veruna-releases.exe
(
  cd "$staging"
  shasum -a 256 ./*.tar.gz ./*.zip > SHA256SUMS
)

output=$PWD/dist/releases/$version
mkdir -p "$output"
mv "$staging/"* "$output/"
printf '\nРелиз %s собран: %s\n' "$version" "$output"
