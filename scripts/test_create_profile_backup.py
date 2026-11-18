import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import create_profile_backup as backup


class ExportTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name) / "data"
        self.trial = self.root / "profiles/t0123456789ab"
        self.trial.mkdir(parents=True)
        self.out = self.root / "backups/out.zip"
        self.hotkeys = self.save(self.root / "bindings.json", b'{"version":1,"bindings":[]}')

    def save(self, path, data):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def export(self):
        with patch.object(backup, "profile_dirs", return_value=[self.root, self.trial]):
            return backup.create_backup(None, self.root, self.out)

    def runtime(self):
        runtime = self.root / "runtimes/script"
        package = runtime / "packages" / ("1" * 64)
        host = self.save(package / "host.exe", b"synthetic host")
        manifest = json.dumps({"files": [{"path": "host.exe", "size": host.stat().st_size,
                                      "sha256": hashlib.sha256(host.read_bytes()).hexdigest()}]}).encode()
        self.save(package / "manifest.json", manifest)
        self.save(runtime / "current.json", json.dumps({"schema_version": 1, "archive_sha256": package.name,
                  "manifest_sha256": hashlib.sha256(manifest).hexdigest()}).encode())
        return host

    def test_restore_scope_preserves_bytes_and_edit_times(self):
        host = self.runtime()
        self.save(host.parent / "scratch.txt", b"not registered")
        self.save(host.parents[1] / ("2" * 64) / "host.exe", b"inactive")
        self.save(self.root / "clipboard-history.json", b"history")
        self.save(self.root / "backups/previous.zip", b"previous")
        self.save(self.root / "actions/.install-legacy-1-2-3/stage.js", b"partial")
        action = self.save(self.trial / "actions/sample/main.mjs", b"managed action")
        empty = action.parent / "assets"
        empty.mkdir()
        origin = self.save(self.trial.with_suffix(".copy-origin.json"), b'"custom"')
        state = self.save(self.trial / "action-state" / ("a" * 64 + ".json"), b"saved")
        revision = self.save(self.root / "backups/edits/wheels.json/edit-0000000000000001.json", b"older")
        self.save(revision.with_name("edit-0000000000000000.json"), b"invalid")
        os.utime(revision, (1700000000, 1700000000))
        result = self.export()
        with zipfile.ZipFile(self.out) as archive:
            expected = [self.hotkeys, action, origin, state, revision, host, host.parent / "manifest.json",
                        self.root / "runtimes/script/current.json"]
            entries = [i.filename for i in archive.infolist() if not i.is_dir()]
            self.assertCountEqual(entries, [p.relative_to(self.root).as_posix() for p in expected])
            self.assertTrue(archive.getinfo(empty.relative_to(self.root).as_posix() + "/").is_dir())
            self.assertEqual(result["files"], len(entries))
            for path in expected:
                self.assertEqual(archive.read(path.relative_to(self.root).as_posix()), path.read_bytes())
            self.assertEqual(archive.getinfo(revision.relative_to(self.root).as_posix()).date_time,
                             zipfile.ZipInfo.from_file(revision).date_time)
        original = self.out.read_bytes()
        with self.assertRaises(FileExistsError):
            self.export()
        self.assertEqual(self.out.read_bytes(), original)

    def test_corrupt_runtime_or_concurrent_edit_never_publishes_partial_archive(self):
        host = self.runtime()
        original = host.read_bytes()
        host.write_bytes(b"x" * len(original))
        with self.assertRaisesRegex(ValueError, "摘要"):
            self.export()
        self.assertFalse(self.out.exists())
        host.write_bytes(original)
        write = backup.write_archive

        def edit_after_write(*args):
            write(*args)
            self.hotkeys.write_bytes(b"new edit")

        with patch.object(backup, "write_archive", side_effect=edit_after_write):
            with self.assertRaisesRegex(ValueError, "变化"):
                self.export()
        self.assertEqual(list(self.out.parent.iterdir()), [])

    def test_existing_backup_wins_a_publication_race(self):
        verify = backup.verify_archive

        def other_export(*args):
            verify(*args)
            self.out.write_bytes(b"another completed backup")

        with patch.object(backup, "verify_archive", side_effect=other_export):
            with self.assertRaises(FileExistsError):
                self.export()
        self.assertEqual(self.out.read_bytes(), b"another completed backup")

    def test_pending_import_is_not_exported_as_a_complete_profile(self):
        self.save(self.trial / "backups/external-import.pending/plan.json", b"{}")
        with self.assertRaisesRegex(ValueError, "未完成"):
            self.export()
        self.assertFalse(self.out.exists())

    @unittest.skipUnless(os.getenv("OMNITRIGGER_BACKUP_CHECK_APP"), "需要已构建的 OmniTrigger")
    def test_installed_app_can_restore_an_edit_from_exported_zip(self):
        app = os.environ["OMNITRIGGER_BACKUP_CHECK_APP"]
        root = self.root.parent / "isolated"
        self.save(root / "bindings.json", self.hotkeys.read_bytes())
        revision = self.save(root / "backups/edits/bindings.json/edit-0000000000000001.json",
                             self.hotkeys.read_bytes() + b"\n")

        def create(directory, alias, seed):
            result = subprocess.run([app, "--data-root", str(directory), "--profile-create", alias, "--seed", seed],
                                    timeout=30, capture_output=True, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)

        trial = create(root, "trial", "copy:default")
        backup.create_backup(app, root, self.out)
        restored = self.root.parent / "restored"
        with zipfile.ZipFile(self.out) as archive:
            archive.extractall(restored)
        dirs = backup.profile_dirs(app, restored)
        self.assertEqual([directory.name for directory in dirs[1:]], [trial["id"]])
        recovered = create(restored, "recovered", "backup:default:1:current")
        self.assertEqual((Path(recovered["directory"]) / "bindings.json").read_bytes(), revision.read_bytes())
