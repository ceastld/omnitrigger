"""只用合成规则核验侧键投影、源设置来源及未完成的验收边界。"""
import json
import unittest
import tempfile
from pathlib import Path
from mouse_action_projection import mouse_trigger, settings_snapshot
from plan_action_migration import build_plan, main
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
        for change in ({'ControlKey':17},{'Location':48},{'ActivatePointingWindow':True},
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
