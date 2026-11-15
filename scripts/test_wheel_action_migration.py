import json
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path
from unittest.mock import patch

from migrate_wheel_action import main, migrate


OLD = {"kind": "invoke", "package_id": "old", "action_id": "toggle", "input": {"flag": True}}
NEW = {"kind": "invoke", "package_id": "local", "action_id": "toggle", "input": {"profile": "daily"}}
BINDINGS = {"bindings": [{"id": 9, "action": NEW}]}


class WheelActionMigration(unittest.TestCase):
    def test_only_matching_action_position_and_scene_in_selected_wheel_change(self):
        selected = {"label": "剪贴板", "action": OLD, "position": {"ring": 0, "sector": 3, "sectors": 8}}
        alternatives = [
            dict(selected, label="编辑器剪贴板", position=dict(selected["position"], half_step=False)),
            dict(selected, scene="work"),
            dict(selected, position={"ring": 1, "sector": 3, "sectors": 8}),
            dict(selected, action=dict(OLD, input={"flag": 1})),
        ]
        wheel = {"id": "personal", "slots": [selected], "app_overrides": [{"process": "editor.exe", "slots": alternatives}]}
        wheels = {"wheels": [wheel, dict(wheel, id="other")]}
        expected = json.loads(json.dumps(wheels))
        expected["wheels"][0]["slots"][0]["action"] = NEW
        expected["wheels"][0]["app_overrides"][0]["slots"][0]["action"] = NEW
        result, changed = migrate(BINDINGS, json.loads(json.dumps(wheels)), 9, "personal", 1)
        self.assertEqual(result, expected)
        self.assertEqual(changed, [None, "editor.exe"])
        self.assertEqual(migrate(BINDINGS, result, 9, "personal", 1), (result, []))

    def test_legacy_even_slots_require_same_count_and_index(self):
        slot = {"label": "A", "action": OLD}
        wheel = {"id": "w", "slots": [slot, slot], "app_overrides": [
            {"process": "one", "slots": [slot]}, {"process": "two", "slots": [slot, slot]}]}
        source = json.loads(json.dumps({"wheels": [wheel]}))
        before = deepcopy(source)
        result, changed = migrate(BINDINGS, source, 9, "w", 2)
        self.assertEqual(changed, [None, "two"])
        self.assertEqual(result["wheels"][0]["app_overrides"][1]["slots"][0]["action"], OLD)
        self.assertEqual(source, before)

    def test_existing_output_is_preserved(self):
        with tempfile.TemporaryDirectory() as root:
            bindings, wheels = Path(root, "bindings.json"), Path(root, "wheels.json")
            bindings.write_text(json.dumps(BINDINGS), encoding="utf-8")
            wheels.write_text(json.dumps({"wheels": [{"id": "w", "slots": [{"action": OLD}]}]}), encoding="utf-8")
            original = wheels.read_bytes()
            with patch("sys.stderr"):
                self.assertEqual(main(["--bindings", str(bindings), "--wheels", str(wheels), "--binding-id", "9",
                                       "--wheel-id", "w", "--base-slot", "1", "--output", str(wheels)]), 1)
            self.assertEqual(wheels.read_bytes(), original)

    def test_cli_preserves_bom_crlf_and_unchanged_candidate_bytes(self):
        with tempfile.TemporaryDirectory() as root:
            bindings, wheels = Path(root, "bindings.json"), Path(root, "wheels.json")
            candidate, repeat = Path(root, "candidate.json"), Path(root, "repeat.json")
            bindings.write_text(json.dumps(BINDINGS), encoding="utf-8")
            source = {"version": 6, "wheels": [{"id": "w", "slots": [{"label": "保留", "action": OLD}]}]}
            original = (json.dumps(source, ensure_ascii=False) + "\r\n").encode("utf-8-sig")
            wheels.write_bytes(original)
            flags = ["--bindings", str(bindings), "--binding-id", "9", "--wheel-id", "w", "--base-slot", "1"]
            with patch("sys.stdout"):
                self.assertEqual(main(flags + ["--wheels", str(wheels), "--output", str(candidate)]), 0)
                self.assertEqual(main(flags + ["--wheels", str(candidate), "--output", str(repeat)]), 0)
            raw = candidate.read_bytes()
            self.assertTrue(raw.startswith(b"\xef\xbb\xbf") and raw.endswith(b"\r\n"))
            self.assertNotIn(b"\n", raw.replace(b"\r\n", b""))
            self.assertEqual(json.loads(raw)["wheels"][0]["slots"][0], {"label": "保留", "action": NEW})
            self.assertEqual(repeat.read_bytes(), raw)
            self.assertEqual(wheels.read_bytes(), original)
