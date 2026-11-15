import json
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path
from unittest.mock import patch

from migrate_wheel_action import main, migrate


OLD = {"kind": "invoke", "package_id": "old", "action_id": "toggle", "action_version": 1, "input": {"flag": True}}
NEW = {"kind": "invoke", "package_id": "local", "action_id": "toggle", "action_version": 1, "input": {"profile": "daily"}}
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
        wheels = {"version": 6, "wheels": [wheel, dict(wheel, id="other")]}
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
        source = json.loads(json.dumps({"version": 3, "wheels": [wheel]}))
        before = deepcopy(source)
        result, changed = migrate(BINDINGS, source, 9, "w", 2)
        self.assertEqual(changed, [None, "two"])
        self.assertEqual(result["wheels"][0]["app_overrides"][1]["slots"][0]["action"], OLD)
        self.assertEqual(source, before)

    def test_legacy_binding_notifications_and_commands_keep_slot_labels_and_arguments(self):
        command = {"program": "历史 工具.exe", "args": ["", "含 空格", "--mode=编辑"],
                   "working_directory": "D:/测试 数据"}
        base = {"label": "通用工具", "action": OLD}
        override = dict(base, label="编辑器工具")
        source = {"version": 3, "wheels": [{"id": "w", "slots": [base],
                  "app_overrides": [{"process": "editor.exe", "slots": [override]}]}]}
        for version in range(1, 5):
            for fields, expected in [
                ({}, {"kind": "notification"}),
                ({"action": None, "command": None, "show_panel": False}, {"kind": "notification"}),
                ({"command": command}, {"kind": "command", **command}),
                ({"show_panel": False, "command": {"program": "工具.exe"}}, {"kind": "command", "program": "工具.exe"}),
            ]:
                with self.subTest(version=version, fields=fields):
                    binding = {"id": 9, "message": "热键名称", **fields}
                    result, changed = migrate({"version": version, "bindings": [binding]}, source, 9, "w", 1)
                    wheel = result["wheels"][0]
                    self.assertEqual(wheel["slots"], [dict(base, action=expected)])
                    self.assertEqual(wheel["app_overrides"][0]["slots"], [dict(override, action=expected)])
                    self.assertEqual(changed, [None, "editor.exe"])
                    self.assertEqual(base["action"], OLD)

    def test_invalid_or_panel_bindings_never_create_a_candidate(self):
        with tempfile.TemporaryDirectory() as root:
            bindings, wheels, output = (Path(root, name) for name in ("bindings.json", "wheels.json", "out.json"))
            wheels.write_text(json.dumps({"version": 3, "wheels": [{"id": "w", "slots": [{"action": OLD}]}]}))
            flags = ["--bindings", str(bindings), "--wheels", str(wheels), "--binding-id", "9",
                     "--wheel-id", "w", "--base-slot", "1", "--output", str(output)]
            for version, fields in [
                (1, {"show_panel": True}), (2, {"show_panel": True, "command": {"program": "tool"}}),
                (3, {"show_panel": 1}), (4, {"action": NEW}),
                (4, {"command": {"program": "tool", "kind": "notification"}}),
                (5, {}), (5, {"action": NEW, "show_panel": False}), (24, {"command": {"program": "tool"}}),
            ]:
                with self.subTest(version=version, fields=fields), patch("sys.stderr"):
                    bindings.write_text(json.dumps({"version": version, "bindings": [{"id": 9, **fields}]}))
                    self.assertEqual(main(flags), 1)
                    self.assertFalse(output.exists())

    def test_candidate_uses_the_minimum_version_for_the_copied_action(self):
        shortcut = {"kind": "send_shortcut", "keys": [162, 67]}
        for action, required in [
            (shortcut, 1),
            (dict(shortcut, timing={"key_down_delay_ms": 0, "process_overrides": {}}), 1),
            ({"kind": "local", "version": 1, "steps": [{"kind": "delay", "milliseconds": 50}]}, 2),
            (NEW, 3),
            ({"kind": "browser", "command": {"operation": "reload_tab"}}, 4),
            (dict(shortcut, timing={"key_down_delay_ms": 20}), 5),
            (dict(shortcut, timing={"process_overrides": {"editor.exe": 0}}), 5),
        ]:
            for version in range(1, 7):
                with self.subTest(action=action, version=version):
                    source = {"version": version, "wheels": [{"id": "w", "slots": [{"action": {"kind": "notification"}}]}]}
                    result, changed = migrate({"bindings": [{"id": 1, "action": action}]}, source, 1, "w", 1)
                    self.assertEqual((result["version"], changed), (max(version, required), [None]))
                    self.assertEqual(source["version"], version)

        for version in (0, 7, True, "1"):
            with self.subTest(unsupported_version=version), self.assertRaisesRegex(ValueError, "轮盘配置版本"):
                migrate(BINDINGS, {"version": version, "wheels": []}, 9, "w", 1)

    def test_existing_output_is_preserved(self):
        with tempfile.TemporaryDirectory() as root:
            bindings, wheels = Path(root, "bindings.json"), Path(root, "wheels.json")
            bindings.write_text(json.dumps(BINDINGS), encoding="utf-8")
            wheels.write_text(json.dumps({"version": 3, "wheels": [{"id": "w", "slots": [{"action": OLD}]}]}), encoding="utf-8")
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
