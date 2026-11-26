"""Exercise version planning and release recovery against disposable real Git repositories."""
from contextlib import redirect_stdout
import base64
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import uuid
from unittest.mock import patch

import release


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="veruna-release-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.git("init", "--quiet")
        self.git("config", "user.name", "Release test")
        self.git("config", "user.email", "release@example.invalid")
        self.git("config", "commit.gpgSign", "false")
        self.git("config", "tag.gpgSign", "false")
        (self.root / "Cargo.toml").write_text('[package]\nname = "test-app"\nversion = "0.1.0"\n')
        (self.root / ".gitignore").write_text("/dist/\n")
        self.commit("chore: начальный коммит")

    def git(self, *args):
        result = subprocess.run(["git", "-c", "core.hooksPath=/dev/null", *args], cwd=self.root,
                                check=True, text=True, capture_output=True)
        return result.stdout.strip()

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "--quiet", "--allow-empty", "-m", message)
        return self.git("rev-parse", "HEAD")

    def fake_build(self, root, name, version, staging):
        for filename in release.filenames(name, version):
            (staging / filename).write_bytes(f"executable {version}".encode())

    def run_release(self, builder=None):
        with patch.object(release, "build_binaries", side_effect=builder or self.fake_build) as build:
            with redirect_stdout(io.StringIO()) as console:
                output = release.release(self.root)
            self.console = console.getvalue()
            return output, build.call_count

    def test_release_commits_include_all_changes_and_repeat_keeps_the_same_list(self):
        self.git("tag", "v0.1.0")
        docs = self.commit("docs: уточнить запуск")
        feature = self.commit("feat: добавить возможность")
        output, _ = self.run_release()
        header = "Коммиты релиза (v0.1.0 → v0.2.0):\n"
        first = self.console.split(header)[1]
        self.assertIn(f"{self.git('rev-parse', '--short', docs)} docs: уточнить запуск", first)
        self.assertIn(f"{self.git('rev-parse', '--short', feature)} feat: добавить возможность", first)
        self.assertLess(first.index("docs:"), first.index("feat:"))
        self.assertNotIn("начальный коммит", first)
        before = {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in output.iterdir()}
        self.assertEqual(self.run_release(), (output, 0))
        self.assertIn("уже собран", self.console)
        self.assertEqual(self.console.split(header)[1], first)
        self.assertEqual(before, {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in output.iterdir()})

    def test_first_release_commit_list_starts_at_the_beginning_of_history(self):
        self.commit("feat: добавить возможность")
        self.run_release()
        self.assertIn("Коммиты релиза (начало истории → v0.2.0):", self.console)
        self.assertIn("chore: начальный коммит", self.console)
        self.assertIn("feat: добавить возможность", self.console)

    def test_tagged_head_commit_list_includes_merged_changes_and_excludes_other_tags(self):
        self.git("tag", "v1.0.0")
        base_branch = self.git("branch", "--show-current")
        self.git("switch", "--quiet", "-c", "feature")
        self.commit("feat: изменение в ветке")
        self.git("tag", "v9.0.0-beta.1")
        self.git("switch", "--quiet", base_branch)
        self.commit("docs: уточнение в основной ветке")
        self.git("merge", "--quiet", "--no-ff", "feature", "-m", "Merge feature")
        self.git("tag", "-a", "v1.1.0", "-m", "Релиз 1.1.0")
        head = self.git("rev-parse", "HEAD")
        self.commit("feat: изменение вне релиза")
        self.git("tag", "v9.0.0")
        self.git("reset", "--hard", head)
        self.run_release()
        history = self.console.split("Коммиты релиза (v1.0.0 → v1.1.0):\n")[1]
        self.assertIn("feat: изменение в ветке", history)
        self.assertIn("docs: уточнение в основной ветке", history)
        self.assertIn("Merge feature", history)
        self.assertNotIn("начальный коммит", history)
        self.assertNotIn("изменение вне релиза", history)

    def test_conventional_commits_choose_the_highest_bump_since_the_last_tag(self):
        for tag, message, expected in [
            ("v1.2.3", "fix(http): исправить ответ", "1.2.4"),
            ("v1.2.3", "perf: ускорить выдачу", "1.2.4"),
            ("v1.2.3", "revert: вернуть проверку", "1.2.4"),
            ("v1.2.3", "feat: добавить проверку", "1.3.0"),
            ("v1.2.3", "refactor(api)!: изменить контракт", "2.0.0"),
            ("v1.2.3", "fix: изменить контракт\n\nBREAKING CHANGE: новый формат", "2.0.0"),
            ("v1.2.3", "feat: изменить контракт\n\nBREAKING-CHANGE: новый формат", "2.0.0"),
            ("v0.2.3", "feat(api)!: изменить контракт", "0.3.0"),
            ("v0.2.3", "fix: исправить ответ", "0.2.4"),
        ]:
            with self.subTest(tag=tag, message=message):
                baseline = self.git("rev-parse", "HEAD")
                self.git("tag", tag)
                self.commit(message)
                self.commit("fix: дополнительное исправление")
                self.assertEqual(release.calculate(self.root).version, expected)
                self.git("tag", "--delete", tag)
                self.git("reset", "--hard", baseline)

    def test_first_release_uses_cargo_seed_and_never_reads_legacy_output_as_state(self):
        legacy = self.root / "dist/releases/0.1.0"
        legacy.mkdir(parents=True)
        (legacy / "old.bin").write_bytes(b"previous release")
        self.commit("feat: добавить возможность")
        self.assertEqual(release.calculate(self.root).version, "0.2.0")
        output, _ = self.run_release()
        self.assertEqual(output.name, "0.2.0")
        self.assertEqual((legacy / "old.bin").read_bytes(), b"previous release")

    def test_only_documentation_does_not_release_and_same_tag_keeps_its_version(self):
        self.git("tag", "v2.3.4")
        self.assertEqual(release.calculate(self.root).version, "2.3.4")
        self.commit("docs: уточнить запуск")
        self.assertIsNone(release.calculate(self.root).version)
        output, count = self.run_release()
        self.assertIsNone(output)
        self.assertEqual(count, 0)

    def test_only_reachable_stable_tags_are_version_anchors(self):
        first = self.git("rev-parse", "HEAD")
        self.git("tag", "v1.2.3")
        self.git("tag", "v8.0.0-beta.1")
        self.git("tag", "unrelated")
        unrelated = self.commit("feat: другая ветка")
        self.git("tag", "v9.0.0", unrelated)
        self.git("reset", "--hard", first)
        self.commit("fix: исправление текущей ветки")
        self.assertEqual(release.calculate(self.root).version, "1.2.4")

    def test_existing_annotated_head_tag_can_be_built_without_replacing_it(self):
        self.git("tag", "-a", "v2.3.4", "-m", "Релиз 2.3.4")
        tag_object = self.git("rev-parse", "refs/tags/v2.3.4")
        output, count = self.run_release()
        self.assertEqual(count, 1)
        self.assertEqual(output.name, "2.3.4")
        self.assertEqual(self.git("rev-parse", "refs/tags/v2.3.4"), tag_object)
        self.assertEqual(self.git("tag", "--list"), "v2.3.4")

    def test_success_publishes_all_files_then_tags_and_repeat_reuses_them(self):
        self.git("tag", "v0.1.0")
        head = self.commit("fix: исправить запуск")
        output, count = self.run_release()
        self.assertEqual(count, 1)
        self.assertEqual(output.name, "0.1.1")
        self.assertEqual(self.git("rev-parse", "v0.1.1^{commit}"), head)
        self.assertEqual(json.loads((output / "release.json").read_text())["commit"], head)
        before = {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in output.iterdir()}
        self.assertEqual(self.run_release(), (output, 0))
        self.assertEqual(before, {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in output.iterdir()})
        self.assertEqual(self.git("status", "--porcelain"), "")

    def test_signed_release_covers_each_platform_and_detects_damaged_signature_on_retry(self):
        # Use a disposable issuer so tests never sign fixtures with the publisher's key.
        with tempfile.TemporaryDirectory() as tmp:
            issuer = Path(tmp)
            signer = release.ROOT / "docs/integrations/standalone/example.mjs"
            subprocess.run(["node", signer, "keygen", issuer / "test"], check=True, capture_output=True)
            product = str(uuid.uuid4())
            identity = issuer / "identity.json"
            identity.write_text(json.dumps({"product_id": product, "public_key": json.loads(
                (issuer / "test.public.jwk").read_text())}))

            def build(root, name, version, staging):
                self.fake_build(root, name, version, staging)
                return release.sign_binaries(release.ROOT, name, version, staging)

            with patch.dict(os.environ, {"RELEASE_SIGNING_KEY": str(issuer / "test.pem"),
                                         "RELEASE_UPDATE_IDENTITY": str(identity)}):
                output, _ = self.run_release(build)
        self.assertTrue(json.loads((output / "release.json").read_text())["signed_updates"])
        signatures = sorted(output.glob("*.jws"))
        targets = []
        for signature in signatures:
            payload = signature.read_text().strip().split(".")[1]
            metadata = json.loads(base64.urlsafe_b64decode(payload + "==="))
            self.assertEqual(metadata["application_id"], product)
            self.assertEqual(metadata["sha256"], release.checksum(signature.with_suffix("")))
            targets.append((metadata["os"], metadata["arch"]))
        self.assertCountEqual(targets, [("darwin", "aarch64"), ("linux", "x86_64"), ("windows", "x86_64")])
        self.assertEqual(self.run_release(), (output, 0))
        signatures[0].write_text("damaged")
        with self.assertRaisesRegex(release.ReleaseError, "суммы"):
            self.run_release()

    def test_failed_platform_does_not_consume_version_or_touch_previous_release(self):
        previous, _ = self.run_release()
        self.commit("fix: исправить запуск")
        def fail(root, name, version, staging):
            (staging / "partial").write_bytes(b"partial")
            raise release.ReleaseError("compiler failed")
        with self.assertRaisesRegex(release.ReleaseError, "compiler failed"):
            self.run_release(fail)
        self.assertEqual(self.git("tag", "--list"), "v0.1.0")
        self.assertFalse((self.root / "dist/releases/0.1.1").exists())
        self.assertEqual(list((self.root / "dist/releases").iterdir()), [previous])
        self.assertEqual(self.run_release()[0].name, "0.1.1")

    def test_completed_output_survives_tag_failure_and_retry_finishes_without_build(self):
        self.commit("feat: добавить возможность")
        with patch.object(release, "ensure_tag", side_effect=release.ReleaseError("tag failed")):
            with self.assertRaisesRegex(release.ReleaseError, "tag failed"):
                self.run_release()
        self.assertEqual(self.git("tag", "--list"), "")
        output, count = self.run_release()
        self.assertEqual(count, 0)
        self.assertEqual(output.name, "0.2.0")
        self.assertEqual(self.git("tag", "--list"), "v0.2.0")

    def test_dirty_tree_conflicting_tag_and_damaged_output_are_never_overwritten(self):
        self.git("tag", "v0.1.0")
        (self.root / "uncommitted").write_text("work")
        with self.assertRaisesRegex(release.ReleaseError, "закоммитьте"):
            self.run_release()
        self.commit("fix: исправить запуск")
        output, _ = self.run_release()
        binary = output / release.filenames("test-app", "0.1.1")[0]
        binary.write_bytes(b"damaged")
        with self.assertRaisesRegex(release.ReleaseError, "не будет перезаписан"):
            self.run_release()
        self.assertEqual(binary.read_bytes(), b"damaged")
        self.git("tag", "--delete", "v0.1.1")
        self.git("tag", "v0.1.1", self.git("rev-parse", "HEAD~1"))
        # This tag is reachable and now identifies a previous release; a tag on
        # another branch occupying the next number must instead stop the build.
        self.commit("fix: исправление")
        next_version = release.calculate(self.root).version
        branch_head = self.git("rev-parse", "HEAD")
        unrelated = self.commit("feat: другая ветка")
        self.git("tag", f"v{next_version}", unrelated)
        self.git("reset", "--hard", branch_head)
        with self.assertRaisesRegex(release.ReleaseError, "занят другим коммитом"):
            self.run_release()

    def test_head_change_and_parallel_build_abort_before_publication(self):
        self.git("tag", "v0.1.0")
        self.commit("fix: исправление")
        def change_head(root, name, version, staging):
            self.fake_build(root, name, version, staging)
            self.commit("feat: конкурентное изменение")
        with self.assertRaisesRegex(release.ReleaseError, "История Git изменилась"):
            self.run_release(change_head)
        self.assertEqual(self.git("tag", "--list"), "v0.1.0")
        with release.release_lock(self.root):
            with self.assertRaisesRegex(release.ReleaseError, "уже выполняется"):
                self.run_release()

    def test_shallow_history_is_rejected(self):
        clone = self.root / "shallow"
        subprocess.run(["git", "clone", "--quiet", "--depth", "1", self.root.as_uri(), str(clone)], check=True)
        with self.assertRaisesRegex(release.ReleaseError, "полная история"):
            release.calculate(clone)


if __name__ == "__main__":
    unittest.main()
