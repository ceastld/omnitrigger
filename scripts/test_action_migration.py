"""迁移筛选必须保留未解析条目和未支持语义；仅使用合成数据。"""

import json
from pathlib import Path
import tempfile
import unittest

from plan_action_migration import build_plan, main, ranked_actions, shortcut_action, same_process_scope, process_scopes_overlap
from action_body_projection import action_body, keyboard_body, subprogram_dependencies


def key_step_record(key=87, params=None, **changes):
    inputs = {"keys": {"VarKey": None, "Value": json.dumps({"CtrlKeys": [162], "Keys": [key]})}}
    inputs.update(params or {})
    body = {"LimitSingleInstance": False, "SummaryExpression": "$$", "SubPrograms": [], "Variables": [],
            "Steps": [{"StepRunnerKey": "sys:keyInput", "InputParams": inputs, "Disabled": False}]}
    body.update(changes)
    text = json.dumps(body)
    return {"Id": "action-a", "ActionType": "XAction", "UseTemplate": False, "Resolution": "local",
            "StoredData": text, "EffectiveData": text}


def fixture():
    rule = {"SecondaryKey": 74, "ActionType": 1, "Data": "2|65"}
    return dict(
        source={"PowerKeys": {"20": {"Key": 20, "IsEnabled": True,
                                      "KeepOriginKeyFunc": False, "KeyActions": [rule]}},
                "InputSettings": {"DefaultModifiedKeyDownDelay": 0},
                "MouseActions": [], "ExeSettings": {}},
        catalog_payload={"payload": {"items": [{"id": "action-a", "title": "示例动作", "actionType": "XAction"}],
                                     "totalCount": 1, "hasMore": False}},
        usage={"Days30": {"IsSuccess": True, "Data": {"action-a": 3}},
               "Days0": {"IsSuccess": True, "Data": {"action-a": 30}}},
        counters=[{"Method": "CountFeature", "Ids": ["feature-a"]}],
        current={"version": 9, "bindings": []})


class MigrationTests(unittest.TestCase):
    def test_subprogram_calls_remain_dependencies_instead_of_guessing_from_action_title(self):
        record = key_step_record()
        body = json.loads(record['EffectiveData'])
        call = {'StepRunnerKey': 'sys:subprogram', 'InputParams': {
            'subProgram': {'VarKey': None, 'Value': '%%synthetic-close'},
            'var:Ctrl_W': {'VarKey': None, 'Value': 'Reader'},
            'var:Alt_F4': {'VarKey': None, 'Value': 'Browser'}}}
        body['Steps'] = [dict(call, Disabled=True), call]
        record.update(Title='关闭', StoredData=json.dumps(body), EffectiveData=json.dumps(body))
        before = json.dumps(record, sort_keys=True)
        result = action_body(record, {'DefaultModifiedKeyDownDelay': 0})
        self.assertIsNone(result['native_action'])
        self.assertEqual(result['status'], 'needs_source_review')
        self.assertIn('source_subprogram_dependency_required', result['action_gaps'])
        self.assertEqual(result['dependency_review'], {
            'scope': 'active_step_tree', 'inventory_complete': True, 'calls': [{
                'path': 'Steps[1]', 'reference': '%%synthetic-close', 'resolution': 'body_not_provided',
                'input_names': ['var:Alt_F4', 'var:Ctrl_W']}]})
        self.assertEqual(json.dumps(record, sort_keys=True), before)
        data = fixture()
        data['body_records'] = [record]
        plan = build_plan(**data)
        self.assertEqual(plan['summary']['selected_action_subprogram_calls'], 1)
        self.assertEqual(plan['summary']['selected_action_available'], 0)
        self.assertEqual(plan['actions'][0]['status'], 'needs_source_review')

    def test_dependency_inventory_keeps_conditional_and_dynamic_calls_without_evaluation(self):
        record = key_step_record()
        def call(reference):
            return {'StepRunnerKey': 'sys:subprogram', 'InputParams': {'subProgram': reference}}
        literal = call({'VarKey': None, 'Value': '%%synthetic-search'})
        variable = call({'VarKey': 'dynamic', 'Value': None})
        expression = call({'VarKey': None, 'Value': '$=lookup()'})
        interpolated = call({'VarKey': None, 'Value': '%%{dynamic}'})
        body = json.loads(record['EffectiveData'])
        body['Steps'] = [{'StepRunnerKey': 'sys:simpleIf', 'IfSteps': [literal],
                          'ElseSteps': [variable, expression, interpolated]},
                         {'Disabled': True, 'IfSteps': [literal]}]
        record.update(StoredData=json.dumps(body), EffectiveData=json.dumps(body))
        review = subprogram_dependencies(record)
        self.assertTrue(review['inventory_complete'])
        self.assertEqual([row['path'] for row in review['calls']], [
            'Steps[0].IfSteps[0]', 'Steps[0].ElseSteps[0]', 'Steps[0].ElseSteps[1]', 'Steps[0].ElseSteps[2]'])
        self.assertEqual([row['resolution'] for row in review['calls']],
                         ['body_not_provided', 'dynamic_reference', 'dynamic_reference', 'dynamic_reference'])
        self.assertEqual([row['reference'] for row in review['calls']], ['%%synthetic-search', None, None, None])

    def test_dependency_inventory_does_not_read_stale_template_data_or_claim_partial_walk_complete(self):
        record = key_step_record()
        for changes in ({'UseTemplate': True}, {'Resolution': 'template_cache_miss'}, {'StoredData': 'old'}):
            self.assertIsNone(subprogram_dependencies(dict(record, **changes)))
        for steps in ([{}] * 257, [{'Disabled': 'false'}], [{'IfSteps': {}}]):
            body = json.loads(record['EffectiveData'])
            body['Steps'] = steps
            record.update(StoredData=json.dumps(body), EffectiveData=json.dumps(body))
            self.assertFalse(subprogram_dependencies(record)['inventory_complete'])
        body = json.loads(key_step_record()['EffectiveData'])
        text = json.dumps(body).replace('"Steps":', '"Steps": [], "Steps":')
        record.update(StoredData=text, EffectiveData=text)
        self.assertFalse(subprogram_dependencies(record)['inventory_complete'])
        data = fixture()
        data['body_records'] = [record]
        self.assertEqual(build_plan(**data)['summary']['selected_action_dependency_inventory_incomplete'], 1)

    def test_literal_key_step_uses_independent_script_and_ignores_only_disabled_steps(self):
        for key in (87, 84):
            record = key_step_record(key)
            body = json.loads(record['EffectiveData'])
            body['Steps'].insert(0, {'StepRunnerKey':'sys:unknown', 'Disabled':True})
            record.update(StoredData=json.dumps(body), EffectiveData=json.dumps(body))
            original = json.dumps(record, sort_keys=True)
            native = action_body(record, {'DefaultModifiedKeyDownDelay':0})['native_action']
            self.assertEqual(native, {'kind':'invoke','package_id':'examples.keyboard-sequence',
                'action_id':'recipe-script','action_version':1,
                'input':{'steps':[{'kind':'sequence','modifiers':[162],'keys':[key]}]}})
            self.assertEqual(json.dumps(record, sort_keys=True), original)
        for hold, setting, expected in [('-1',10,10),('0',10,0),('25',0,25)]:
            record = key_step_record(params={'holdMs':{'VarKey':None,'Value':hold}})
            self.assertEqual(action_body(record, {'DefaultModifiedKeyDownDelay':setting})['native_action']['input']['keyDownDelayMs'],expected)

    def test_key_step_does_not_flatten_dynamic_branch_repeat_or_extra_behavior(self):
        records = [key_step_record(LimitSingleInstance=True), key_step_record(SummaryExpression='$=dynamic'),
            key_step_record(Variables=[{'Type':0,'DefaultValue':'$=dynamic','SaveState':False,'IsInput':False,'IsOutput':False}]),
            key_step_record(Variables=[{'Type':0,'DefaultValue':'','SaveState':True,'IsInput':False,'IsOutput':False}]),
            key_step_record(params={'repeat':{'VarKey':None,'Value':'2'}}),
            key_step_record(params={'holdMs':{'VarKey':None,'Value':'1001'}}),
            key_step_record(params={'keys':{'VarKey':'dynamic','Value':None}})]
        for field,value in [('IfSteps',[{'StepRunnerKey':'sys:unknown'}]), ('DelayMs',10),
                            ('OutputParams',{'result':'saved'}), ('Disabled','false'), ('Unknown',True)]:
            record=key_step_record(); body=json.loads(record['EffectiveData']); body['Steps'][0][field]=value
            record.update(StoredData=json.dumps(body),EffectiveData=json.dumps(body)); records.append(record)
        for keys in ({'CtrlKeys':[17],'Keys':[87]}, {'CtrlKeys':[162,163],'Keys':[87]},
                     {'CtrlKeys':[True],'Keys':[87]}, {'CtrlKeys':[162],'Keys':[87,84]},
                     {'CtrlKeys':[162],'Keys':[None]}, {'CtrlKeys':[162],'Keys':[87],'Other':1}):
            records.append(key_step_record(params={'keys':{'VarKey':None,'Value':json.dumps(keys)}}))
        for record in records:
            self.assertIsNone(action_body(record, {'DefaultModifiedKeyDownDelay':0})['native_action'])
        record=key_step_record()
        for change in ({'UseTemplate':True},{'Resolution':'template_cache_miss'},{'StoredData':'old'}):
            self.assertIsNone(action_body(dict(record,**change), {'DefaultModifiedKeyDownDelay':0})['native_action'])
        self.assertIsNone(action_body(record,{})['native_action'])
        duplicate=record['EffectiveData'].replace('"LimitSingleInstance": false','"LimitSingleInstance": true, "LimitSingleInstance": false')
        self.assertIsNone(action_body(dict(record,StoredData=duplicate,EffectiveData=duplicate),{'DefaultModifiedKeyDownDelay':0})['native_action'])

    def test_selected_statistics_only_actions_get_calls_without_inventing_bindings(self):
        data=fixture(); data['source']['PowerKeys']={}
        result=build_plan(**data,body_records=[key_step_record()])
        row=result['actions'][0]
        self.assertEqual(row['binding_keys'],[])
        self.assertEqual(row['status'],'needs_runtime_verification')
        self.assertEqual(row['implementation']['native_action']['action_id'],'recipe-script')
        self.assertEqual(result['bindings'],[])
        self.assertEqual(result['summary']['selected_action_available'],1)
        self.assertFalse(result['complete'])
        data['usage']['Days30']['Data']['action-a']=0
        deferred=build_plan(**data,body_records=[key_step_record()])['actions'][0]
        self.assertEqual(deferred['status'],'deferred_unused')
        self.assertNotIn('implementation',deferred)

    def test_builtin_brightness_reuses_native_action_and_keeps_source_policy_reviews(self):
        for name, delta in (('monitor_brightness_increase',5),('monitor_brightness_decrease',-5)):
            data = fixture()
            source = data['source']['PowerKeys']['20']['KeyActions'][0]
            source.update(ActionType=11, Data=name)
            result = build_plan(**data)
            row = result['bindings'][0]
            native = row['operation']['native_action']
            self.assertEqual(native, {'kind':'adjust_system','target':'brightness','delta_percent':delta})
            self.assertEqual(row['status'], 'needs_action_semantics_review')
            self.assertEqual(result['summary']['native_action_requires_semantics_review'], 1)
            self.assertEqual(row['operation']['action_gaps'], ['brightness_target_sampling_review','brightness_quantization_review'])
            data['current']['bindings'] = [dict(row['trigger'], id=99, enabled=True, action=native)]
            checked = build_plan(**data)['bindings'][0]
            self.assertEqual(checked['existing_trigger_ids'], [99])
            self.assertEqual(checked['matching_action_configuration_ids'], [99])
            self.assertEqual(checked['equivalent_configuration_ids'], [])
            source['Message'] = '另有通知'
            self.assertIn('notification_side_effect', build_plan(**data)['bindings'][0]['operation']['action_gaps'])
        for value in ('windows_volume_up','windows_volume_down','monitor_brightness_increase ',None,[]):
            data = fixture()
            data['source']['PowerKeys']['20']['KeyActions'][0].update(ActionType=11, Data=value)
            self.assertNotIn('native_action', build_plan(**data)['bindings'][0]['operation'])
        for operation in (2, 4.0, '4', None):
            data = fixture()
            data['source']['PowerKeys']['20']['KeyActions'][0].update(ActionType=11, Data='monitor_brightness_increase', Operation=operation)
            self.assertNotIn('native_action', build_plan(**data)['bindings'][0]['operation'])

    def test_keyboard_body_preserves_automatic_wait_before_explicit_wait(self):
        record = {"ActionType":"SendKeys", "UseTemplate":False, "Resolution":"local",
                  "StoredData":"@LSHIFT+HOME\r\n;50\r\n@+BACK", "EffectiveData":"@LSHIFT+HOME\r\n;50\r\n@+BACK"}
        result = keyboard_body(record, {"DefaultModifiedKeyDownDelay":0})
        self.assertEqual(result["native_action"]["input"], {"steps":[
            {"kind":"sequence","modifiers":[160],"keys":[36]},
            {"kind":"delay","milliseconds":30}, {"kind":"delay","milliseconds":50},
            {"kind":"sequence","modifiers":[],"keys":[8]}]})

    def test_keyboard_body_keeps_shared_modifiers_literal_text_and_nonpositive_waits(self):
        text = "@CONTROL+VK_M,VK_M\n;-3\n%${cliptext}\n!END"
        record = {"ActionType":"SendKeys","UseTemplate":False,"Resolution":"local","StoredData":text,"EffectiveData":text}
        result = keyboard_body(record, {"DefaultModifiedKeyDownDelay":10})
        self.assertEqual(result["native_action"]["input"], {"keyDownDelayMs":10,"steps":[
            {"kind":"sequence","modifiers":[163],"keys":[77,77]},
            {"kind":"delay","milliseconds":30}, {"kind":"delay","milliseconds":0},
            {"kind":"text","text":"${cliptext}"}, {"kind":"delay","milliseconds":30},
            {"kind":"tap","key":35}]})

    def test_keyboard_body_unverified_templates_and_unsupported_values_cannot_become_noops(self):
        base = {"ActionType":"SendKeys","UseTemplate":False,"Resolution":"local","StoredData":"@+HOME","EffectiveData":"@+HOME"}
        for change in ({"UseTemplate":True},{"UseTemplate":None},{"Resolution":"template_cache_miss"},
                       {"EffectiveData":None},{"StoredData":"different"},{"ActionType":"XAction"}):
            self.assertIsNone(keyboard_body(dict(base, **change), {"DefaultModifiedKeyDownDelay":0})["native_action"])
        for text in ("@LSHIFT+HOME+BACK", "@+unknown", "@+1", "!LSHIFT", "@LSHIFT+", ";10001", "\ue000", "\0", "%"+'字'*22000):
            row = dict(base, StoredData=text, EffectiveData=text)
            self.assertIsNone(keyboard_body(row, {"DefaultModifiedKeyDownDelay":0})["native_action"],repr(text[:30]))
        self.assertIsNone(keyboard_body(base,{})["native_action"])

    def test_body_snapshot_requires_unique_identity_and_retains_unresolved_references(self):
        data = fixture()
        data["source"]["PowerKeys"]["20"]["KeyActions"][0].update(ActionType=12, Data="action-a")
        record = {"Id":"action-a","ActionType":"SendKeys","UseTemplate":False,"Resolution":"local","StoredData":"@+HOME","EffectiveData":"@+HOME"}
        result = build_plan(**data, body_records={"Items":[record]})
        self.assertEqual(result["bindings"][0]["operation"]["native_action"]["action_id"], "recipe")
        self.assertFalse(result["complete"])
        self.assertEqual(build_plan(**data,body_records=[])["bindings"][0]["operation"]["action_gaps"],["source_body_not_in_snapshot"])
        for snapshot in ([record,dict(record,Id="ACTION-A")],[dict(record,Id="")],{"Items":None}):
            with self.assertRaises(ValueError):build_plan(**data,body_records=snapshot)

    def test_literal_text_preserves_dollars_unicode_and_source_rule_without_usage_heuristics(self):
        for text in ("\\", "`", "字😀", "${missing}", "$$", " $= ", "\t", "\r"):
            data = fixture()
            rule = data["source"]["PowerKeys"]["20"]["KeyActions"][0]
            rule.update(ActionType=6, Data=text)
            original = dict(rule)
            row = build_plan(**data)["bindings"][0]
            self.assertEqual(rule, original)
            self.assertEqual(row["operation"]["native_action"], {
                "kind": "local", "version": 1,
                "steps": [{"kind": "input_text", "text": text.replace("$", "$$")}]})
            self.assertEqual(row["status"], "needs_runtime_verification")
            self.assertEqual(row["operation"]["action_gaps"], [])

    def test_literal_text_does_not_replace_dynamic_expansion_or_physical_newline_semantics(self):
        for text, gap in [("{cliptext}", "source_text_expansion_required"),
                          ("{selection}", "source_text_expansion_required"),
                          ("{0:yyyy}", "source_text_expansion_required"),
                          ("fIlE:C:/example.txt", "source_text_expansion_required"),
                          ("a\r\nb", "source_text_newline_key_semantics"),
                          ("\n", "source_text_newline_key_semantics"),
                          ("\ue000", "source_text_extended_flag_semantics"),
                          (None, "invalid_literal_text"), ("", "invalid_literal_text"),
                          ("a\0b", "invalid_literal_text"), ("\ud800", "invalid_literal_text"),
                          ("😀" * 32769, "unsupported_literal_text_size"),
                          ("\x01" * 65536, "unsupported_literal_text_size"),
                          ("$" * 32769, "unsupported_literal_text_size")]:
            data = fixture()
            data["source"]["PowerKeys"]["20"]["KeyActions"][0].update(ActionType=6, Data=text)
            row = build_plan(**data)["bindings"][0]
            self.assertIsNone(row["operation"]["native_action"])
            self.assertIn(gap, row["operation"]["action_gaps"])
            self.assertEqual(row["status"], "needs_action_support")
        data = fixture()
        data["source"]["PowerKeys"]["20"]["KeyActions"][0].update(ActionType=6, Data="x", Message="通知")
        self.assertEqual(build_plan(**data)["bindings"][0]["operation"]["action_gaps"], ["notification_side_effect"])

    def test_literal_text_scope_and_existing_configuration_must_also_match(self):
        data = fixture()
        rule = data["source"]["PowerKeys"]["20"]["KeyActions"][0]
        rule.update(ActionType=6, Data="${literal}")
        projected = build_plan(**data)["bindings"][0]
        data["current"]["bindings"] = [dict(id=7, **projected["trigger"], enabled=True,
                                             action=projected["operation"]["native_action"])]
        self.assertEqual(build_plan(**data)["bindings"][0]["equivalent_configuration_ids"], [7])
        data["current"]["bindings"][0]["action"]["steps"][0]["text"] = "${literal}"
        self.assertEqual(build_plan(**data)["bindings"][0]["equivalent_configuration_ids"], [])

    def test_retained_projection_requires_explicit_source_timing_and_keeps_hint_semantics_gaps(self):
        data=fixture();leader=data["source"]["PowerKeys"]["20"];leader["KeepOriginKeyFunc"]=True
        settings=data["source"]["InputSettings"]
        settings.update(PowerKeys_DelayBeforeCtrlKey=50,PowerKeys_ContinuousInputCheckTime=50,
                        PowerKeys_CancelLongPressKeyDelayMs=1500,PowerKeys_HintWindowDelay=20000,PowerKeys_IgnoreUnknownKey=False)
        row=build_plan(**data)["bindings"][0]
        self.assertEqual(row["trigger_gaps"],[])
        self.assertEqual(row["trigger"]["keyboard"]["retained_prefix"],{"key":{"kind":"key","value":20},"quiet_ms":50,"fast_ms":50})
        self.assertEqual(row["trigger"]["keyboard"]["tap_timeout_ms"],1500)
        for key,value in [("PowerKeys_DelayBeforeCtrlKey",True),("PowerKeys_ContinuousInputCheckTime",60001),
                          ("PowerKeys_CancelLongPressKeyDelayMs",0),("PowerKeys_HintWindowDelay",1000),("PowerKeys_IgnoreUnknownKey",True)]:
            previous=settings[key];settings[key]=value
            self.assertIn("legacy_retained_prefix_timing",build_plan(**data)["bindings"][0]["trigger_gaps"])
            settings[key]=previous

    def test_source_input_timing_requires_evidence_and_preserves_global_and_process_delays(self):
        data=fixture()
        row=build_plan(**data)["bindings"][0]
        self.assertEqual(row["operation"]["native_action"]["keys"],[163,65])
        self.assertEqual(row["operation"]["native_action"]["timing"],{"process_overrides":{"wps":10,"et":10,"wpp":10,"wpspdf":10,"mstsc":10}})
        for delay in [10,1000]:
            data["source"]["InputSettings"]["DefaultModifiedKeyDownDelay"]=delay
            self.assertEqual(build_plan(**data)["bindings"][0]["operation"]["native_action"]["timing"],{"key_down_delay_ms":delay})
        for value,gap in [(None,"source_key_down_delay_unknown"),(True,"source_key_down_delay_unknown"),(1001,"unsupported_key_down_delay")]:
            data["source"]["InputSettings"]["DefaultModifiedKeyDownDelay"]=value
            row=build_plan(**data)["bindings"][0]
            self.assertEqual(row["operation"]["status"],"needs_implementation")
            self.assertIn(gap,row["operation"]["action_gaps"])
            self.assertIsNone(row["operation"]["native_action"])

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
            "either_side_modifier_matching"})
        self.assertEqual(result["summary"]["native_action_available"], 1)
        self.assertFalse(result["complete"])

    def test_existing_actions_compare_trigger_action_enabled_and_conditions(self):
        data = fixture()
        existing = {"id": 5, "held": [{"kind": "key", "value": 20}],
                    "activation": {"kind": "press", "value": {"kind": "key", "value": 74}},
                    "keyboard": {"dedicated_prefix": {"kind": "key", "value": 20}, "repeat": True,
                                 "matching": {"kind": "captured", "priority": 0}},
                    "action": {"kind": "send_shortcut", "keys": [163, 65], "timing": {"process_overrides": {
                        "wps":10,"et":10,"wpp":10,"wpspdf":10,"mstsc":10}}}, "enabled": True}
        data["current"]["bindings"] = [existing]
        self.assertEqual(build_plan(**data)["bindings"][0]["equivalent_configuration_ids"], [5])
        for change in ({"enabled": False}, {"conditions": [{"kind": "example"}]},
                       {"action": {"kind": "notification"}}, {"keyboard": {"repeat": False}},
                       {"process_scope": {"deny": ["editor"]}},
                       {"keyboard": {"dedicated_prefix": {"kind": "key", "value": 20}, "repeat": True}}):
            data["current"]["bindings"] = [dict(existing, **change)]
            self.assertEqual(build_plan(**data)["bindings"][0]["equivalent_configuration_ids"], [])
            self.assertEqual(build_plan(**data)["bindings"][0]["existing_trigger_ids"], [5])

    def test_native_layer_projects_shell_context_and_keeps_unknown_timing_gap(self):
        data = fixture()
        leader = data["source"]["PowerKeys"]["20"]
        leader["BlackList"] = "game.exe;preview"
        leader["KeyActions"][0]["BindingProcessName"] = "Code;cursor.exe"
        row = build_plan(**data)["bindings"][0]
        self.assertEqual(row["trigger_gaps"], [])
        self.assertEqual(row["trigger"]["process_scope"], {"identity": "shell_context", "allow": ["Code", "cursor.exe"], "deny": ["game.exe", "preview"]})
        self.assertEqual(row["trigger"]["keyboard"], {"dedicated_prefix": {"kind": "key", "value": 20}, "repeat": True,
                                                    "matching": {"kind": "captured", "priority": 0}})
        leader["KeepOriginKeyFunc"] = True
        leader["BlackList"] = "explorer"
        self.assertEqual(set(build_plan(**data)["bindings"][0]["trigger_gaps"]), {"legacy_retained_prefix_timing"})

    def test_shell_context_scope_identity_prevents_false_existing_equivalence(self):
        executable = {"allow": ["explorer.exe"]}
        explorer = dict(executable, identity="shell_context")
        desktop = {"allow": ["desktop"], "identity": "shell_context"}
        self.assertFalse(same_process_scope(executable, explorer))
        self.assertTrue(same_process_scope({}, {"identity": "shell_context"}))
        self.assertFalse(process_scopes_overlap(explorer, desktop))
        self.assertTrue(process_scopes_overlap(executable, desktop))
        data = fixture()
        leader = data["source"]["PowerKeys"]["20"]
        leader["KeyActions"][0]["BindingProcessName"] = "explorer"
        leader["KeyActions"].append(dict(leader["KeyActions"][0], BindingProcessName="desktop"))
        rows = build_plan(**data)["bindings"]
        self.assertTrue(all(not row["trigger_gaps"] for row in rows))
        self.assertTrue(all(row["trigger"]["process_scope"]["identity"] == "shell_context" for row in rows))

    def test_standalone_restore_and_duplicate_process_entries_are_explicit_projections(self):
        data = fixture()
        leader = data["source"]["PowerKeys"]["20"]
        leader["KeyActions"][0]["BindingProcessName"] = "Code;Code"
        leader["KeyActions"].append({"SecondaryKey": None, "ActionType": 1, "Data": "0|20"})
        result = build_plan(**data)
        self.assertIn("legacy_standalone_timing_unknown", result["bindings"][0]["trigger_gaps"])
        data["source"]["InputSettings"] = {"PowerKeys_HintWindowDelay": 20000}
        rows = build_plan(**data)["bindings"]
        self.assertEqual(rows[0]["trigger_gaps"], [])
        self.assertEqual(rows[0]["trigger"]["process_scope"]["allow"], ["Code"])
        self.assertEqual(rows[0]["source_normalizations"][0]["kind"], "duplicate_process_entry")
        self.assertTrue(rows[0]["trigger"]["keyboard"]["restore_on_tap"])
        self.assertEqual(rows[0]["trigger"]["keyboard"]["tap_timeout_ms"], 20000)
        self.assertEqual(rows[1]["status"], "prefix_policy_projected_unverified")
        self.assertEqual(rows[1]["folded_into_prefix"], 20)
        leader["KeyActions"][1]["BindingProcessName"] = "Wormhole"
        rows = build_plan(**data)["bindings"]
        self.assertEqual(rows[0]["trigger"]["keyboard"]["tap_scope"], {"identity": "shell_context", "allow": ["Wormhole"]})
        self.assertEqual(rows[1]["status"], "prefix_policy_projected_unverified")

    def test_ordered_application_rules_become_disjoint_and_shadowed_rows_are_retained(self):
        data = fixture()
        leader = data["source"]["PowerKeys"]["20"]
        leader["KeyActions"] = [
            {"SecondaryKey": 74, "ActionType": 1, "Data": "2|65"},
            {"SecondaryKey": 74, "BindingProcessName": "Code;editor", "ActionType": 1, "Data": "2|66"},
            {"SecondaryKey": 74, "BindingProcessName": "code;viewer", "ActionType": 1, "Data": "2|67"},
            {"SecondaryKey": 74, "BindingProcessName": "CODE", "ActionType": 1, "Data": "2|68"},
            {"SecondaryKey": 74, "ActionType": 1, "Data": "2|69"},
        ]
        rows = build_plan(**data)["bindings"]
        self.assertEqual(rows[0]["trigger"]["process_scope"]["deny"], ["Code", "editor", "viewer"])
        self.assertEqual(rows[1]["trigger"]["process_scope"]["allow"], ["Code", "editor"])
        self.assertEqual(rows[2]["trigger"]["process_scope"]["allow"], ["viewer"])
        self.assertEqual(rows[3]["status"], "shadowed_source_rule")
        self.assertEqual(rows[4]["status"], "shadowed_source_rule")
        self.assertEqual(rows[3]["source"]["rule"]["BindingProcessName"], "CODE")
        self.assertEqual(len(rows), 5)

    def test_unimplemented_specific_actions_still_exclude_the_global_fallback_range(self):
        data = fixture()
        leader = data["source"]["PowerKeys"]["20"]
        leader["KeyActions"].append({"SecondaryKey": 74, "ActionType": 12, "Data": "action-a", "BindingProcessName": "Code"})
        rows = build_plan(**data)["bindings"]
        self.assertEqual(rows[0]["trigger"]["process_scope"], {"identity": "shell_context", "deny": ["Code"]})
        self.assertEqual(rows[1]["status"], "needs_action_support")

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

    def test_captured_terminal_preemption_is_preserved_as_a_note_instead_of_a_registration_gap(self):
        data = fixture()
        leader = data["source"]["PowerKeys"]["20"]
        leader["KeyActions"] = [
            {"SecondaryKey": 74, "BindingProcessName": "Code", "ActionType": 1, "Data": "2|65"},
            {"SecondaryKey": 75, "AdornKey": 74, "BindingProcessName": "Code", "ActionType": 1, "Data": "2|66"},
        ]
        rows = build_plan(**data)["bindings"]
        self.assertTrue(all(not row["trigger_gaps"] for row in rows))
        self.assertTrue(all(any(note["kind"] == "source_terminal_preempts_capture" for note in row["source_normalizations"]) for row in rows))
        self.assertTrue(all(row["status"] == "needs_runtime_verification" for row in rows))
        self.assertEqual([row["trigger"]["keyboard"]["matching"]["priority"] for row in rows], [1, 0])
        leader["KeyActions"][1]["BindingProcessName"] = "viewer"
        self.assertTrue(all(row["status"] == "needs_runtime_verification" for row in build_plan(**data)["bindings"]))

    def test_terminal_and_adornment_with_distinct_held_contexts_are_supported(self):
        data = fixture()
        leader = data["source"]["PowerKeys"]["20"]
        leader["KeyActions"] = [
            {"SecondaryKey": 69, "AdornKey": 32, "ActionType": 1, "Data": "2|65"},
            {"SecondaryKey": 80, "AdornKey": 69, "ActionType": 1, "Data": "2|66"},
        ]
        rows = build_plan(**data)["bindings"]
        self.assertTrue(all(not row["trigger_gaps"] for row in rows))
        self.assertTrue(all(row["status"] == "needs_runtime_verification" for row in rows))
        self.assertEqual(rows[0]["trigger"]["held"], [
            {"kind": "key", "value": 20}, {"kind": "key", "value": 32}])
        self.assertEqual(rows[1]["trigger"]["held"], [
            {"kind": "key", "value": 20}, {"kind": "key", "value": 69}])

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
        self.assertEqual(shortcut_action("15|65"), {"kind": "send_shortcut", "keys": [163, 165, 161, 91, 65]})
        for invalid in (None, "^a", "2|65|x", "16|65", "2|1", "2|17", "2|162", "２|65"):
            self.assertIsNone(shortcut_action(invalid), invalid)

    def test_cli_never_overwrites_output_or_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = fixture()
            data["source"]["PowerKeys"]["20"]["KeyActions"][0].update(ActionType=12, Data="action-a")
            args = []
            for name, value in data.items():
                name = "catalog" if name == "catalog_payload" else name
                path = root / (name + ".json")
                path.write_text(json.dumps(value), encoding="utf-8")
                args += ["--" + name, str(path)]
            bodies = root / "bodies.json"
            bodies.write_text(json.dumps([{"Id":"action-a","ActionType":"SendKeys","UseTemplate":False,
                                         "Resolution":"local","StoredData":"@+HOME","EffectiveData":"@+HOME"}]),encoding="utf-8")
            args += ["--bodies",str(bodies)]
            before = {path.name: path.read_bytes() for path in root.iterdir()}
            output = root / "plan.json"
            main(args + ["--out", str(output)])
            self.assertIn("sha256", json.loads(output.read_text(encoding="utf-8"))["evidence"]["source"])
            self.assertIn("sha256", json.loads(output.read_text(encoding="utf-8"))["evidence"]["bodies"])
            self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["bindings"][0]["operation"]["native_action"]["action_id"],"recipe")
            with self.assertRaises(FileExistsError):
                main(args + ["--out", str(output)])
            for name, content in before.items():
                self.assertEqual((root / name).read_bytes(), content)


if __name__ == "__main__":
    unittest.main()
