"""纯合成配置投影测试；不运行应用、不读取来源或个人配置。"""
import copy
import contextlib
import io
import json
import pathlib
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock

from mouse_snapshot_projection import MINIMUM_INPUT_VERSION, SNAPSHOT_CONFIG_VERSION, ModifierFlags, PROFILES, main, parser, project_mouse_snapshots


def synthetic_document():
    bindings = []

    def append(held, activation, matching):
        number = len(bindings)
        bindings.append({"id": number + 1, "held": held, "activation": activation, "keyboard": {"matching": matching},
                         "process_scope": {"allow": [f"synthetic-{number % 3}"], "identity": "executable"},
                         "pointer_scope": {"deny": ["synthetic-denied"], "identity": "shell_context"},
                         "conditions": [{"kind": "pointer_region", "region": {"kind": "screen_boundary", "edge": "top"}}],
                         "enabled": False, "message": "合成候选", "action": {"kind": "invoke", "package_id": "synthetic", "action_id": f"item-{number % 50}", "action_version": 1, "input": {"synthetic": True}}})

    for index in range(186):
        append([{"kind": "key", "value": 20}], {"kind": "press", "value": {"kind": "key", "value": 65}}, {"kind": "captured", "priority": index})
    for priority in [0, 1, 4, 5, 6, 7, 8, 9, 10]:
        append([], {"kind": "wheel_up"}, {"kind": "mouse_unmodified", "priority": priority, "pointer_scope_priority": False})
    for key, priority, activation in [(17, 2, {"kind": "wheel_up"}), (17, 3, {"kind": "wheel_down"}), (17, 13, {"kind": "press", "value": {"kind": "middle"}}), (17, 17, {"kind": "press", "value": {"kind": "x1"}}), (164, 19, {"kind": "press", "value": {"kind": "x2"}}), (17, 20, {"kind": "press", "value": {"kind": "x2"}})]:
        append([{"kind": "key", "value": key}], activation, {"kind": "mouse_exact", "priority": priority, "pointer_scope_priority": False})
    for priority in [11, 12, 18, 21]:
        append([{"kind": "key", "value": 20}], {"kind": "wheel_up"}, {"kind": "mouse_captured_leader", "priority": priority, "pointer_scope_priority": False})
    for activation in [{"kind": "wheel_up"}, {"kind": "wheel_down"}, {"kind": "press", "value": {"kind": "right"}}]:
        append([{"kind": "x1"}], activation, {"kind": "exact"})
    assert len(bindings) == 208
    return {"version": MINIMUM_INPUT_VERSION, "bindings": bindings}


class SnapshotProjectionTests(unittest.TestCase):
    def test_profiles_preserve_208_shapes_50_unique_actions_and_original_order(self):
        source = synthetic_document()
        original = copy.deepcopy(source)
        for flags in PROFILES.values():
            projected, report = project_mouse_snapshots(source, modifier_flags=flags)
            self.assertEqual(projected["version"], SNAPSHOT_CONFIG_VERSION)
            self.assertEqual(len(projected["bindings"]), 208)
            self.assertEqual(len(report["changed"]), 15)
            self.assertEqual(report["sampled_count"], 15)
            self.assertEqual(report["sampled_unmodified_count"], 9)
            self.assertFalse(report["source_equivalence"])
            self.assertTrue(report["actions_preserved"])
            self.assertEqual(len(report["potential_prefix_entry_relations"]), 3)
            self.assertTrue(all(item["relation"] == "conditional_preemption" for item in report["potential_prefix_entry_relations"]))
            actions = {json.dumps(binding["action"], sort_keys=True) for binding in projected["bindings"]}
            self.assertEqual(len(actions), 50)
            for before, after in zip(source["bindings"], projected["bindings"]):
                self.assertEqual(before["action"], after["action"])
                self.assertEqual(before["activation"], after["activation"])
                self.assertEqual(before["enabled"], after["enabled"])
                for field in ["priority", "pointer_scope_priority"]:
                    self.assertEqual(before["keyboard"]["matching"].get(field), after["keyboard"]["matching"].get(field))
                remaining_before, remaining_after = copy.deepcopy(before), copy.deepcopy(after)
                for remaining in [remaining_before, remaining_after]:
                    remaining.pop("held")
                    remaining["keyboard"].pop("matching")
                self.assertEqual(remaining_before, remaining_after)
            self.assertEqual(source, original)

    def test_six_modifier_held_sets_move_explicitly_while_owned_mouse_and_capture_rules_stay(self):
        source = synthetic_document()
        projected, report = project_mouse_snapshots(source, modifier_flags=PROFILES["required-only"])
        changed = {change["id"]: change for change in report["changed"]}
        self.assertEqual(sum(change["required"] == "control" for change in changed.values()), 5)
        self.assertEqual(sum(change["required"] == "left_alt" for change in changed.values()), 1)
        for before, after in zip(source["bindings"], projected["bindings"]):
            if before["id"] in changed:
                self.assertEqual(after["held"], [])
            else:
                self.assertEqual(before, after)

    def test_unmodified_capabilities_are_preserved_and_custom_flags_are_explicit(self):
        source = synthetic_document()
        source["bindings"][186]["keyboard"]["matching"]["kind"] = "mouse_exact"
        projected, _ = project_mouse_snapshots(source, modifier_flags=ModifierFlags(True, False, True))
        strict = projected["bindings"][186]["keyboard"]["matching"]["policy"]
        passed = projected["bindings"][187]["keyboard"]["matching"]["policy"]
        self.assertEqual([strict[key] for key in ["allow_other_modifiers", "allow_extra_passed_keys", "allow_extra_passed_mouse"]], [False, False, False])
        self.assertEqual([passed[key] for key in ["allow_other_modifiers", "allow_extra_passed_keys", "allow_extra_passed_mouse"]], [True, True, True])
        overridden, _ = project_mouse_snapshots(source, modifier_flags=PROFILES["required-only"], unmodified_flags=ModifierFlags(True, False, False))
        self.assertFalse(overridden["bindings"][187]["keyboard"]["matching"]["policy"]["allow_extra_passed_keys"])

    def test_reprojection_is_idempotent_and_only_explicit_modifier_flags_change(self):
        first, _ = project_mouse_snapshots(synthetic_document(), modifier_flags=PROFILES["required-only"])
        again, report = project_mouse_snapshots(first, modifier_flags=PROFILES["required-only"])
        self.assertEqual(first, again)
        self.assertEqual(report["changed"], [])
        changed, report = project_mouse_snapshots(first, modifier_flags=PROFILES["passed-extras"])
        self.assertEqual(len(report["changed"]), 6)
        self.assertEqual(changed["bindings"][186], first["bindings"][186])
        first["bindings"][0]["id"] = 0
        again, _ = project_mouse_snapshots(first, modifier_flags=PROFILES["required-only"])
        self.assertEqual(again["bindings"][0]["id"], 0)

    def test_invalid_flags_unknown_fields_and_keyboard_timing_are_not_silently_dropped(self):
        source = synthetic_document()
        for flags in [None, ModifierFlags(0, True, False), ModifierFlags(False, None, True)]:
            with self.assertRaises(ValueError):
                project_mouse_snapshots(source, modifier_flags=flags)
        for mutate in [
            lambda binding: binding["keyboard"].update({"repeat": 0}),
            lambda binding: binding["keyboard"].update({"tap_timeout_ms": 0}),
            lambda binding: binding["keyboard"]["matching"].update({"priority": True}),
            lambda binding: binding["keyboard"]["matching"].update({"unknown": True}),
            lambda binding: binding["held"][0].update({"unknown": True}),
        ]:
            broken = synthetic_document()
            mutate(broken["bindings"][195])
            with self.assertRaises(ValueError):
                project_mouse_snapshots(broken, modifier_flags=PROFILES["required-only"])
        sampled, _ = project_mouse_snapshots(source, modifier_flags=PROFILES["required-only"])
        sampled["bindings"][195]["keyboard"]["matching"]["policy"]["unknown"] = True
        with self.assertRaises(ValueError):
            project_mouse_snapshots(sampled, modifier_flags=PROFILES["required-only"])
        sampled["bindings"][195]["keyboard"]["matching"]["policy"].pop("unknown")
        sampled["bindings"][195]["keyboard"]["matching"]["policy"]["allow_other_modifiers"] = 0
        with self.assertRaises(ValueError):
            project_mouse_snapshots(sampled, modifier_flags=PROFILES["passed-extras"])

    def test_unsupported_modifier_and_existing_owned_mouse_are_preserved(self):
        source = synthetic_document()
        source["bindings"][195]["held"][0]["value"] = 16
        projected, report = project_mouse_snapshots(source, modifier_flags=PROFILES["passed-extras"])
        self.assertEqual(projected["bindings"][195], source["bindings"][195])
        self.assertIn(source["bindings"][195]["id"], report["unsupported_mouse_candidates_preserved"])
        self.assertEqual(projected["bindings"][-3:], source["bindings"][-3:])

    def test_cli_requires_policy_and_custom_flags_without_reading_input(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser().parse_args(["--input", "synthetic.json", "--output", "candidate.json"])
        with self.assertRaisesRegex(ValueError, "custom"):
            main(["--input", "synthetic.json", "--output", "candidate.json", "--policy", "custom"])
        with self.assertRaisesRegex(ValueError, "命名策略"):
            main(["--input", "synthetic.json", "--output", "candidate.json", "--policy", "required-only", "--allow-other-modifiers"])

    def test_cli_only_creates_new_output_and_rejects_duplicate_json_keys(self):
        temporary_root = pathlib.Path(tempfile.gettempdir()).resolve()
        with tempfile.TemporaryDirectory(prefix="omnitrigger-mouse-projection-") as name:
            directory = pathlib.Path(name).resolve()
            self.assertEqual(directory.parent, temporary_root)
            self.assertTrue(directory.name.startswith("omnitrigger-mouse-projection-"))
            input_path, output_path = directory / "input.json", directory / "candidate.json"
            original = json.dumps(synthetic_document(), ensure_ascii=False).encode("utf-8")
            input_path.write_bytes(original)
            arguments = ["--input", str(input_path), "--output", str(output_path), "--policy", "required-only"]
            with contextlib.redirect_stdout(io.StringIO()) as captured:
                self.assertEqual(main(arguments), 0)
            self.assertEqual(json.loads(captured.getvalue())["bindings"], 208)
            result = output_path.read_bytes()
            with self.assertRaises(FileExistsError):
                main(arguments)
            self.assertEqual(output_path.read_bytes(), result)
            self.assertEqual(input_path.read_bytes(), original)
            custom_path = directory / "custom.json"
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main(["--input", str(input_path), "--output", str(custom_path), "--policy", "custom", "--no-allow-other-modifiers", "--no-allow-extra-passed-keys", "--allow-extra-passed-mouse"]), 0)
            custom = json.loads(custom_path.read_text(encoding="utf-8"))["bindings"][195]["keyboard"]["matching"]["policy"]
            self.assertEqual([custom[key] for key in ["allow_other_modifiers", "allow_extra_passed_keys", "allow_extra_passed_mouse"]], [False, False, True])
            with self.assertRaisesRegex(ValueError, "覆盖"):
                main(["--input", str(input_path), "--output", str(input_path), "--policy", "required-only"])
            duplicate = directory / "duplicate.json"
            duplicate.write_text('{"version":22,"version":23,"bindings":[]}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "重复"):
                main(["--input", str(duplicate), "--output", str(directory / "unused.json"), "--policy", "required-only"])
            self.assertFalse((directory / "unused.json").exists())

    def test_placeholder_and_reparse_attributes_reject_before_resolving_or_reading(self):
        for attributes in [0x1000, 0x40000, 0x400000, 0x400]:
            with mock.patch.object(pathlib.Path, "lstat", return_value=SimpleNamespace(st_file_attributes=attributes)), mock.patch.object(pathlib.Path, "resolve", side_effect=AssertionError("不得提前解析占位文件")), mock.patch.object(pathlib.Path, "read_text", side_effect=AssertionError("不得读取占位内容")):
                with self.assertRaisesRegex(ValueError, "占位"):
                    main(["--input", "synthetic.json", "--output", "candidate.json", "--policy", "required-only"])

    def test_supported_versions_follow_declared_bounds_and_future_version_is_relative(self):
        for version in range(MINIMUM_INPUT_VERSION, SNAPSHOT_CONFIG_VERSION + 1):
            document = synthetic_document()
            document["version"] = version
            result, _ = project_mouse_snapshots(document, modifier_flags=PROFILES["required-only"])
            self.assertEqual(result["version"], SNAPSHOT_CONFIG_VERSION)
        for invalid in [MINIMUM_INPUT_VERSION - 1, SNAPSHOT_CONFIG_VERSION + 1, True, float(SNAPSHOT_CONFIG_VERSION), None]:
            document = synthetic_document()
            document["version"] = invalid
            with self.assertRaises(ValueError):
                project_mouse_snapshots(document, modifier_flags=PROFILES["required-only"])


if __name__ == "__main__":
    unittest.main()
