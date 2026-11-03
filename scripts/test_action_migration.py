"""迁移筛选必须保留未解析条目和未支持语义；仅使用合成数据。"""

import json
from pathlib import Path
import tempfile
import unittest

from plan_action_migration import build_plan, main, ranked_actions, shortcut_action


def fixture():
    rule = {"SecondaryKey": 74, "ActionType": 1, "Data": "2|65"}
    return dict(
        source={"PowerKeys": {"20": {"Key": 20, "IsEnabled": True,
                                      "KeepOriginKeyFunc": True, "KeyActions": [rule]}},
                "MouseActions": [], "ExeSettings": {}},
        catalog_payload={"payload": {"items": [{"id": "action-a", "title": "示例动作", "actionType": "XAction"}],
                                     "totalCount": 1, "hasMore": False}},
        usage={"Days30": {"IsSuccess": True, "Data": {"action-a": 3}},
               "Days0": {"IsSuccess": True, "Data": {"action-a": 30}}},
        counters=[{"Method": "CountFeature", "Ids": ["feature-a"]}],
        current={"version": 9, "bindings": []})


class MigrationTests(unittest.TestCase):
    def test_ranking_excludes_features_before_top_and_keeps_missing_actions(self):
        ranked = ranked_actions({"feature-a": 500, "missing": 100, "local": 99},
                                {"local": {"title": "当前动作"}}, {"feature-a"}, 1)
        self.assertEqual(ranked, [{"id": "missing", "count": 100, "title": None, "loaded_here": False}])
        for count in (-1, 0.5, True):
            with self.assertRaises(ValueError):
                ranked_actions({"id": count}, {}, set(), 20)

    def test_disabled_rows_and_leaders_are_not_selected(self):
        data = fixture()
        leader = data["source"]["PowerKeys"]["20"]
        leader["KeyActions"].append({"IsDisabled": True, "ActionType": 12, "Data": "disabled"})
        data["source"]["PowerKeys"]["32"] = dict(leader, IsEnabled=False)
        result = build_plan(**data)
        self.assertEqual(result["summary"]["enabled_bindings"], 1)
        self.assertNotIn("disabled", [row["id"] for row in result["actions"]])

    def test_enabled_direct_rules_have_no_usage_deletion_heuristic(self):
        data = fixture()
        data["usage"]["Days30"]["Data"] = {}
        result = build_plan(**data)
        self.assertEqual(result["bindings"][0]["status"], "needs_runtime_verification")
        self.assertEqual(result["actions"][0]["status"], "deferred_unused")
        rule = data["source"]["PowerKeys"]["20"]["KeyActions"][0]
        rule.update(ActionType=12, Data="action-a")
        result = build_plan(**data)
        self.assertEqual(result["actions"][0]["status"], "needs_source_review")
        self.assertEqual(result["bindings"][0]["status"], "needs_action_support")

    def test_semantics_gaps_prevent_false_migration_claim(self):
        data = fixture()
        leader = data["source"]["PowerKeys"]["20"]
        leader.update(KeepOriginKeyFunc=False, BlackList="sample")
        leader["KeyActions"][0].update(Control=True, BindingProcessName="editor;browser")
        result = build_plan(**data)
        self.assertEqual(set(result["bindings"][0]["trigger_gaps"]), {
            "foreground_process_scope", "leader_process_exclusions",
            "suppress_unused_prefix", "either_side_modifier_matching"})
        self.assertEqual(result["summary"]["native_action_available"], 1)
        self.assertFalse(result["complete"])

    def test_existing_actions_compare_trigger_action_enabled_and_conditions(self):
        data = fixture()
        existing = {"id": 5, "held": [{"kind": "key", "value": 20}],
                    "activation": {"kind": "press", "value": {"kind": "key", "value": 74}},
                    "action": {"kind": "send_shortcut", "keys": [162, 65]}, "enabled": True}
        data["current"]["bindings"] = [existing]
        self.assertEqual(build_plan(**data)["bindings"][0]["equivalent_configuration_ids"], [5])
        for change in ({"enabled": False}, {"conditions": [{"kind": "example"}]},
                       {"action": {"kind": "notification"}}):
            data["current"]["bindings"] = [dict(existing, **change)]
            self.assertEqual(build_plan(**data)["bindings"][0]["equivalent_configuration_ids"], [])
            self.assertEqual(build_plan(**data)["bindings"][0]["existing_trigger_ids"], [5])

    def test_duplicate_mouse_ids_and_per_app_rules_are_preserved(self):
        data = fixture()
        mouse = {"Id": "repeated", "IsEnabled": True, "ActionType": 12, "Data": "action-a"}
        data["source"]["MouseActions"] = [mouse, dict(mouse)]
        data["source"]["ExeSettings"] = {"editor.exe": {"MouseActions": [mouse]}}
        result = build_plan(**data)
        rows = [row for row in result["bindings"] if row["kind"] == "mouse"]
        self.assertEqual(len(rows), 3)
        self.assertEqual(len({row["key"] for row in rows}), 3)
        self.assertEqual(len(next(row for row in result["actions"] if row["id"] == "action-a")["binding_keys"]), 3)

    def test_partial_catalog_or_failed_statistics_fail_without_output(self):
        for patch in ({"hasMore": True}, {"totalCount": 2}):
            data = fixture()
            data["catalog_payload"]["payload"].update(patch)
            with self.assertRaises(ValueError):
                build_plan(**data)
        data = fixture()
        data["usage"]["Days30"]["IsSuccess"] = False
        with self.assertRaises(ValueError):
            build_plan(**data)

    def test_shortcuts_reject_sendkeys_unknown_bits_and_mouse(self):
        self.assertEqual(shortcut_action("15|65"), {"kind": "send_shortcut", "keys": [162, 160, 164, 91, 65]})
        for invalid in (None, "^a", "2|65|x", "16|65", "2|1", "2|17", "2|162", "２|65"):
            self.assertIsNone(shortcut_action(invalid), invalid)

    def test_cli_never_overwrites_output_or_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = fixture()
            args = []
            for name, value in data.items():
                name = "catalog" if name == "catalog_payload" else name
                path = root / (name + ".json")
                path.write_text(json.dumps(value), encoding="utf-8")
                args += ["--" + name, str(path)]
            before = {path.name: path.read_bytes() for path in root.iterdir()}
            output = root / "plan.json"
            main(args + ["--out", str(output)])
            self.assertIn("sha256", json.loads(output.read_text(encoding="utf-8"))["evidence"]["source"])
            with self.assertRaises(FileExistsError):
                main(args + ["--out", str(output)])
            for name, content in before.items():
                self.assertEqual((root / name).read_bytes(), content)


if __name__ == "__main__":
    unittest.main()
