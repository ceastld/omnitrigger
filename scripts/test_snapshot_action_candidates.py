"""合成目录验证快照与静态模块检查；不调用动作入口或系统 API。"""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest

from snapshot_action_candidates import (SnapshotError, guard, relative, snapshot, verify_snapshot)

CHECKER = Path(__file__).with_name("check_action_modules.mjs")


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value) if isinstance(value, dict) else value, encoding="utf-8")


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)
        self.packages = self.root / "examples"
        self.package = self.packages / "demo"
        self.candidates = self.root / "calls"
        self.reference = self.root / "reference.json"
        self.output = self.root / "new-snapshot"
        self.manifest = {"schema_version":1,"id":"synthetic.demo","actions":[
            {"id":"run","version":1,"runtime":"node","entry":"main.mjs"}]}
        write(self.package / "action.json", self.manifest)
        write(self.package / "main.mjs", "import {value} from './lib/timing.mjs';\nthrow Error('INITIALIZER MUST NOT RUN');\nexport default () => value;\n")
        write(self.package / "lib/timing.mjs", "export const value = 42;\n")
        write(self.package / "README.md", "complete package fixture\n")
        call = {"package_id":"synthetic.demo","action_id":"run","action_version":1,"input":{}}
        write(self.candidates / "one.json", call)
        write(self.candidates / "plugin.json", {"kind":"plugin","pluginId":"synthetic","actionId":"capture"})
        digest = hashlib.sha256((self.candidates / "one.json").read_bytes()).hexdigest()
        write(self.reference, {"passed":True,"results":[{"source_id":"one","kind":"ActionCall","call_sha256":digest},
            {"source_id":"plugin","kind":"ActionSpec"}]})

    def build(self):
        return snapshot(self.candidates, self.reference, self.packages, self.output)

    def modules(self, *, trusted=False, name="module-check.json"):
        return subprocess.run(["node","--experimental-vm-modules",str(CHECKER),
            "--snapshot",str(self.output / "snapshot.json"),"--out",str(self.output / name),
            *(["--import-trusted-project"] if trusted else [])], capture_output=True, text=True,
            encoding="utf-8", errors="replace")

    def test_full_copy_keeps_modules_docs_and_distinguishes_plugin_specs(self):
        before = (self.reference.read_bytes(), (self.package / "main.mjs").read_bytes())
        report = self.build()
        self.assertEqual(len(report["candidates"]), 2)
        self.assertEqual(len(report["packages"]), 1)
        self.assertEqual((self.output / "packages/demo/lib/timing.mjs").read_bytes(),
                         (self.package / "lib/timing.mjs").read_bytes())
        self.assertTrue((self.output / "packages/demo/README.md").is_file())
        plugin = next(row for row in report["candidates"] if row["kind"] == "ActionSpec")
        self.assertEqual(plugin["module_check"], "not_applicable")
        self.assertFalse(report["actions_executed"])
        verify_snapshot(self.output / "snapshot.json")
        self.assertEqual(before, (self.reference.read_bytes(), (self.package / "main.mjs").read_bytes()))

    def test_existing_output_and_nested_output_never_get_overwritten(self):
        self.output.mkdir()
        write(self.output / "evidence.txt", "original")
        with self.assertRaises(SnapshotError): self.build()
        self.assertEqual((self.output / "evidence.txt").read_text(), "original")
        with self.assertRaises(SnapshotError): snapshot(self.candidates,self.reference,self.packages,self.package / "nested")

    def test_reference_hash_collection_and_version_mismatch_fail_before_creation(self):
        changes = [lambda: write(self.candidates / "extra.json", {}),
                   lambda: write(self.candidates / "one.json", {}),
                   lambda: write(self.package / "action.json", {**self.manifest,"actions":[
                       {**self.manifest["actions"][0],"version":2}]})]
        for change in changes:
            with self.subTest(change=changes.index(change)):
                old_call = (self.candidates / "one.json").read_bytes()
                old_manifest = (self.package / "action.json").read_bytes()
                change()
                with self.assertRaises(SnapshotError): self.build()
                self.assertFalse(self.output.exists())
                (self.candidates / "extra.json").unlink(missing_ok=True)
                (self.candidates / "one.json").write_bytes(old_call)
                (self.package / "action.json").write_bytes(old_manifest)

    def test_path_traversal_absolute_backslash_and_stream_paths_are_rejected(self):
        for path in ["../x","/x","C:/x","x\\y","./x","x//y","x/../y","x?query","x#hash",""]:
            with self.subTest(path=path), self.assertRaises(SnapshotError): relative(path)
        bad = copy.deepcopy(self.manifest)
        bad["actions"][0]["entry"] = "../private.mjs"
        write(self.package / "action.json", bad)
        with self.assertRaises(SnapshotError): self.build()

    def test_cloud_and_reparse_metadata_are_refused_without_opening_files(self):
        for flag in [0x400,0x1000,0x40000,0x400000]:
            metadata = lambda _: SimpleNamespace(st_mode=stat.S_IFREG,st_file_attributes=flag)
            with self.subTest(flag=flag), self.assertRaises(SnapshotError):
                guard(self.package / "file", metadata=metadata)
        for cloud in ["OneDrive - Work","Dropbox","iCloud Drive"]:
            with self.assertRaises(SnapshotError): guard(self.root / cloud / "not-opened", missing=True)

    def test_symbolic_link_inside_package_is_never_followed(self):
        target = self.root / "outside.txt"
        write(target, "must not copy")
        try: os.symlink(target, self.package / "linked.txt")
        except OSError as error:
            if getattr(error,"winerror",None) == 1314: self.skipTest("系统未授权创建合成符号链接；reparse 元数据测试仍执行")
            raise
        with self.assertRaises(SnapshotError): self.build()
        self.assertFalse(self.output.exists())

    def test_hard_link_is_rejected_without_reading_link_target(self):
        target = self.root / "outside.txt"
        write(target, "must not copy")
        os.link(target, self.package / "hard-linked.txt")
        with self.assertRaises(SnapshotError): self.build()
        self.assertFalse(self.output.exists())

    def test_missing_manifest_entry_is_rejected_before_output_creation(self):
        (self.package / "main.mjs").unlink()
        with self.assertRaises(SnapshotError): self.build()
        self.assertFalse(self.output.exists())

    def test_tampered_snapshot_and_source_fail_integrity_checks(self):
        self.build()
        verify_snapshot(self.output / "snapshot.json", self.packages)
        write(self.package / "lib/timing.mjs", "changed")
        with self.assertRaises(SnapshotError): verify_snapshot(self.output / "snapshot.json",self.packages)
        write(self.output / "candidates/one.json", {})
        with self.assertRaises(SnapshotError): verify_snapshot(self.output / "snapshot.json")

    @unittest.skipUnless(shutil.which("node"), "需 Node，仅解析合成模块")
    def test_static_module_graph_does_not_execute_throwing_initializer(self):
        self.build()
        result = self.modules()
        self.assertEqual(result.returncode,0,result.stderr)
        report = json.loads((self.output / "module-check.json").read_text(encoding="utf-8"))
        self.assertTrue(report["static_graph_passed"])
        self.assertFalse(report["trusted_initializers_evaluated"])
        self.assertFalse(report["entry_functions_called"])
        self.assertEqual(report["packages"][0]["edges"],[{"from":"main.mjs","to":"lib/timing.mjs"}])

    @unittest.skipUnless(shutil.which("node"), "需 Node，仅解析合成模块")
    def test_entry_only_copy_is_not_a_complete_module_graph(self):
        self.build()
        report_path = self.output / "snapshot.json"
        value = json.loads(report_path.read_text(encoding="utf-8"))
        (self.output / "packages/demo/lib/timing.mjs").unlink()
        value["packages"][0]["files"] = [row for row in value["packages"][0]["files"] if row["path"] != "lib/timing.mjs"]
        write(report_path, value)
        result = self.modules()
        self.assertEqual(result.returncode,1)
        result = json.loads((self.output / "module-check.json").read_text(encoding="utf-8"))
        self.assertFalse(result["static_graph_passed"])
        self.assertEqual(result["packages"][0]["errors"][0]["code"],"missing_or_unsupported_module")

    @unittest.skipUnless(shutil.which("node"), "需 Node，禁止导入未受信合成模块")
    def test_trusted_import_flag_cannot_authorize_arbitrary_fixture_root(self):
        self.build()
        result = self.modules(trusted=True)
        self.assertNotEqual(result.returncode,0)
        self.assertFalse((self.output / "module-check.json").exists())

    @unittest.skipUnless(shutil.which("node"), "需 Node，仅解析合成模块")
    def test_static_dependencies_reject_outside_and_percent_encoded_paths(self):
        for index, specifier in enumerate(["../outside.mjs","./%2e%2e/outside.mjs"]):
            self.output = self.root / f"snapshot-{index}"
            write(self.package / "main.mjs", f"import '{specifier}'; export default () => null;")
            self.build()
            result = self.modules()
            self.assertEqual(result.returncode,1)
            report = json.loads((self.output / "module-check.json").read_text(encoding="utf-8"))
            self.assertFalse(report["static_graph_passed"])
            self.assertFalse(report["trusted_initializers_evaluated"])


if __name__ == "__main__": unittest.main()
