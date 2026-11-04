"""离线投影已解析的键盘动作正文；不加载源程序或执行输入。"""
import json
import re


def _key(name):
    # 明确键码，不通过字符布局猜测组合键。
    aliases = {"BACK": 8, "BACKSPACE": 8, "TAB": 9, "RETURN": 13, "ENTER": 13,
               "ESC": 27, "ESCAPE": 27, "SPACE": 32, "PRIOR": 33, "PGUP": 33,
               "NEXT": 34, "PGDN": 34, "END": 35, "HOME": 36, "LEFT": 37,
               "UP": 38, "RIGHT": 39, "DOWN": 40, "INSERT": 45, "DELETE": 46,
               "CONTROL": 17, "CTRL": 17, "CONTROLKEY": 17, "SHIFT": 16, "SHIFTKEY": 16,
               "ALT": 18, "MENU": 18, "WIN": 91, "LWIN": 91, "RWIN": 92,
               "LSHIFT": 160, "LSHIFTKEY": 160, "RSHIFT": 161, "RSHIFTKEY": 161,
               "LCONTROL": 162, "LCONTROLKEY": 162, "LCTRL": 162,
               "RCONTROL": 163, "RCONTROLKEY": 163, "RCTRL": 163,
               "LMENU": 164, "LALT": 164, "RMENU": 165, "RALT": 165}
    upper = name.upper()
    if upper in aliases:
        return aliases[upper]
    if re.fullmatch(r"F(?:[1-9]|1[0-9]|2[0-4])", upper):
        return 111 + int(upper[1:])
    if re.fullmatch(r"(?:VK_)?[A-Z]", upper):
        return ord(upper[-1])
    if re.fullmatch(r"VK_[0-9]", upper):
        return ord(upper[-1])
    if re.fullmatch(r"[0-9]{1,10}", name):
        return int(name)
    if re.fullmatch(r"#[0-9]{1,10}", name):
        return int(name[1:])
    if re.fullmatch(r"#0[xX][0-9A-Fa-f]{1,8}", name):
        return int(name[3:], 16)
    return None


def _ordinary(key):
    return (key in (8, 9, 12, 13, 19, 20, 27, 93, 144, 145, 226)
            or key is not None and (32 <= key <= 57 or 65 <= key <= 90
                                   or 95 <= key <= 135 or 166 <= key <= 183
                                   or 186 <= key <= 192 or 219 <= key <= 223 or 246 <= key <= 254))


def action_body(record, settings):
    if record.get("ActionType") == "XAction":
        result = key_step_body(record, settings)
        if result["native_action"] is None:
            review = subprogram_dependencies(record)
            if review is not None:
                result["dependency_review"] = review
                if review["calls"]:
                    result["action_gaps"].append("source_subprogram_dependency_required")
        return result
    return keyboard_body(record, settings)


def subprogram_dependencies(record):
    """只列出已核实本机正文中的调用位置；不执行表达式或按名称猜测子程序行为。"""
    if record.get("UseTemplate") is not False or record.get("Resolution") != "local":
        return None
    data = record.get("EffectiveData")
    if not isinstance(data, str) or data != record.get("StoredData"):
        return None

    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("正文含重复字段")
            result[key] = value
        return result

    calls = []
    remaining = 256

    def walk(steps, path, depth):
        nonlocal remaining
        if steps is None:
            return
        if not isinstance(steps, list) or depth > 16:
            raise ValueError("步骤结构或深度需要人工检查")
        for index, step in enumerate(steps):
            remaining -= 1
            if remaining < 0 or not isinstance(step, dict) or type(step.get("Disabled", False)) is not bool:
                raise ValueError("步骤数量或禁用标记需要人工检查")
            if step.get("Disabled", False):
                continue
            location = f"{path}[{index}]"
            if step.get("StepRunnerKey") == "sys:subprogram":
                params = step.get("InputParams")
                if not isinstance(params, dict):
                    raise ValueError("调用参数需要人工检查")
                reference = params.get("subProgram")
                literal = (isinstance(reference, dict) and set(reference) == {"VarKey", "Value"}
                           and reference["VarKey"] is None and isinstance(reference["Value"], str)
                           and bool(reference["Value"])
                           and not any(marker in reference["Value"] for marker in ("$", "{", "}")))
                calls.append({"path": location, "reference": reference["Value"] if literal else None,
                              "resolution": "body_not_provided" if literal else "dynamic_reference",
                              "input_names": sorted(key for key in params if key.startswith("var:"))})
            for branch in ("IfSteps", "ElseSteps"):
                walk(step.get(branch), location + "." + branch, depth + 1)

    try:
        body = json.loads(data, object_pairs_hook=unique_object)
        if not isinstance(body, dict) or not isinstance(body.get("Steps"), list):
            return None
        walk(body["Steps"], "Steps", 0)
        return {"scope": "active_step_tree", "inventory_complete": True, "calls": calls}
    except (ValueError, TypeError, KeyError, RecursionError):
        # 部分清单仍有定位价值，但绝不标成完整或可投影动作。
        return {"scope": "active_step_tree", "inventory_complete": False, "calls": calls}


def key_step_body(record, settings):
    """将单个字面按键步骤交给现有脚本 API；不解释条件、变量或子程序。"""
    def fail(code):
        return {"status": "needs_source_review", "action_gaps": [code], "native_action": None}
    if record.get("UseTemplate") is not False or record.get("Resolution") != "local":
        return fail("source_body_resolution_required")
    data = record.get("EffectiveData")
    if not isinstance(data, str) or data != record.get("StoredData"):
        return fail("source_body_data_unverified")
    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("正文含重复字段")
            result[key] = value
        return result
    try:
        body = json.loads(data, object_pairs_hook=unique_object)
        if (not isinstance(body, dict) or set(body) - {"LimitSingleInstance", "SummaryExpression", "SubPrograms", "Variables", "Steps"}
                or body.get("LimitSingleInstance") is not False or body.get("SubPrograms") != []
                or body.get("SummaryExpression", "") not in ("", "$$")):
            return fail("source_step_action_metadata_review")
        variables = body.get("Variables")
        if not isinstance(variables, list) or any(not isinstance(var, dict)
                or any(var.get(key) is not False for key in ("SaveState", "IsInput", "IsOutput"))
                or var.get("DefaultValue") not in ("", None) or type(var.get("Type")) is not int or var.get("Type") != 0
                or var.get("CustomType") is not None or var.get("TableDef") is not None for var in variables):
            return fail("source_step_variables_review")
        steps = body.get("Steps")
        if not isinstance(steps, list) or not 1 <= len(steps) <= 32:
            return fail("source_body_step_limit")
        if any(not isinstance(step, dict) or type(step.get("Disabled", False)) is not bool for step in steps):
            return fail("source_step_disabled_invalid")
        active = [step for step in steps if not step.get("Disabled", False)]
        if len(active) != 1:
            return fail("source_step_sequence_review")
        step = active[0]
        if (step.get("StepRunnerKey") != "sys:keyInput" or set(step) - {
                "StepRunnerKey", "InputParams", "OutputParams", "IfSteps", "ElseSteps", "Note", "Disabled", "Collapsed", "DelayMs"}
                or step.get("IfSteps") not in (None, []) or step.get("ElseSteps") not in (None, [])
                or step.get("OutputParams", {}) != {} or type(step.get("DelayMs", 0)) is not int or step.get("DelayMs", 0) != 0):
            return fail("source_step_behavior_review")
        params = step["InputParams"]
        if not isinstance(params, dict) or set(params) - {"keys", "repeat", "interval", "holdMs"}:
            return fail("source_step_parameters_review")
        def literal(name, default=None):
            if name not in params:
                return default
            param = params[name]
            if not isinstance(param, dict) or set(param) != {"VarKey", "Value"} or param["VarKey"] is not None:
                raise ValueError("仅支持字面参数")
            return param["Value"]
        def integer(name, default):
            value = literal(name, default)
            if isinstance(value, str) and re.fullmatch(r"[+-]?[0-9]+", value):
                value = int(value)
            if type(value) is not int or not -(2**31) <= value < 2**31:
                raise ValueError("需要整数参数")
            return value
        if integer("repeat", 1) != 1:
            return fail("source_step_repeat_review")
        integer("interval", 1)  # 单次输入不使用重复间隔，但动态参数仍不能被忽略。
        hold = integer("holdMs", -1)
        keys = json.loads(literal("keys"), object_pairs_hook=unique_object)
        if not isinstance(keys, dict) or set(keys) != {"CtrlKeys", "Keys"}:
            return fail("source_step_keys_invalid")
        modifiers, terminals = keys["CtrlKeys"], keys["Keys"]
        if (not isinstance(modifiers, list) or len(modifiers) > 4
                or any(type(key) is not int or key not in (91, 92, 160, 161, 162, 163, 164, 165) for key in modifiers)
                or len(set(modifiers)) != len(modifiers) or not isinstance(terminals, list) or len(terminals) != 1
                or type(terminals[0]) is not int or not _ordinary(terminals[0])):
            return fail("source_step_keys_invalid")
        families = [{91: 3, 92: 3, 160: 0, 161: 0, 162: 1, 163: 1, 164: 2, 165: 2}[key] for key in modifiers]
        if len(set(families)) != len(families):
            return fail("source_step_keys_invalid")
        delay = settings.get("DefaultModifiedKeyDownDelay") if hold < 0 else hold
        if type(delay) is not int or delay > 1000:
            return fail("source_key_down_delay_unknown")
        inputs = {"steps": [{"kind": "sequence", "modifiers": modifiers, "keys": terminals}]}
        if hold >= 0 or delay > 0:
            inputs["keyDownDelayMs"] = max(0, delay)
        return {"status": "native_action_available", "action_gaps": [],
                "native_action": {"kind": "invoke", "package_id": "examples.keyboard-sequence",
                                  "action_id": "recipe-script", "action_version": 1, "input": inputs}}
    except (ValueError, TypeError, KeyError, RecursionError):
        return fail("source_step_literal_data_required")


def keyboard_body(record, settings):
    """只使用明确的本机正文；模板与未知来源不会回退到旧 StoredData。"""
    def fail(code):
        return {"status": "needs_source_review", "action_gaps": [code], "native_action": None}
    if record.get("UseTemplate") is not False or record.get("Resolution") != "local":
        return fail("source_body_resolution_required")
    if record.get("ActionType") != "SendKeys":
        return fail("source_body_type_not_supported")
    data = record.get("EffectiveData")
    if not isinstance(data, str) or data != record.get("StoredData") or not data:
        return fail("source_body_data_unverified")
    lines = [line for line in re.split(r"[\r\n]", data) if line]
    if len(lines) > 16:
        return fail("source_body_step_limit")
    steps = []
    previous_sleep = True
    has_sequence = False
    for line in lines:
        if not previous_sleep:
            steps.append({"kind": "delay", "milliseconds": 30})
        previous_sleep = False
        if line.startswith(";"):
            value = line[1:]
            if not re.fullmatch(r"\s*[+-]?[0-9]+\s*", value):
                return fail("source_body_delay_invalid")
            delay = int(value)
            if not -(2**31) <= delay <= 2**31 - 1:
                return fail("source_body_delay_invalid")
            steps.append({"kind": "delay", "milliseconds": max(0, delay)})
            previous_sleep = True
        elif line.startswith("@"):
            parts = re.split(r"[|+]", line[1:])
            if len(parts) != 2:
                return fail("source_body_key_format")
            modifiers = [_key(name) for name in parts[0].split(",") if name]
            modifiers = [{16: 161, 17: 163, 18: 165, 65536: 161, 131072: 163, 262144: 165}.get(key, key) for key in modifiers]
            keys = [_key(name) for name in parts[1].split(",") if name]
            if (not keys or len(keys) > 16 or len(modifiers) > 4 or len(set(modifiers)) != len(modifiers)
                    or any(key not in (91, 92, 160, 161, 162, 163, 164, 165) for key in modifiers)
                    or any(not _ordinary(key) for key in keys)):
                return fail("source_body_key_not_supported")
            steps.append({"kind": "sequence", "modifiers": modifiers, "keys": keys})
            has_sequence = True
        elif line.startswith("!") and len(line) > 1:
            key = _key(line[1:])
            if not _ordinary(key):
                return fail("source_body_single_key_not_supported")
            steps.append({"kind": "tap", "key": key})
        else:
            text = line[1:] if line.startswith("%") else line
            if ("\0" in text or any(0xD800 <= ord(c) <= 0xDFFF or 0xE000 <= ord(c) <= 0xE0FF for c in text)
                    or len(text.encode("utf-16-le")) // 2 > 65536):
                return fail("source_body_text_not_supported")
            steps.append({"kind": "text", "text": text})
    if not steps or sum(step.get("milliseconds", 0) for step in steps) > 10000:
        return fail("source_body_wait_limit")
    inputs = {"steps": steps}
    if has_sequence:
        delay = settings.get("DefaultModifiedKeyDownDelay")
        if type(delay) is not int or delay > 1000:
            return fail("source_key_down_delay_unknown")
        if any((len(step["keys"]) + 1) * max(0, delay) > 3000 for step in steps if step["kind"] == "sequence"):
            return fail("source_body_key_wait_limit")
        if delay > 0:
            inputs["keyDownDelayMs"] = delay
    if len(json.dumps(inputs, ensure_ascii=False, separators=(",", ":")).encode("utf-8")) > 65536:
        return fail("source_body_input_limit")
    return {"status": "native_action_available", "action_gaps": [],
            "native_action": {"kind": "invoke", "package_id": "examples.keyboard-sequence",
                              "action_id": "recipe", "action_version": 1, "input": inputs}}
