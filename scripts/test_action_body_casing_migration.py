"""两种原始 DTO 拼写必须保持同一含义；规范化编辑格式不能误判成空动作。"""
import copy
import json
import unittest

from action_body_projection import action_body, subprogram_dependencies


def record(body):
    data = json.dumps(body)
    return {"ActionType": "XAction", "UseTemplate": False, "Resolution": "local",
            "StoredData": data, "EffectiveData": data}


def body():
    return {"LimitSingleInstance": False, "SummaryExpression": "$$", "SubPrograms": [],
            "Variables": [{"Key": "保留变量名", "SaveState": False, "IsInput": False, "IsOutput": False,
                           "DefaultValue": "", "Type": 0, "CustomType": None, "TableDef": None}],
            "Steps": [{"StepRunnerKey": "sys:keyInput", "Disabled": False,
                       "InputParams": {"keys": {"VarKey": None, "Value": json.dumps({"CtrlKeys": [162], "Keys": [87]})}}}]}


def camel(value):
    if isinstance(value, dict):
        return {key[:1].lower() + key[1:]: camel(item) for key, item in value.items()}
    if isinstance(value, list):
        return [camel(item) for item in value]
    return value


class CasingTests(unittest.TestCase):
    def test_keyboard_dto_casing_preserves_call_and_literal_key_order(self):
        pascal = body()
        lower = camel(pascal)
        lower['steps'][0]['inputParams']['keys']['value'] = json.dumps({'ctrlKeys': [162], 'keys': [87]})
        expected = action_body(record(pascal), {'DefaultModifiedKeyDownDelay': 0})
        self.assertEqual(expected['status'], 'native_action_available')
        self.assertEqual(action_body(record(lower), {'DefaultModifiedKeyDownDelay': 0}), expected)

    def test_dependency_paths_preserve_actual_field_spelling_and_variable_names(self):
        source = {'steps': [{'stepRunnerKey': 'sys:simpleIf', 'ifSteps': [
            {'stepRunnerKey': 'sys:subprogram', 'inputParams': {
                'subProgram': {'varKey': None, 'value': '%%literal'}, 'var:KeepCASE': {'value': 'x'}}}],
            'elseSteps': [{'disabled': True, 'ifSteps': 'disabled child is not inspected'}]}]}
        result = subprogram_dependencies(record(source))
        self.assertTrue(result['inventory_complete'])
        self.assertEqual(result['calls'], [{'path': 'steps[0].ifSteps[0]', 'reference': '%%literal',
            'resolution': 'body_not_provided', 'input_names': ['var:KeepCASE']}])

    def test_duplicate_semantic_aliases_never_choose_one_interpretation(self):
        fixtures = []
        source = body(); source['steps'] = copy.deepcopy(source['Steps']); fixtures.append(source)
        source = body(); source['Steps'][0]['stepRunnerKey'] = 'sys:subprogram'; fixtures.append(source)
        source = body(); source['Steps'][0]['InputParams']['keys']['value'] = '{}'; fixtures.append(source)
        source = body(); source['Variables'][0]['saveState'] = True; fixtures.append(source)
        source = body(); source['Steps'][0]['InputParams']['keys']['Value'] = '{"CtrlKeys":[162],"Keys":[87],"keys":[84]}'; fixtures.append(source)
        for source in fixtures:
            with self.subTest(source=source):
                self.assertIsNone(action_body(record(source), {'DefaultModifiedKeyDownDelay': 0})['native_action'])

    def test_normalized_authoring_steps_are_not_a_complete_empty_raw_inventory(self):
        source = {'steps': [{'key': 'sys:subprogram', 'inputs': {'subProgram': '%%literal'}}]}
        self.assertIsNone(action_body(record(source), {})['native_action'])
        self.assertFalse(subprogram_dependencies(record(source))['inventory_complete'])

    def test_missing_or_invalid_runner_keeps_partial_inventory_incomplete(self):
        for malformed in [{}, {'stepRunnerKey': None}, {'stepRunnerKey': 1}]:
            result = subprogram_dependencies(record({'steps': [
                {'stepRunnerKey': 'sys:subprogram', 'inputParams': {'subProgram': {'varKey': None, 'value': 'literal'}}}, malformed]}))
            self.assertFalse(result['inventory_complete'])
            self.assertEqual(len(result['calls']), 1)

    def test_saved_variables_and_unknown_behavior_still_require_review(self):
        for field, value in [('saveState', True), ('isInput', True), ('type', 1)]:
            source = camel(body()); source['variables'][0][field] = value
            self.assertIsNone(action_body(record(source), {'DefaultModifiedKeyDownDelay': 0})['native_action'])
        source = camel(body()); source['steps'][0]['unexpectedBehavior'] = True
        self.assertIsNone(action_body(record(source), {'DefaultModifiedKeyDownDelay': 0})['native_action'])


if __name__ == '__main__':
    unittest.main()
