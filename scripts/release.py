#!/usr/bin/env python3
"""Build immutable local releases, using Git version tags and Conventional Commits."""
import argparse
from contextlib import contextmanager
from dataclasses import asdict, dataclass
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import shutil
import subprocess
import sys
if sys.version_info < (3, 11):
    sys.exit("release: требуется Python 3.11 или новее.")
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
HEADER = re.compile(r"([a-z]+)(?:\([^()\r\n]+\))?(!)?: .+")
TARGETS = (
    ("aarch64-apple-darwin", "macos-aarch64.tool", "build", "darwin-aarch64"),
    ("x86_64-unknown-linux-musl", "linux-x86_64", "zigbuild", "linux-x86_64"),
    ("x86_64-pc-windows-gnu", "windows-x86_64.exe", "build", "windows-x86_64"),
)


class ReleaseError(Exception):
    pass


def git(root, *args):
    result = subprocess.run(["git", *args], cwd=root, text=True, capture_output=True)
    if result.returncode:
        raise ReleaseError(result.stderr.strip() or f"git {args[0]} завершился с ошибкой")
    return result.stdout.strip()


def version_parts(value):
    if not VERSION.fullmatch(value):
        raise ReleaseError(f"Ожидалась версия X.Y.Z без prerelease: {value}")
    return tuple(map(int, value.split(".")))


def package(root):
    with (root / "Cargo.toml").open("rb") as source:
        return tomllib.load(source)["package"]


@dataclass(frozen=True)
class Plan:
    version: str | None
    commit: str
    previous_tag: str | None
    bump: str | None


def stable_tags(root, commit):
    return [tag for tag in git(root, "tag", "--merged", commit).splitlines()
            if tag.startswith("v") and VERSION.fullmatch(tag[1:])]


def calculate(root):
    if git(root, "rev-parse", "--is-shallow-repository") == "true":
        raise ReleaseError("Нужна полная история Git: выполните git fetch --unshallow --tags.")
    seed = package(root)["version"]
    version_parts(seed)
    commit = git(root, "rev-parse", "HEAD")
    tags = stable_tags(root, commit)
    previous = max(tags, key=lambda tag: version_parts(tag[1:]), default=None)
    base = previous[1:] if previous else seed
    if previous and git(root, "rev-parse", f"{previous}^{{commit}}") == commit:
        return Plan(base, commit, previous, None)
    history = git(root, "log", "--format=%B%x00", f"{previous}..{commit}" if previous else commit)
    impact = 0
    for message in history.split("\0"):
        message = message.strip()
        if not message:
            continue
        header = HEADER.fullmatch(message.splitlines()[0])
        if (header and header[2]) or re.search(r"(?m)^BREAKING(?: CHANGE|-CHANGE):", message):
            impact = max(impact, 3)
        elif header:
            impact = max(impact, {"feat": 2, "fix": 1, "perf": 1, "revert": 1}.get(header[1], 0))
    if not impact:
        return Plan(None if previous else base, commit, previous, None)
    major, minor, patch = version_parts(base)
    if impact == 3 and major:
        return Plan(f"{major + 1}.0.0", commit, previous, "major")
    if impact >= 2:
        return Plan(f"{major}.{minor + 1}.0", commit, previous, "minor")
    return Plan(f"{major}.{minor}.{patch + 1}", commit, previous, "patch")


def show_release_commits(root, plan):
    # HEAD may already carry this release's tag. Exclude it so retries show
    # the release's changes, rather than an empty current-tag..HEAD range.
    previous = max(
        (tag for tag in stable_tags(root, plan.commit)
         if version_parts(tag[1:]) < version_parts(plan.version)),
        key=lambda tag: version_parts(tag[1:]), default=None,
    )
    history = f"{previous}..{plan.commit}" if previous else plan.commit
    commits = git(root, "log", "--reverse", "--topo-order", "--no-color",
                  "--no-show-signature", "--format=%h %s", history).splitlines()
    print(f"Коммиты релиза ({previous or 'начало истории'} → v{plan.version}):")
    for commit in commits:
        print(f"  {commit}")
    if not commits:
        print("  Новых коммитов нет.")


def require_clean(root):
    if git(root, "status", "--porcelain", "--untracked-files=normal"):
        raise ReleaseError("Рабочее дерево содержит изменения. Сначала закоммитьте их и повторите make release.")


@contextmanager
def release_lock(root):
    filename = Path(git(root, "rev-parse", "--git-path", "veruna-release.lock"))
    if not filename.is_absolute():
        filename = root / filename
    with filename.open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ReleaseError("Сборка релиза в этом checkout уже выполняется.") from error
        try:
            yield
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


def filenames(name, version):
    return [f"{name}-{version}-{suffix}" for _, suffix, _, _ in TARGETS]


def release_files(name, version, signed):
    binaries = filenames(name, version)
    signatures = [filename + ".jws" for filename in binaries] if signed else []
    return [*binaries, *signatures, "release.json"]


def checksum(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def verify_output(output, name, plan):
    if output.is_symlink() or not output.is_dir():
        raise ReleaseError(f"Путь релиза занят: {output}")
    try:
        metadata = json.loads((output / "release.json").read_text())
        if metadata["version"] != plan.version or metadata["commit"] != plan.commit:
            raise ValueError("версия или коммит не совпадают")
        expected = "".join(f"{checksum(output / item)}  {item}\n"
                           for item in release_files(name, plan.version, metadata.get("signed_updates", False)))
        if (output / "SHA256SUMS").read_text() != expected:
            raise ValueError("контрольные суммы не совпадают")
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise ReleaseError(f"Каталог {output} уже существует и не будет перезаписан: {error}") from error


def ensure_tag(root, plan):
    tag = f"v{plan.version}"
    if tag in git(root, "tag", "--list", tag).splitlines():
        if git(root, "rev-parse", f"{tag}^{{commit}}") != plan.commit:
            raise ReleaseError(f"Тег {tag} уже указывает на другой коммит.")
    else:
        git(root, "-c", "tag.gpgSign=false", "tag", tag, plan.commit)


def sign_binaries(root, name, version, staging):
    configured = os.environ.get("RELEASE_SIGNING_KEY")
    key = Path(configured) if configured is not None else Path.home() / ".config/veruna-releases/update-signing.pem"
    if configured is None and not key.exists():
        print("Ключ издателя не настроен. Дистрибутивы собраны для ручной установки без .jws.")
        return False
    if not key.is_file():
        raise ReleaseError("Не найден закрытый ключ издателя для подписи релиза.")
    identity_path = Path(os.environ.get("RELEASE_UPDATE_IDENTITY", "deploy/self-update-identity.json"))
    identity = json.loads((root / identity_path).read_text())
    signer = root / "docs/integrations/standalone/example.mjs"
    with tempfile.TemporaryDirectory() as directory:
        public = Path(directory) / "public.jwk"
        public.write_text(json.dumps(identity["public_key"]))
        for filename, (_, _, _, target) in zip(filenames(name, version), TARGETS, strict=True):
            binary = staging / filename
            context = [identity["product_id"], version, target, str(binary)]
            signature = subprocess.run(["node", signer, "sign", key, *context, "--kid"],
                                       check=True, capture_output=True).stdout
            path = staging / (filename + ".jws")
            with path.open("xb") as output:
                output.write(signature)
            subprocess.run(["node", signer, "verify", public, *context, path], check=True,
                           stdout=subprocess.DEVNULL)
    return True


def build_binaries(root, name, version, staging):
    if sys.platform != "darwin":
        raise ReleaseError("Сборка трёх платформ требует macOS с Xcode Command Line Tools.")
    for tool in ["npm", "rustup", "zig", "cargo-zigbuild", "xcrun",
                 "x86_64-w64-mingw32-gcc", "x86_64-w64-mingw32-ar"]:
        if not shutil.which(tool):
            raise ReleaseError(f"Не найдена команда {tool}. Установка описана в README.md.")
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    if soft != resource.RLIM_INFINITY and soft < 8192:
        try:
            resource.setrlimit(resource.RLIMIT_NOFILE, (8192, hard))
        except (OSError, ValueError) as error:
            raise ReleaseError(f"Для Zig нужен лимит открытых файлов 8192 (soft={soft}, hard={hard}).") from error
    environment = os.environ.copy()
    environment["RELEASE_VERSION"] = version
    toolchain = environment.get("RELEASE_TOOLCHAIN", "stable")

    def run(*args, env=environment, capture=False):
        result = subprocess.run(args, cwd=root, env=env, check=True, text=True,
                                stdout=subprocess.PIPE if capture else None)
        return result.stdout.strip() if capture else None

    run("xcrun", "--sdk", "macosx", "--show-sdk-path", capture=True)
    rustc = run("rustup", "which", "--toolchain", toolchain, "rustc", capture=True)
    environment["RUSTC"] = rustc
    environment["PATH"] = str(Path(rustc).parent) + os.pathsep + environment.get("PATH", "")
    for target, _, _, _ in TARGETS:
        libdir = run("rustup", "run", toolchain, "rustc", "--print", "target-libdir", "--target", target, capture=True)
        if not Path(libdir).is_dir():
            raise ReleaseError(f"Выполните rustup target add --toolchain {toolchain} {target}")
    run("npm", "--prefix", "frontend", "ci")
    run("npm", "--prefix", "frontend", "run", "build")
    for target, suffix, action, _ in TARGETS:
        target_environment = environment.copy()
        if target == "x86_64-pc-windows-gnu":
            target_environment.update({
                "CC_x86_64_pc_windows_gnu": "x86_64-w64-mingw32-gcc",
                "AR_x86_64_pc_windows_gnu": "x86_64-w64-mingw32-ar",
                "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER": "x86_64-w64-mingw32-gcc",
            })
        run("rustup", "run", toolchain, "cargo", action, "--locked", "--release", "--bin", name,
            "--target", target, "--target-dir", "target", env=target_environment)
        source = root / "target" / target / "release" / (name + (".exe" if suffix.endswith(".exe") else ""))
        destination = staging / f"{name}-{version}-{suffix}"
        shutil.copyfile(source, destination)
        destination.chmod(0o755)
    return sign_binaries(root, name, version, staging)


def release(root):
    with release_lock(root):
        require_clean(root)
        plan = calculate(root)
        if plan.version is None:
            print(f"После {plan.previous_tag} нет изменений для нового релиза.")
            return None
        name = package(root)["name"]
        tag = f"v{plan.version}"
        if tag in git(root, "tag", "--list", tag).splitlines():
            if git(root, "rev-parse", f"{tag}^{{commit}}") != plan.commit:
                raise ReleaseError(f"Тег {tag} занят другим коммитом; проверьте историю релизов.")
        output = root / "dist" / "releases" / plan.version
        if output.exists() or output.is_symlink():
            verify_output(output, name, plan)
            ensure_tag(root, plan)
            print(f"Релиз {plan.version} уже собран: {output}")
            show_release_commits(root, plan)
            return output
        reason = f"{plan.previous_tag or package(root)['version']} → {plan.version} ({plan.bump})" if plan.bump else (
            f"тег {tag}" if plan.previous_tag else "базовая Cargo-версия")
        print(f"Сборка релиза {plan.version}: {reason}", flush=True)
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix=".release-", dir=output.parent) as directory:
            staging = Path(directory) / "complete"
            staging.mkdir()
            signed = build_binaries(root, name, plan.version, staging)
            require_clean(root)
            if calculate(root) != plan:
                raise ReleaseError("История Git изменилась во время сборки; релиз не сохранён.")
            metadata = {
                "version": plan.version, "commit": plan.commit,
                "previous_tag": plan.previous_tag,
            }
            if signed:
                metadata["signed_updates"] = True
            (staging / "release.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n")
            files = release_files(name, plan.version, signed)
            (staging / "SHA256SUMS").write_text("".join(f"{checksum(staging / item)}  {item}\n" for item in files))
            if output.exists() or output.is_symlink():
                raise ReleaseError(f"Каталог {output} появился во время сборки и не будет перезаписан.")
            staging.rename(output)
        # If tag creation fails, keep the complete directory: the next invocation
        # validates its hashes and finishes tagging without rebuilding or overwriting.
        ensure_tag(root, plan)
        print(f"Релиз {plan.version} собран: {output}\nЛокальный тег: {tag}.")
        show_release_commits(root, plan)
        return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", action="store_true", help="показать расчёт версии без сборки и изменений Git")
    args = parser.parse_args()
    try:
        if args.plan:
            print(json.dumps(asdict(calculate(ROOT)), ensure_ascii=False, indent=2))
        else:
            release(ROOT)
    except (ReleaseError, OSError, subprocess.CalledProcessError, KeyError, tomllib.TOMLDecodeError) as error:
        print(f"release: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
