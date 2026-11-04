"""只用合成规则核验侧键投影、源设置来源及未完成的验收边界。"""
import json
import unittest
import tempfile
from pathlib import Path
from mouse_action_projection import mouse_trigger, mouse_requirements, settings_snapshot
from plan_action_migration import build_plan, main, same_keyboard_policy
from test_action_migration import fixture


def settings():
    return {"DisableOnFullscreenApp":False,"EnableReverseVScroll":False,
            "SpecialExeList":[{"ExeName":"blocked.exe"}]}


def rule(kind=11, data="6|9"):
    return {"IsEnabled":True,"ControlKey":5,"MouseActionType":kind,"MouseButton":2097152 if kind==4 else None,
            "Operation":4,"AdornKey":None,"Location":0,"TriggerDistance":0,"DebounceMs":0,
            "TriggerActionWhenMouseUp":False,"DisableInFullScreen":False,"LimitOnPrimaryScreen":False,
            "ActivatePointingWindow":False,"WhiteList":[],"BlackList":[],"ActionType":1,"Data":data}


class MouseMigrationTests(unittest.TestCase):
    def test_edge_requirements_are_known_without_claiming_matching_equivalence(self):
        for location, edge in ((48,'top'),(192,'bottom'),(196608,'left'),(786432,'right')):
            source = dict(rule(), ControlKey=None, Location=location)
            result = mouse_trigger(source, settings())
            self.assertIsNone(result['trigger'])
            self.assertEqual(result['source_requirements']['control'], {'kind':'unmodified'})
            self.assertEqual(result['source_requirements']['region'], {'kind':'screen_edge','edge':edge,'thickness_px':1})
            self.assertNotIn('mouse_unmodified_control_exclusions', result['trigger_gaps'])
            self.assertFalse(result['source_requirements']['projection_policy']['source_equivalence'])
            self.assertIn('mouse_screen_edge_intersection_priority', result['trigger_gaps'])
            self.assertIn('mouse_screen_edge_seam_policy', result['trigger_gaps'])
            self.assertNotIn('mouse_control_key_semantics', result['trigger_gaps'])

    def test_generic_modifiers_and_captured_keys_keep_distinct_requirements(self):
        for key, family in ((16,'shift'),(17,'control'),(18,'alt')):
            result = mouse_trigger(dict(rule(), ControlKey=key), settings())
            self.assertEqual(result['trigger']['held'], [{'kind':'key','value':key}])
            self.assertEqual(result['trigger']['keyboard'], {'matching':{'kind':'mouse_exact'}})
            self.assertEqual(result['source_requirements']['control'], {'kind':'modifier_family','family':family})
            self.assertNotIn('mouse_either_side_modifier_matching', result['trigger_gaps'])
            self.assertIn('mouse_modifier_state_sampling', result['trigger_gaps'])
            self.assertIn('mouse_other_held_keys_review', result['trigger_gaps'])
        for key in (20, 32):
            result = mouse_trigger(dict(rule(), ControlKey=key), settings())
            self.assertIsNone(result['trigger'])
            self.assertEqual(result['source_requirements']['control'], {'kind':'captured_keyboard_key','key':key})
            self.assertIn('mouse_keyboard_capture_ownership', result['trigger_gaps'])
        for value in (True, 164.0, '164', {}, -1):
            result = mouse_trigger(dict(rule(), ControlKey=value), settings())
            self.assertIsNone(result['trigger'])
            self.assertIn('mouse_control_key_semantics', result['trigger_gaps'])
        self.assertIn('mouse_control_key_missing', mouse_requirements({})['unresolved'])

    def test_explicit_left_alt_candidate_keeps_delivery_and_sampling_reviews(self):
        source = dict(rule(4, '3|71'), ControlKey=164, MouseButton=16777216)
        result = mouse_trigger(source, settings())
        self.assertEqual(result['trigger']['held'], [{'kind':'key','value':164}])
        self.assertEqual(result['trigger']['activation'], {'kind':'press','value':{'kind':'x2'}})
        self.assertIn('mouse_modifier_capture_delivery_review', result['trigger_gaps'])
        self.assertIn('mouse_modifier_state_sampling', result['trigger_gaps'])
        self.assertIn('mouse_other_held_keys_review', result['trigger_gaps'])
        data = fixture(); data['source']['MouseActions'] = [source]
        row = build_plan(**data, mouse_settings=settings())['bindings'][-1]
        self.assertEqual(row['status'], 'needs_trigger_semantics_review')
        self.assertEqual(row['operation']['native_action']['keys'], [163,165,71])
        self.assertEqual(row['equivalent_configuration_ids'], [])
        self.assertIsNone(mouse_trigger(dict(source, MouseButton=16777216.0), settings())['trigger'])
        self.assertEqual(mouse_trigger(dict(source, ControlKey=165), settings())['trigger']['held'],
                         [{'kind':'key','value':165}])

    def test_unmodified_projection_excludes_all_held_inputs_and_keeps_source_ownership_gap(self):
        for value in (None, 0):
            result = mouse_trigger(dict(rule(), ControlKey=value), settings())
            self.assertEqual(result['trigger']['held'], [])
            self.assertEqual(result['trigger']['keyboard'], {'matching':{'kind':'mouse_exact'}})
            self.assertIn('mouse_unmodified_source_ownership_review', result['trigger_gaps'])
            self.assertIn('mouse_virtual_control_state_review', result['trigger_gaps'])
            self.assertEqual(result['source_requirements']['projection_policy']['extra_held_inputs'], 'exclude')
            self.assertFalse(result['source_requirements']['projection_policy']['source_equivalence'])

    def test_supported_modifier_projection_never_claims_source_or_existing_binding_equivalence(self):
        data = fixture()
        data['source']['MouseActions'] = [dict(rule(), ControlKey=17)]
        data['current']['bindings'].append({'id':99,'held':[{'kind':'key','value':17}],
            'activation':{'kind':'wheel_up'},'wheel':'packet','enabled':True,
            'action':{'kind':'notification'}})
        result = build_plan(**data, mouse_settings=settings())
        row = [row for row in result['bindings'] if row['kind']=='mouse'][0]
        self.assertIsNotNone(row['trigger'])
        self.assertEqual(row['status'], 'needs_trigger_semantics_review')
        # 旧清单该字段只表示按键形状相同；完整等价仍为空，策略须单独比较。
        self.assertEqual(row['matching_trigger_configuration_ids'], [99])
        self.assertFalse(same_keyboard_policy(data['current']['bindings'][-1].get('keyboard', {}),
                                             row['trigger']['keyboard']))
        self.assertEqual(row['equivalent_configuration_ids'], [])
        self.assertFalse(result['complete'])

    def test_three_actual_shapes_keep_side_order_scope_and_source_timing(self):
        data=fixture()
        data['source']['MouseActions']=[rule(),rule(12,"2|9"),rule(4,"2|87")]
        data['current']['bindings'].append({"id":99,"held":[{"kind":"x1"}],"activation":{"kind":"wheel_up"},
                                            "enabled":True,"action":{"kind":"notification"}})
        result=build_plan(**data,mouse_settings=settings())
        rows=[row for row in result['bindings'] if row['kind']=='mouse']
        self.assertEqual([row['operation']['native_action']['keys'] for row in rows],[[163,161,9],[163,9],[163,87]])
        self.assertEqual(rows[0]['trigger']['wheel'],'packet')
        self.assertNotIn('wheel',rows[2]['trigger'])
        self.assertEqual(rows[2]['trigger']['activation'],{'kind':'press','value':{'kind':'right'}})
        self.assertEqual(rows[0]['trigger']['pointer_scope'],{'identity':'shell_context','deny':['blocked','lockapp','lockapphost']})
        self.assertEqual(rows[0]['matching_trigger_configuration_ids'],[99])
        self.assertTrue(all(row['status']=='needs_trigger_semantics_review' and row['trigger_gaps'] and not row['equivalent_configuration_ids'] for row in rows))
        self.assertEqual(rows[0]['operation']['native_action']['timing']['process_overrides']['wps'],10)
        self.assertFalse(result['complete'])

    def test_whitelist_bypasses_global_paths_and_own_blacklist(self):
        source=rule();source['WhiteList']=['Reader.EXE'];source['BlackList']=['Reader.exe']
        value=settings();value['SpecialExeList']=[{'ExeName':'C:/apps/'}]
        result=mouse_trigger(source,value)
        self.assertEqual(result['trigger']['pointer_scope'],{'identity':'shell_context','allow':['reader']})
        self.assertTrue(result['source_normalizations'][0]['bypasses_blacklists'])

    def test_missing_settings_or_unsupported_flags_never_become_enabled_shortcuts(self):
        self.assertIsNone(mouse_trigger(rule(),None)['trigger'])
        for change in ({'Location':48},{'ActivatePointingWindow':True},
                       {'DebounceMs':50},{'TriggerActionWhenMouseUp':True},{'IsEnabled':False}):
            self.assertIsNone(mouse_trigger(dict(rule(),**change),settings())['trigger'])
        self.assertIsNone(mouse_trigger(rule(),dict(settings(),EnableReverseVScroll=True))['trigger'])
        self.assertIsNone(mouse_trigger(rule(),dict(settings(),DisableOnFullscreenApp=True))['trigger'])

    def test_paths_and_excess_lists_are_preserved_as_gaps_not_dropped(self):
        for special in ([{'ExeName':'C:/apps/'}],[{'ExeName':f'app{i}.exe'} for i in range(65)]):
            result=mouse_trigger(rule(),dict(settings(),SpecialExeList=special))
            self.assertIsNone(result['trigger'])
            self.assertIn('unsupported_mouse_process_list',result['trigger_gaps'])
        source=rule();source['BlackList']=['bad\x85name.exe']
        self.assertIsNone(mouse_trigger(source,settings())['trigger'])

    def test_empty_special_list_does_not_invent_source_lock_exclusions(self):
        result=mouse_trigger(rule(),dict(settings(),SpecialExeList=[]))
        self.assertNotIn('deny',result['trigger']['pointer_scope'])

    def test_settings_envelope_requires_success_and_unique_names(self):
        payload={'Values':[{'Name':key,'Value':value} for key,value in settings().items()]}
        self.assertEqual(settings_snapshot({'success':True,'resultJson':json.dumps(payload)}),settings())
        with self.assertRaises(ValueError):settings_snapshot({'success':False,'resultJson':'{}'})
        payload['Values'].append(payload['Values'][0])
        with self.assertRaises(ValueError):settings_snapshot(payload)

    def test_cli_records_mouse_settings_hash_without_changing_inputs(self):
        data=fixture();data['source']['MouseActions']=[rule()]
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);args=[]
            for name,value in data.items():
                name='catalog' if name=='catalog_payload' else name
                path=root/(name+'.json');path.write_text(json.dumps(value),encoding='utf-8')
                args+=['--'+name,str(path)]
            mouse=root/'mouse.json';mouse.write_text(json.dumps(settings()),encoding='utf-8')
            args+=['--mouse-settings',str(mouse)]
            before={path.name:path.read_bytes() for path in root.iterdir()}
            output=root/'plan.json';main(args+['--out',str(output)])
            result=json.loads(output.read_text(encoding='utf-8'))
            self.assertIn('sha256',result['evidence']['mouse_settings'])
            self.assertEqual(result['bindings'][-1]['trigger']['wheel'],'packet')
            self.assertFalse(result['complete'])
            for name,content in before.items():self.assertEqual((root/name).read_bytes(),content)
            with self.assertRaises(FileExistsError):main(args+['--out',str(output)])


if __name__=='__main__':unittest.main()
