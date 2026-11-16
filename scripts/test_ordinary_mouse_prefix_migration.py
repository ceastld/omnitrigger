"""普通Caps鼠标改编必须显式选择，保留来源与当前配置，只生成审阅计划。"""
import copy
import json
from pathlib import Path
import tempfile
import unittest

from mouse_action_projection import mouse_trigger, ordinary_mouse_keys, validate_ordinary_mouse_configuration
from plan_action_migration import build_plan, main
from test_action_migration import fixture
from test_mouse_action_migration import rule, settings


def source_rule(kind=4, key=20):
    return dict(rule(kind), ControlKey=key, MouseButton=16777216 if kind == 4 else None,
                ActionType=12, Data='action-a', ParamData='')


def data():
    value = fixture()
    value['source']['MouseActions'] = [source_rule(), source_rule(11), source_rule(12), source_rule(key=32)]
    return value


class OrdinaryMousePrefixMigrationTests(unittest.TestCase):
    def test_default_and_empty_choice_keep_the_captured_leader_contract(self):
        value = data()
        before = build_plan(**value, mouse_settings=settings())
        self.assertEqual(build_plan(**value, mouse_settings=settings(), ordinary_mouse_prefixes=[]), before)
        mouse = [r for r in before['bindings'] if r['kind'] == 'mouse']
        self.assertTrue(all(row['trigger']['keyboard']['matching']['kind'] == 'mouse_captured_leader' for row in mouse))
        self.assertTrue(all(row['trigger_dependencies'][0]['kind'] == 'existing_keyboard_leader' for row in mouse))

    def test_caps_x2_uses_default_exact_and_keeps_original_callback_and_pointer_scope(self):
        source = source_rule()
        frozen = copy.deepcopy(source)
        projected = mouse_trigger(source, settings(), priority=21, ordinary_keys=[20])
        trigger = projected['trigger']
        self.assertEqual(trigger['held'], [{'kind': 'key', 'value': 20}])
        self.assertEqual(trigger['activation'], {'kind': 'press', 'value': {'kind': 'x2'}})
        self.assertNotIn('keyboard', trigger)
        self.assertEqual(trigger['pointer_scope'], {'identity': 'shell_context', 'deny': ['blocked', 'lockapp', 'lockapphost']})
        self.assertNotIn('mouse_existing_captured_leader_dependency', projected['trigger_gaps'])
        self.assertEqual(projected['trigger_dependencies'], [{'kind': 'ordinary_current_keyboard_configuration', 'key': 20, 'captured_owner_allowed': False}])
        self.assertFalse(projected['source_requirements']['projection_policy']['source_equivalence'])
        self.assertFalse(projected['source_requirements']['projection_policy']['creates_capture_layer'])
        self.assertTrue(projected['source_requirements']['projection_policy']['requires_owned_prefix'])
        self.assertIn('source_mouse_priority_not_represented_by_ordinary_exact', projected['source_requirements']['semantic_differences'])
        note = projected['source_normalizations'][-1]
        self.assertEqual(note['source_priority'], 21)
        self.assertIs(note['source_priority_preserved'], False)
        self.assertEqual(source, frozen)

    def test_only_selected_caps_changes_space_and_x1_and_modifier_rules_stay_identical(self):
        value = data()
        value['source']['MouseActions'] += [rule(), dict(rule(), ControlKey=17), dict(rule(), ControlKey=None)]
        before = build_plan(**value, mouse_settings=settings())
        after = build_plan(**value, mouse_settings=settings(), ordinary_mouse_prefixes=[20])
        old = [r for r in before['bindings'] if r['kind'] == 'mouse']
        new = [r for r in after['bindings'] if r['kind'] == 'mouse']
        self.assertEqual(new[3:], old[3:])
        self.assertTrue(all('keyboard' not in row['trigger'] for row in new[:3]))
        self.assertEqual([r['trigger'].get('wheel') for r in new[:3]], [None, 'packet', 'packet'])
        self.assertEqual(after['summary']['ordinary_mouse_prefix_adapted'], 3)
        self.assertEqual(after['criteria']['ordinary_mouse_prefixes'], [20])

    def test_plan_preserves_all_source_power_rows_current_bindings_and_action_operations(self):
        value = data()
        value['current']['bindings'] = [{'id': 77, 'held': [{'kind': 'key', 'value': 20}], 'activation': {'kind': 'press', 'value': {'kind': 'key', 'value': 87}}, 'enabled': True}]
        original = copy.deepcopy(value)
        before = build_plan(**value, mouse_settings=settings())
        after = build_plan(**value, mouse_settings=settings(), ordinary_mouse_prefixes=[20])
        self.assertEqual(value, original)
        self.assertEqual([r for r in after['bindings'] if r['kind'] == 'power_key'], [r for r in before['bindings'] if r['kind'] == 'power_key'])
        for a, b in zip(after['bindings'], before['bindings']):
            self.assertEqual(a['source'], b['source'])
            self.assertEqual(a['operation'], b['operation'])
            if a['kind'] == 'mouse':
                self.assertEqual(a['equivalent_configuration_ids'], [])
                self.assertEqual(a['status'], 'needs_trigger_semantics_review')
        self.assertFalse(after['complete'])
        self.assertEqual(after['criteria']['ordinary_mouse_configuration'], 'preserve_current_keyboard_configuration')
        self.assertTrue(after['criteria']['source_power_key_rows_are_review_only'])

    def test_passed_unmodified_choice_combines_without_changing_its_contract(self):
        value = data()
        value['source']['MouseActions'] = [source_rule(), dict(rule(), ControlKey=None)]
        baseline = build_plan(**value, mouse_settings=settings(), unmodified_mouse_policy='passed')
        result = build_plan(**value, mouse_settings=settings(), unmodified_mouse_policy='passed', ordinary_mouse_prefixes=[20])
        before = next(row for row in baseline['bindings'] if row['key'] == 'mouse:global:1')
        after = next(row for row in result['bindings'] if row['key'] == 'mouse:global:1')
        self.assertEqual(after, before)
        self.assertEqual(after['trigger']['keyboard']['matching']['kind'], 'mouse_unmodified')
        self.assertEqual(result['criteria']['minimum_configuration_version'], 22)

    def test_same_key_capture_owner_is_rejected_even_if_disabled(self):
        for policy in [
            {'dedicated_prefix': {'kind': 'key', 'value': 20}},
            {'retained_prefix': {'key': {'kind': 'key', 'value': 20}}},
        ]:
            for enabled in [False, True]:
                value = data()
                value['current']['bindings'] = [{'id': 88, 'keyboard': policy, 'enabled': enabled}]
                with self.assertRaisesRegex(ValueError, '捕获owner'):
                    build_plan(**value, mouse_settings=settings(), ordinary_mouse_prefixes=[20])

    def test_space_capture_is_not_removed_or_implicitly_adapted(self):
        value = data()
        owner = {'id': 10, 'keyboard': {'retained_prefix': {'key': {'kind': 'key', 'value': 32}}}, 'enabled': False}
        value['current']['bindings'] = [owner]
        result = build_plan(**value, mouse_settings=settings(), ordinary_mouse_prefixes=[20])
        space = next(r for r in result['bindings'] if r['key'] == 'mouse:global:3')
        self.assertEqual(space['trigger']['keyboard']['matching']['kind'], 'mouse_captured_leader')
        self.assertEqual(value['current']['bindings'], [owner])
        with self.assertRaises(ValueError):
            build_plan(**value, mouse_settings=settings(), ordinary_mouse_prefixes=[32])

    def test_invalid_keys_and_duplicate_keys_fail_before_projection(self):
        for keys in [20, '20', [True], [5], [16], [17], [18], [91], [92], [160], [165], [255], [-1], [20, 20], [[20]]]:
            with self.assertRaises(ValueError, msg=repr(keys)):
                build_plan(**data(), mouse_settings=settings(), ordinary_mouse_prefixes=keys)
        self.assertEqual(ordinary_mouse_keys([32, 20]), frozenset([20, 32]))

    def test_malformed_current_owner_never_silently_bypasses_the_guard(self):
        for row in [None, {'keyboard': None}, {'keyboard': {'retained_prefix': True}},
                    {'keyboard': {'retained_prefix': {}}},
                    {'keyboard': {'dedicated_prefix': 20}},
                    {'keyboard': {'dedicated_prefix': {'kind': 'key', 'value': [20]}}}]:
            with self.assertRaises(ValueError):
                validate_ordinary_mouse_configuration([20], {'bindings': [row]})

    def test_priority_and_source_flags_remain_strict_under_explicit_adaptation(self):
        for priority in [True, -1, 512, '21']:
            result = mouse_trigger(source_rule(), settings(), priority=priority, ordinary_keys=[20])
            self.assertIsNone(result['trigger'])
            self.assertIn('mouse_rule_order_invalid', result['trigger_gaps'])
        for update in [{'DebounceMs': 50}, {'ActivatePointingWindow': True}, {'TriggerActionWhenMouseUp': True}]:
            result = mouse_trigger(dict(source_rule(), **update), settings(), ordinary_keys=[20])
            self.assertIsNone(result['trigger'])

    def test_whitelist_and_conditions_are_preserved_when_layer_dependency_is_removed(self):
        source = dict(source_rule(11), WhiteList=['reader.exe'], BlackList=['reader.exe'], Location=48)
        before = mouse_trigger(source, settings(), priority=0)
        after = mouse_trigger(source, settings(), priority=0, ordinary_keys=[20])
        self.assertEqual(after['trigger']['pointer_scope'], before['trigger']['pointer_scope'])
        self.assertEqual(after['trigger']['conditions'], before['trigger']['conditions'])
        self.assertEqual(after['trigger']['wheel'], 'packet')

    def test_cli_records_explicit_choice_preserves_inputs_and_never_overwrites_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = []
            for name, content in data().items():
                name = 'catalog' if name == 'catalog_payload' else name
                path = root / (name + '.json')
                path.write_text(json.dumps(content), encoding='utf8')
                args += ['--' + name, str(path)]
            path = root / 'mouse.json'
            path.write_text(json.dumps(settings()), encoding='utf8')
            args += ['--mouse-settings', str(path)]
            inputs = {p.name: p.read_bytes() for p in root.iterdir()}
            main(args + ['--out', str(root / 'default.json')])
            default = json.loads((root / 'default.json').read_bytes())
            self.assertNotIn('ordinary_mouse_prefixes', default['criteria'])
            self.assertEqual(default['bindings'][-1]['trigger']['keyboard']['matching']['kind'], 'mouse_captured_leader')
            output = root / 'ordinary.json'
            main(args + ['--ordinary-mouse-key', '20', '--out', str(output)])
            plan = json.loads(output.read_bytes())
            self.assertEqual(plan['criteria']['ordinary_mouse_prefixes'], [20])
            self.assertFalse(plan['complete'])
            self.assertTrue(plan['evidence']['current']['sha256'])
            for name, raw in inputs.items():
                self.assertEqual((root / name).read_bytes(), raw)
            with self.assertRaises(FileExistsError):
                main(args + ['--ordinary-mouse-key', '20', '--out', str(output)])

    def test_cli_rejects_conflict_without_creating_output_or_touching_inputs(self):
        value = data()
        value['current']['bindings'] = [{'id': 10, 'keyboard': {'dedicated_prefix': {'kind': 'key', 'value': 20}}}]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = []
            for name, content in value.items():
                name = 'catalog' if name == 'catalog_payload' else name
                path = root / (name + '.json')
                path.write_text(json.dumps(content), encoding='utf8')
                args += ['--' + name, str(path)]
            before = {p.name: p.read_bytes() for p in root.iterdir()}
            output = root / 'rejected.json'
            with self.assertRaises(ValueError):
                main(args + ['--ordinary-mouse-key', '20', '--out', str(output)])
            self.assertFalse(output.exists())
            self.assertEqual(before, {p.name: p.read_bytes() for p in root.iterdir()})


if __name__ == '__main__':
    unittest.main()
