"""侧键动作的离线投影；保留身份采样、捕获生命周期和队列差异，不启用绑定。"""
import json


def settings_snapshot(payload):
    if isinstance(payload, dict) and "resultJson" in payload:
        if payload.get("success") is not True:
            raise ValueError("鼠标设置快照未成功导出")
        payload = json.loads(payload["resultJson"])
    if isinstance(payload, dict) and "Values" in payload:
        rows = payload["Values"]
        if not isinstance(rows, list) or any(not isinstance(row, dict) or not isinstance(row.get("Name"), str) for row in rows):
            raise ValueError("鼠标设置 Values 必须是带名称的记录数组")
        names = [row["Name"] for row in rows]
        if len(names) != len(set(names)):
            raise ValueError("鼠标设置快照有重复名称")
        payload = {row["Name"]: row.get("Value") for row in rows}
    if payload is not None and not isinstance(payload, dict):
        raise ValueError("鼠标设置快照必须为对象")
    return payload


def names(values):
    if not isinstance(values, list):
        raise ValueError("鼠标进程名单必须为数组")
    result = []
    for value in values:
        if not isinstance(value, str) or not value or value.strip() != value or len(value.encode('utf-8')) > 260 or any(ord(c) < 32 or 127 <= ord(c) <= 159 or c in '/\\:*?;"<>|' for c in value):
            raise ValueError("鼠标进程名单包含无法表达的路径或名称")
        value = value.lower().removesuffix(".exe")
        if not value or value in (".", ".."):
            raise ValueError("鼠标进程名称为空")
        if value not in result:
            result.append(value)
    return result


def mouse_trigger(rule, settings):
    gaps = []
    if rule.get("ControlKey") not in (5, 6):
        return {"trigger": None, "trigger_gaps": ["mouse_control_key_semantics"]}
    if not isinstance(settings, dict) or any(key not in settings for key in ("SpecialExeList", "DisableOnFullscreenApp", "EnableReverseVScroll")):
        return {"trigger": None, "trigger_gaps": ["mouse_global_settings_missing"]}
    flags = {"IsEnabled":True,"Operation":4,"AdornKey":None,"Location":0,"TriggerDistance":0,"DebounceMs":0,
             "TriggerActionWhenMouseUp":False,"DisableInFullScreen":False,
             "LimitOnPrimaryScreen":False,"ActivatePointingWindow":False}
    for key, expected in flags.items():
        if key not in rule or type(rule[key]) is not type(expected) or rule[key] != expected:
            gaps.append("unsupported_mouse_" + key)
    if settings["DisableOnFullscreenApp"] is not False:
        gaps.append("mouse_global_fullscreen_policy")
    kind = rule.get("MouseActionType")
    if type(kind) is not int or kind not in (4, 11, 12):
        gaps.append("mouse_event_type")
    if kind in (11,12) and settings["EnableReverseVScroll"] is not False:
        gaps.append("mouse_reverse_scroll_policy")
    control = "x1" if rule["ControlKey"] == 5 else "x2"
    if kind == 4:
        button = {1048576:"left",2097152:"right",4194304:"middle",8388608:"x1",16777216:"x2"}.get(rule.get("MouseButton"))
        if button is None or button == control:
            gaps.append("mouse_terminal_button")
        activation = {"kind":"press", "value":{"kind":button}}
    else:
        activation = {"kind":"wheel_up" if kind == 11 else "wheel_down"}
    try:
        allow = names(rule.get("WhiteList"))
        special = settings["SpecialExeList"]
        if not isinstance(special, list) or any(not isinstance(row,dict) for row in special):
            raise ValueError("全局排除名单必须为记录数组")
        deny = [] if allow else names([row["ExeName"] for row in special if row.get("ExeName")]
                                      + (["LockApp.exe","LockAppHost.exe"] if special else [])
                                      + (rule.get("BlackList") if isinstance(rule.get("BlackList"),list) else [None]))
        if len(allow)+len(deny)>64:
            raise ValueError("鼠标进程名单超过上限")
    except ValueError:
        gaps.append("unsupported_mouse_process_list")
        allow, deny = [], []
    if gaps:
        return {"trigger":None,"trigger_gaps":sorted(set(gaps))}
    scope = {"identity":"shell_context"}
    if allow: scope["allow"]=allow
    if deny: scope["deny"]=deny
    trigger = {"held":[{"kind":control}],"activation":activation,"pointer_scope":scope}
    if kind in (11,12): trigger["wheel"]="packet"
    return {"trigger":trigger,"trigger_gaps":["mouse_prefix_lifecycle_review", "mouse_other_held_buttons_review",
        "mouse_queue_drop_delay_review", "pointer_host_identity_mapping", "pointer_context_sampling"],
        "source_normalizations":[{"kind":"mouse_whitelist_precedence","bypasses_blacklists":bool(allow)}]}
