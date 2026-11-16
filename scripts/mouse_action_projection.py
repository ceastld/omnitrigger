"""鼠标需求的离线核对；保留身份采样、捕获生命周期和队列差异，不启用绑定。"""
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


def validate_unmodified_mouse_policy(value):
    if not isinstance(value, str) or value not in ("strict", "passed"):
        raise ValueError("无控制键鼠标策略必须为 strict 或 passed")


def ordinary_mouse_keys(values=None):
    """只接受显式选择的非修饰键；默认保留来源捕获层，不能自动推断普通前缀。"""
    if values is None:
        return frozenset()
    if not isinstance(values, (list, tuple, set, frozenset)):
        raise ValueError("普通鼠标前缀改编必须是键码集合")
    modifiers = {16, 17, 18, 91, 92, 160, 161, 162, 163, 164, 165}
    if any(type(key) is not int or not 7 <= key <= 254 or key in modifiers for key in values):
        raise ValueError("普通鼠标前缀只支持7–254范围内的明确非修饰键")
    if len(values) != len(set(values)):
        raise ValueError("普通鼠标前缀键码不能重复")
    return frozenset(values)


def validate_ordinary_mouse_configuration(keys, current):
    """普通改编不得与当前同键专用/保留捕获owner混用；不替用户删除任何绑定。"""
    selected = ordinary_mouse_keys(keys)
    if not selected:
        return selected
    if not isinstance(current, dict) or not isinstance(current.get("bindings"), list):
        raise ValueError("普通鼠标前缀需要明确的当前键盘配置")
    for row in current["bindings"]:
        if not isinstance(row, dict):
            raise ValueError("当前绑定必须为对象")
        policy = row.get("keyboard", {})
        if not isinstance(policy, dict):
            raise ValueError("当前键盘策略必须为对象")
        retained = policy.get("retained_prefix")
        if retained is not None and not isinstance(retained, dict):
            raise ValueError("当前保留捕获策略必须为对象")
        prefixes = [policy.get("dedicated_prefix")]
        if isinstance(retained, dict):
            if retained.get("key") is None:
                raise ValueError("当前保留捕获owner缺少明确键码")
            prefixes.append(retained.get("key"))
        for button in prefixes:
            if button is not None and (not isinstance(button, dict) or button.get("kind") != "key"
                                       or type(button.get("value")) is not int):
                raise ValueError("当前捕获owner必须有明确键码")
        if any(isinstance(button, dict) and button.get("kind") == "key"
               and button.get("value") in selected for button in prefixes):
            raise ValueError("当前同键仍有专用/保留捕获owner，不能投影普通鼠标前缀")
    return selected


def mouse_requirements(rule, *, unmodified_mouse_policy="strict", ordinary_keys=None):
    """已核对的 V1 控制键与区域含义；识别枚举不等于已经实现其完整匹配策略。"""
    validate_unmodified_mouse_policy(unmodified_mouse_policy)
    selected_ordinary = ordinary_mouse_keys(ordinary_keys)
    key, location = rule.get("ControlKey"), rule.get("Location")
    gaps = []
    if "ControlKey" not in rule:
        control = {"kind":"unknown"}
        gaps.append("mouse_control_key_missing")
    elif key is None or type(key) is int and key == 0:
        control = {"kind":"unmodified"}
    elif type(key) is int and key in (5, 6):
        control = {"kind":"mouse_button", "button":"x1" if key == 5 else "x2"}
    elif type(key) is int and key in (1, 2, 4):
        control = {"kind":"mouse_button", "button":{1:"left",2:"right",4:"middle"}[key]}
        gaps.append("mouse_primary_button_capture")
    elif type(key) is int and key in (16, 17, 18):
        control = {"kind":"modifier_family", "family":{16:"shift",17:"control",18:"alt"}[key]}
    elif type(key) is int and key in (160,161,162,163,164,165,91,92):
        family, side = {160:("shift","left"),161:("shift","right"),162:("control","left"),
                        163:("control","right"),164:("alt","left"),165:("alt","right"),
                        91:("windows","left"),92:("windows","right")}[key]
        control = {"kind":"modifier_key", "key":key, "side":side, "family":family}
    elif type(key) is int and 7 <= key <= 254:
        control = {"kind":"captured_keyboard_key", "key":key}
    else:
        control = {"kind":"unknown", "value":key}
        gaps.append("mouse_control_key_semantics")
    edges = {48:"top",192:"bottom",196608:"left",786432:"right"}
    if type(location) is int and location == 0:
        region = {"kind":"unrestricted"}
    elif type(location) is int and location in edges:
        region = {"kind":"screen_edge", "edge":edges[location], "thickness_px":1}
    else:
        region = {"kind":"unsupported", "value":location}
        gaps.append("unsupported_mouse_Location")
    result = {"control":control, "region":region, "unresolved":sorted(set(gaps))}
    if control["kind"] in ("unmodified", "modifier_family", "modifier_key"):
        result["projection_policy"] = {"kind":"mouse_exact", "modifier_delivery":"observed",
                                       "extra_held_inputs":"exclude", "source_equivalence":False}
        result["semantic_differences"] = (["source_checks_captured_keys_not_all_ordinary_keys",
                                            "source_unmodified_does_not_exclude_windows_keys",
                                            "source_virtual_control_state_unavailable"]
                                           if control["kind"] == "unmodified" else
                                           ["source_async_modifier_sampling_unavailable",
                                            "source_async_modifier_path_can_allow_extra_keyboard_keys"])
        result["semantic_differences"].append("source_checks_captured_mouse_buttons_not_all_physical_buttons")
        if control["kind"] == "unmodified" and unmodified_mouse_policy == "passed":
            result["projection_policy"] = {
                "kind":"mouse_unmodified", "modifier_delivery":"exclude_ctrl_shift_alt",
                "extra_held_inputs":"passed_only", "creates_capture":False,
                "minimum_configuration_version":22, "source_equivalence":False}
            result["semantic_differences"] = [
                "source_checks_captured_keys_not_core_ownership",
                "source_checks_mouse_session_captures_not_core_ownership",
                "source_virtual_control_state_unavailable",
                "source_async_modifier_sampling_unavailable",
                "source_v1_binary_parity_unverified",
                "source_builtin_enablement_unverified",
                "source_queue_drop_delay_unverified",
                "source_recovery_equivalence_unverified"]
            if type(rule.get("MouseActionType")) is int and rule["MouseActionType"] in (11, 12):
                result["semantic_differences"].append("source_small_vertical_delta_gate_unverified")
    if control["kind"] == "captured_keyboard_key":
        result["projection_policy"] = {"kind":"mouse_captured_leader", "creates_capture":False,
            "requires_active_leader":control["key"], "source_equivalence":False}
        result["semantic_differences"] = ["source_keyboard_session_contract_review",
            "source_checks_mouse_session_captures_not_all_owned_releases"]
        if key in selected_ordinary:
            result["projection_policy"] = {"kind":"ordinary_exact", "creates_capture_layer":False,
                "requires_owned_prefix":True, "source_equivalence":False}
            result["semantic_differences"] = [
                "source_capture_layer_explicitly_replaced_by_ordinary_exact",
                "ordinary_exact_rejects_extra_held_inputs",
                "passed_or_revoked_prefix_requires_fresh_physical_press",
                "source_mouse_priority_not_represented_by_ordinary_exact"]
    return result


def mouse_rule_priorities(rules):
    """同一全局规则层：较高源 Priority、较具体位置、快照中的稳定顺序。

    未导出 Priority 的用户规则使用声明默认值0；不包含运行时生成的内置规则。
    """
    enabled = [(index, rule) for index, rule in enumerate(rules) if rule.get("IsEnabled")]
    if len(enabled) > 512:
        raise ValueError("鼠标规则顺序超过512条上限")
    locations = {1:1, 2:1, 4:1, 8:1,
                 16:2, 32:2, 64:2, 128:2, 65536:2, 131072:2, 262144:2, 524288:2,
                 48:3, 192:3, 196608:3, 786432:3, 256:4,
                 4096:5, 8192:5, 16384:5, 32768:5,
                 12288:6, 49152:6, 20480:6, 40960:6, 61440:7}
    def key(item):
        index, rule = item
        priority = rule.get("Priority", 0)
        if type(priority) is not int or not -(2**31) <= priority < 2**31:
            raise ValueError("鼠标源 Priority 必须是32位整数")
        location = rule.get("Location")
        order = locations.get(location, 1000) if type(location) is int else 1000
        return -priority, order, index
    return {index: priority for priority, (index, _) in enumerate(sorted(enabled, key=key))}


def mouse_trigger(rule, settings, *, priority=None, unmodified_mouse_policy="strict", ordinary_keys=None):
    requirements = mouse_requirements(rule, unmodified_mouse_policy=unmodified_mouse_policy, ordinary_keys=ordinary_keys)
    def unavailable(gaps):
        return {"trigger":None, "trigger_gaps":sorted(set(gaps)), "source_requirements":requirements}
    if requirements["unresolved"]:
        return unavailable(requirements["unresolved"])
    gaps = []
    if not isinstance(settings, dict) or any(key not in settings for key in ("SpecialExeList", "DisableOnFullscreenApp", "EnableReverseVScroll")):
        return unavailable(["mouse_global_settings_missing"])
    flags = {"IsEnabled":True,"Operation":4,"AdornKey":None,"TriggerDistance":0,"DebounceMs":0,
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
    control_kind = requirements["control"]["kind"]
    keyboard_control = control_kind in ("modifier_key", "modifier_family")
    observed_control = keyboard_control or control_kind == "unmodified"
    captured_leader = control_kind == "captured_keyboard_key"
    ordinary_leader = captured_leader and requirements["projection_policy"]["kind"] == "ordinary_exact"
    control = None if observed_control or captured_leader else requirements["control"]["button"]
    if kind == 4:
        mouse_button = rule.get("MouseButton")
        button = ({1048576:"left",2097152:"right",4194304:"middle",8388608:"x1",16777216:"x2"}.get(mouse_button)
                  if type(mouse_button) is int else None)
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
        return unavailable(gaps)
    scope = {"identity":"shell_context"}
    if allow: scope["allow"]=allow
    if deny: scope["deny"]=deny
    held = ([{"kind":"key", "value":rule["ControlKey"]}] if keyboard_control or captured_leader
            else [] if observed_control else [{"kind":control}])
    trigger = {"held":held,"activation":activation,"pointer_scope":scope}
    if requirements["region"]["kind"] == "screen_edge":
        trigger["conditions"] = [{"kind":"pointer_region", "region":{
            "kind":"screen_boundary", "edge":requirements["region"]["edge"]}}]
    if observed_control or captured_leader:
        if priority is not None and (type(priority) is not int or not 0 <= priority < 512):
            return unavailable(["mouse_rule_order_invalid"])
    if observed_control or (captured_leader and not ordinary_leader):
        matching = {"kind":"mouse_captured_leader" if captured_leader else
                    "mouse_unmodified" if control_kind == "unmodified" and unmodified_mouse_policy == "passed"
                    else "mouse_exact"}
        if priority is not None:
            if type(priority) is not int or not 0 <= priority < 512:
                return unavailable(["mouse_rule_order_invalid"])
            matching["priority"] = priority
            matching["pointer_scope_priority"] = False
        trigger["keyboard"]={"matching":matching}
    if kind in (11,12): trigger["wheel"]="packet"
    reviews = (["mouse_current_keyboard_configuration_required", "mouse_ordinary_prefix_habits_review",
                "mouse_ordinary_fallback_repeat_release_review"] if ordinary_leader else
               ["mouse_existing_captured_leader_dependency", "mouse_owner_lifecycle_review"] if captured_leader else
               ["mouse_modifier_capture_delivery_review", "mouse_modifier_state_sampling", "mouse_other_held_keys_review"]
               if keyboard_control else ["mouse_unmodified_source_ownership_review", "mouse_virtual_control_state_review",
                                         "mouse_other_held_keys_review"] if observed_control
               else ["mouse_prefix_lifecycle_review"])
    if requirements["region"]["kind"] == "screen_edge":
        reviews.append("mouse_region_rule_priority_review" if priority is None else "mouse_builtin_rule_precedence_review")
    result = {"trigger":trigger,"trigger_gaps":reviews + ["mouse_other_held_buttons_review",
        "mouse_queue_drop_delay_review", "pointer_host_identity_mapping", "pointer_context_sampling"],
        "source_requirements":requirements,
        "source_normalizations":[{"kind":"mouse_whitelist_precedence","bypasses_blacklists":bool(allow)}]}
    if ordinary_leader:
        result["trigger_dependencies"] = [{"kind":"ordinary_current_keyboard_configuration",
            "key":rule["ControlKey"], "captured_owner_allowed":False}]
        result["source_normalizations"].append({"kind":"mouse_ordinary_prefix_adaptation",
            "key":rule["ControlKey"], "explicit":True, "creates_capture_layer":False,
            "source_equivalence":False, "source_priority":priority, "source_priority_preserved":False})
    elif captured_leader:
        result["trigger_dependencies"] = [{"kind":"existing_keyboard_leader","key":rule["ControlKey"],
            "creates_capture":False}]
    if control_kind == "unmodified" and unmodified_mouse_policy == "passed":
        result["trigger_gaps"].extend([
            "mouse_async_system_state_review", "mouse_source_v1_binary_parity_review",
            "mouse_builtin_enablement_review", "mouse_recovery_semantics_review",
            "mouse_physical_application_effects_unverified"])
        result["source_normalizations"].append({
            "kind":"mouse_unmodified_passed_policy", "minimum_configuration_version":22,
            "extra_held_inputs":"passed_only", "creates_capture":False,
            "source_equivalence":False})
        if kind in (11, 12):
            result["trigger_gaps"].append("mouse_small_vertical_delta_review")
    return result
