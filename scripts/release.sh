#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."

fail() {
  printf 'release: %s\n' "$*" >&2
  exit 1
}

[ "$(uname -s)" = Darwin ] || fail "Сборка трёх платформ требует macOS с Xcode Command Line Tools."

# Zig opens many object files concurrently when linking static musl binaries.
# Raise only this process's soft limit; preserve higher and unlimited limits.
minimum_open_files=8192
open_files=$(ulimit -S -n)
if [ "$open_files" != unlimited ] && [ "$open_files" -lt "$minimum_open_files" ]; then
  ulimit -S -n "$minimum_open_files" 2>/dev/null ||
    fail "Для линковки Zig нужен лимит открытых файлов не ниже $minimum_open_files (soft=$open_files, hard=$(ulimit -H -n)). Увеличьте hard limit в окружении запуска и повторите make release."
fi

for tool in npm rustup zig cargo-zigbuild xcrun x86_64-w64-mingw32-gcc x86_64-w64-mingw32-ar shasum; do
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

# Copy only after every build succeeds; keep previous binaries on failure.
mkdir -p dist/releases
staging=$(mktemp -d "$PWD/dist/releases/.release.XXXXXX")
trap 'rm -rf "$staging"' EXIT
trap 'exit 1' HUP INT TERM

prefix=veruna-releases-$version
# Finder treats the dots in SemVer as an unknown extension without .tool.
# Terminal registers .tool and executes this same Mach-O binary directly.
cp target/aarch64-apple-darwin/release/veruna-releases "$staging/$prefix-macos-aarch64.tool"
cp target/x86_64-unknown-linux-musl/release/veruna-releases "$staging/$prefix-linux-x86_64"
cp target/x86_64-pc-windows-gnu/release/veruna-releases.exe "$staging/$prefix-windows-x86_64.exe"
(
  cd "$staging"
  shasum -a 256 "$prefix-macos-aarch64.tool" "$prefix-linux-x86_64" "$prefix-windows-x86_64.exe" > SHA256SUMS
)

output=$PWD/dist/releases/$version
mkdir -p "$output"
mv "$staging/"* "$output/"
# Remove only legacy artifacts generated for this same version.
rm -f "$output/$prefix-macos-aarch64.tar.gz" \
  "$output/$prefix-linux-x86_64.tar.gz" "$output/$prefix-windows-x86_64.zip" \
  "$output/$prefix-macos-aarch64"
printf '\nРелиз %s собран: %s\n' "$version" "$output"
