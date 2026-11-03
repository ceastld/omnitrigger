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
