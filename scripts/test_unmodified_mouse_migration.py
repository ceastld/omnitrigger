"""合成快照验证显式无控制键策略；只生成离线报告，不注册或运行动作。"""

import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from mouse_action_projection import mouse_requirements, mouse_trigger
from plan_action_migration import build_plan, main, same_keyboard_policy
from test_action_migration import fixture
from test_mouse_action_migration import rule, settings


def unmodified_rules():
    # 九条独立候选：四条边缘的滚轮上下，以及底边的鼠标按下。
    return [dict(rule(kind), ControlKey=None, Location=location)
            for location, kind in [(48, 11), (48, 12), (192, 12), (196608, 11),
                                   (196608, 12), (786432, 11), (786432, 12),
                                   (192, 11), (192, 4)]]


class UnmodifiedMouseMigrationTests(unittest.TestCase):
    def test_default_and_explicit_strict_are_identical(self):
        for row in unmodified_rules() + [dict(rule(), ControlKey=0), rule()]:
            self.assertEqual(mouse_requirements(row), mouse_requirements(row, unmodified_mouse_policy="strict"))
            self.assertEqual(mouse_trigger(row, settings()),
                             mouse_trigger(row, settings(), unmodified_mouse_policy="strict"))
        data = fixture()
        data["source"]["MouseActions"] = unmodified_rules()
        implicit = build_plan(**data, mouse_settings=settings())
        self.assertEqual(implicit, build_plan(**data, mouse_settings=settings(), unmodified_mouse_policy="strict"))
        self.assertNotIn("unmodified_mouse_policy", implicit["criteria"])
        self.assertNotIn("minimum_configuration_version", implicit["criteria"])
        self.assertNotIn("mouse_unmodified_projected", implicit["summary"])
        self.assertTrue(all(row["trigger"]["keyboard"]["matching"]["kind"] == "mouse_exact"
                            for row in implicit["bindings"] if row["kind"] == "mouse"))

    def test_passed_changes_only_matching_for_none_or_integer_zero(self):
        for key in (None, 0):
            for priority in (None, 0, 511):
                row = dict(rule(), ControlKey=key, Location=48)
                before = copy.deepcopy(row)
                strict = mouse_trigger(row, settings(), priority=priority)
                passed = mouse_trigger(row, settings(), priority=priority, unmodified_mouse_policy="passed")
                expected = copy.deepcopy(strict["trigger"])
                expected["keyboard"]["matching"]["kind"] = "mouse_unmodified"
                self.assertEqual(passed["trigger"], expected)
                self.assertEqual(passed["trigger"]["held"], [])
                policy = passed["source_requirements"]["projection_policy"]
                self.assertEqual(policy["extra_held_inputs"], "passed_only")
                self.assertFalse(policy["creates_capture"])
                self.assertFalse(policy["source_equivalence"])
                self.assertEqual(policy["minimum_configuration_version"], 22)
                self.assertEqual(row, before)

    def test_other_controls_and_invalid_zero_lookalikes_keep_exact_results(self):
        values = [1, 2, 4, 5, 6, 16, 17, 18, 21, 20, 32, 91, 92,
                  160, 161, 162, 163, 164, 165, True, False, 0.0, "0", {}, -1]
        for value in values:
            with self.subTest(control=value):
                row = dict(rule(), ControlKey=value)
                self.assertEqual(mouse_trigger(row, settings()),
                                 mouse_trigger(row, settings(), unmodified_mouse_policy="passed"))
        missing = rule()
        del missing["ControlKey"]
        self.assertEqual(mouse_trigger(missing, settings()),
                         mouse_trigger(missing, settings(), unmodified_mouse_policy="passed"))

    def test_press_and_wheel_shapes_preserve_scope_and_packet_semantics(self):
        for kind in (4, 11, 12):
            row = dict(rule(kind), ControlKey=None, WhiteList=["Reader.EXE"])
            result = mouse_trigger(row, settings(), unmodified_mouse_policy="passed")
            trigger = result["trigger"]
            self.assertEqual(trigger["pointer_scope"], {"identity":"shell_context", "allow":["reader"]})
            self.assertEqual(trigger["activation"], {"kind":"press", "value":{"kind":"right"}}
                             if kind == 4 else {"kind":"wheel_up" if kind == 11 else "wheel_down"})
            self.assertEqual(trigger.get("wheel"), None if kind == 4 else "packet")
            self.assertEqual("mouse_small_vertical_delta_review" in result["trigger_gaps"], kind in (11, 12))
            self.assertNotIn("enabled", trigger)
            self.assertNotIn("trigger_dependencies", result)

    def test_passed_does_not_bypass_missing_settings_flags_or_priority_validation(self):
        base = dict(rule(), ControlKey=None)
        for row, options in [(base, None), (dict(base, DebounceMs=10), settings()),
                             (dict(base, Location=16), settings()),
                             (dict(base, IsEnabled=False), settings())]:
            strict = mouse_trigger(row, options)
            passed = mouse_trigger(row, options, unmodified_mouse_policy="passed")
            self.assertIsNone(passed["trigger"])
            self.assertEqual(passed["trigger_gaps"], strict["trigger_gaps"])
        for priority in (True, -1, 512, "1"):
            result = mouse_trigger(base, settings(), priority=priority, unmodified_mouse_policy="passed")
            self.assertIsNone(result["trigger"])
            self.assertIn("mouse_rule_order_invalid", result["trigger_gaps"])

    def test_remaining_source_differences_stay_explicit(self):
        row = mouse_trigger(dict(rule(), ControlKey=None, Location=48), settings(),
                            priority=2, unmodified_mouse_policy="passed")
        gaps = row["trigger_gaps"]
        for gap in ("mouse_unmodified_source_ownership_review", "mouse_virtual_control_state_review",
                    "mouse_other_held_keys_review", "mouse_other_held_buttons_review",
                    "mouse_queue_drop_delay_review", "mouse_builtin_rule_precedence_review",
                    "mouse_async_system_state_review", "mouse_source_v1_binary_parity_review",
                    "mouse_builtin_enablement_review", "mouse_recovery_semantics_review",
                    "mouse_physical_application_effects_unverified", "mouse_small_vertical_delta_review"):
            self.assertIn(gap, gaps)
        differences = row["source_requirements"]["semantic_differences"]
        self.assertNotIn("source_unmodified_does_not_exclude_windows_keys", differences)
        self.assertNotIn("source_checks_captured_keys_not_all_ordinary_keys", differences)
        self.assertIn("source_checks_captured_keys_not_core_ownership", differences)
        self.assertIn("source_async_modifier_sampling_unavailable", differences)
        self.assertIn("source_v1_binary_parity_unverified", differences)
        self.assertIn("source_small_vertical_delta_gate_unverified", differences)

    def test_report_wires_nine_candidates_without_claiming_equivalence(self):
        data = fixture()
        data["source"]["MouseActions"] = unmodified_rules() + [dict(rule(), ControlKey=21)]
        data["current"]["bindings"] = [{"id":900, "held":[], "activation":{"kind":"wheel_up"},
            "enabled":True, "keyboard":{"matching":{"kind":"mouse_exact"}}, "action":{"kind":"notification"}}]
        before = copy.deepcopy(data)
        plan = build_plan(**data, mouse_settings=settings(), unmodified_mouse_policy="passed")
        self.assertEqual(data, before)
        self.assertFalse(plan["complete"])
        self.assertEqual(plan["criteria"]["unmodified_mouse_policy"], "passed")
        self.assertEqual(plan["criteria"]["minimum_configuration_version"], 22)
        self.assertEqual(plan["summary"]["mouse_unmodified_projected"], 9)
        rows = [row for row in plan["bindings"] if row["kind"] == "mouse"]
        self.assertEqual(len(rows), 10)
        self.assertEqual(rows[0]["matching_trigger_configuration_ids"], [900])
        self.assertEqual(rows[-1]["trigger"]["keyboard"]["matching"]["kind"], "mouse_captured_leader")
        self.assertEqual(sorted(row["trigger"]["keyboard"]["matching"]["priority"] for row in rows), list(range(10)))
        for row in rows:
            self.assertEqual(row["status"], "needs_trigger_semantics_review")
            self.assertEqual(row["equivalent_configuration_ids"], [])
            order = [note for note in row["source_normalizations"] if note["kind"] == "mouse_rule_order"]
            self.assertEqual(len(order), 1)
            self.assertFalse(order[0]["runtime_builtin_rules_included"])
            self.assertFalse(row["source_requirements"]["projection_policy"]["source_equivalence"])
        self.assertEqual(plan["summary"]["normalizations"]["mouse_unmodified_passed_policy"], 9)

    def test_application_profiles_are_not_silently_promoted(self):
        data = fixture()
        data["source"]["ExeSettings"]["reader"] = {"MouseActions":unmodified_rules()}
        strict = build_plan(**data, mouse_settings=settings())
        passed = build_plan(**data, mouse_settings=settings(), unmodified_mouse_policy="passed")
        self.assertEqual(strict["bindings"], passed["bindings"])
        self.assertEqual(passed["summary"]["mouse_unmodified_projected"], 0)

    def test_new_mode_is_distinct_from_existing_keyboard_matching(self):
        old = mouse_trigger(dict(rule(), ControlKey=None), settings())["trigger"]["keyboard"]
        new = mouse_trigger(dict(rule(), ControlKey=None), settings(), unmodified_mouse_policy="passed")["trigger"]["keyboard"]
        self.assertFalse(same_keyboard_policy(old, new))
        self.assertTrue(same_keyboard_policy(new, copy.deepcopy(new)))

    def test_invalid_policy_is_rejected_even_without_mouse_rules(self):
        for value in (None, True, 0, "", "PASSED", " permissive ", [], {}):
            with self.subTest(policy=value):
                with self.assertRaises(ValueError):
                    mouse_trigger(dict(rule(), ControlKey=None), settings(), unmodified_mouse_policy=value)
                with self.assertRaises(ValueError):
                    build_plan(**fixture(), unmodified_mouse_policy=value)

    def test_cli_default_strict_and_passed_reports_preserve_all_inputs(self):
        data = fixture()
        data["source"]["MouseActions"] = unmodified_rules()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = []
            inputs = {}
            for name, value in {**data, "mouse_settings":settings()}.items():
                flag = {"catalog_payload":"catalog", "mouse_settings":"mouse-settings"}.get(name, name)
                path = root / (name + ".json")
                path.write_text(json.dumps(value), encoding="utf-8")
                inputs[path] = path.read_bytes()
                args += ["--" + flag, str(path)]
            reports = {}
            for policy in (None, "strict", "passed"):
                output = root / ((policy or "default") + ".json")
                extra = ["--unmodified-mouse-policy", policy] if policy else []
                with contextlib.redirect_stdout(io.StringIO()):
                    main(args + extra + ["--out", str(output)])
                reports[policy] = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(reports[None], reports["strict"])
            self.assertEqual(reports["passed"]["summary"]["mouse_unmodified_projected"], 9)
            self.assertEqual(reports["passed"]["criteria"]["minimum_configuration_version"], 22)
            for path, content in inputs.items():
                self.assertEqual(path.read_bytes(), content)
                name = "catalog" if path.stem == "catalog_payload" else path.stem
                self.assertEqual(reports["passed"]["evidence"][name]["sha256"], hashlib.sha256(content).hexdigest())
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(args + ["--unmodified-mouse-policy", "passed", "--out", str(root / "passed.json")])

    def test_cli_rejects_unknown_policy_before_reading_inputs(self):
        args = [part for name in ("source", "catalog", "usage", "counters", "current", "out")
                for part in ("--" + name, "missing.json")]
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
            main(args + ["--unmodified-mouse-policy", "unknown"])
        self.assertEqual(caught.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
