import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import compare_profile_backup as backup


class ProfileBackupTest(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name) / "data"
        self.test_dir = self.root / "profiles" / "t0123456789ab"
        self.test_dir.mkdir(parents=True)
        self.zip_path = Path(scratch.name) / "backup.zip"
        self.dirs = [self.root, self.test_dir]

    def save_live(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def save_zip(self, files):
        with zipfile.ZipFile(self.zip_path, "w") as archive:
            for name, data in files.items():
                archive.writestr(name, data)

    def compare(self):
        return backup.compare(self.root, self.dirs, self.zip_path)

    def test_reports_changed_missing_and_retired_files(self):
        prefix = self.test_dir.relative_to(self.root).as_posix()
        wheel = prefix + "/wheels.json"
        action = prefix + "/actions/search/入口.mjs"
        manifest = prefix + "/actions/search/action.json"
        retired = "profiles/tabcdef012345/bindings.json"
        self.save_live(wheel, b"new")
        self.save_live(action, b"current implementation")
        self.save_live(manifest, b"new manifest")
        self.save_live("bindings.json", b"daily")
        self.save_zip({wheel: b"old", manifest: b"old", retired: b"[]", "bindings.json": b"daily"})
        result = self.compare()
        self.assertFalse(result["matches"])
        self.assertEqual(result["checked_profiles"], 2)
        self.assertEqual(result["changed"], [manifest, wheel])
        self.assertEqual(result["missing_from_backup"], [action])
        self.assertEqual(result["only_in_backup"], [retired])

    def test_windows_zip_and_ignored_local_state(self):
        name = "actions/.install-legacy-custom/入口.mjs"
        self.save_live(name, b"action")
        self.save_live("backups/old/bindings.json", b"previous")
        self.save_live("actions/.install-legacy-upgrade-1-2-3/action.json", b"partial")
        self.save_zip({"actions\\": b"", name.replace("/", "\\"): b"action",
                       "actions/.install-legacy-1-2-3/staged.js": b"partial"})
        self.assertTrue(self.compare()["matches"])

    def test_retired_profile_origin_is_distinct_from_local_backups(self):
        self.save_live("bindings.json", b"[]")
        origin = "profiles/tabcdef012345.copy-origin.json"
        self.save_zip({"bindings.json": b"[]", origin: b'"custom"',
                       "backups/seed-source.json": b"unrelated",
                       "profiles/other.copy-origin.json": b"unrelated",
                       "profiles/t0123456789ab/backups/profile-reset/seed-source.json": b"old"})
        self.assertEqual(self.compare()["only_in_backup"], [origin])

    def test_state_documents_are_compared_per_profile(self):
        document = "a" * 64 + ".json"
        daily = "action-state/" + document
        trial = self.test_dir.relative_to(self.root).as_posix() + "/" + daily
        retired = "profiles/tabcdef012345/" + daily
        self.save_live(daily, b'{"count":1}')
        self.save_live(trial, b'{"count":2}')
        self.save_live(daily + ".23.0.tmp", b"staging")
        self.save_live(daily.removesuffix(".json") + ".lock", b"")
        self.save_live("action-state/scratch/" + document, b"old")
        self.save_zip({daily.upper(): b'{"count":1}', trial: b'{"count":2}',
                       daily.replace(".json", ".jſon"): b"unrelated",
                       "action-state/notes.json": b"notes", "action-state/": b""})
        self.assertTrue(self.compare()["matches"])
        self.save_zip({trial: b'{"count":1}', retired: b'{"count":5}'})
        result = self.compare()
        self.assertFalse(result["matches"])
        self.assertEqual(result["missing_from_backup"], [daily])
        self.assertEqual(result["changed"], [trial])
        self.assertEqual(result["only_in_backup"], [retired])

    def test_state_document_cannot_be_a_directory(self):
        doc = self.test_dir.relative_to(self.root).as_posix() + "/action-state/" + "a" * 64 + ".json"
        self.save_live("bindings.json", b"[]")
        for entry in (doc + "/", doc + "/child", doc.rsplit("/", 1)[0]):
            with self.subTest(entry=entry):
                self.save_zip({"bindings.json": b"[]", entry: b""})
                with self.assertRaisesRegex(ValueError, "扩展状态"):
                    self.compare()
        (self.root / doc).mkdir(parents=True)
        self.save_zip({"bindings.json": b"[]"})
        with self.assertRaisesRegex(ValueError, "文件类型不符"):
            self.compare()

    def test_rejects_ambiguous_zip_and_empty_comparison(self):
        self.save_zip({"bindings.json": b"one", "Bindings.json": b"two"})
        with self.assertRaisesRegex(ValueError, "重复"):
            self.compare()
        self.save_zip({"logs/example.txt": b"log"})
        with self.assertRaisesRegex(ValueError, "均未找到"):
            self.compare()

    def test_concurrent_edit_invalidates_result(self):
        self.save_live("bindings.json", b"before")
        self.save_zip({"bindings.json": b"before"})
        original = backup.digest

        def edit_after_hash(stream):
            result = original(stream)
            if isinstance(stream, zipfile.ZipExtFile):
                self.save_live("wheels.json", b"new")
            return result

        with patch.object(backup, "digest", side_effect=edit_after_hash):
            with self.assertRaisesRegex(ValueError, "比对期间"):
                self.compare()

    @unittest.skipUnless(os.getenv("OMNITRIGGER_BACKUP_CHECK_APP"), "需要已构建的 OmniTrigger")
    def test_cli_uses_installed_profile_catalog(self):
        app = os.environ["OMNITRIGGER_BACKUP_CHECK_APP"]
        self.root = self.root / "isolated"
        self.root.mkdir()

        def create(alias, seed, *args):
            output = subprocess.run(
                [app, "--data-root", str(self.root), "--profile-create", alias, "--seed", seed, *args],
                capture_output=True, timeout=30, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
            self.assertEqual(output.returncode, 0, output.stderr)
            return "profiles/" + json.loads(output.stdout)["id"]

        bindings = b'{"version":1,"bindings":[]}'
        self.save_live("bindings.json", bindings)
        source = create("daily-copy", "copy:default", "--name", "日常配置副本")
        self.save_live(source + "/backups/edits/bindings.json/edit-0000000000000001.json", bindings)
        restored = create("from-history", "backup:daily-copy:1:current")
        files = {path.relative_to(self.root).as_posix(): path.read_bytes()
                 for path in self.root.rglob("*") if path.is_file() and path.suffix == ".json"}
        args = ["--app", app, "--data-root", str(self.root), "--archive", str(self.zip_path)]
        self.save_zip(files)
        with contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(backup.main(args), 0)
            result = json.loads(log.getvalue())
            self.assertTrue(result["matches"])
            self.assertEqual(result["checked_profiles"], 3)
        origin = source + ".copy-origin.json"
        seed = restored + "/backups/seed-source.json"
        revisions = restored + "/backups/restored-revisions.json"
        self.assertEqual(json.loads(files.pop(origin)), "custom")
        self.assertIn(seed, files)
        self.assertIn(revisions, files)
        files.pop(seed)
        files[revisions] = b"{}"
        self.save_zip(files)
        with contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(backup.main(args), 1)
            result = json.loads(log.getvalue())
            self.assertCountEqual(result["missing_from_backup"], [origin, seed])
            self.assertEqual(result["changed"], [revisions])
        self.zip_path.write_bytes(b"invalid zip")
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(backup.main(args), 2)
