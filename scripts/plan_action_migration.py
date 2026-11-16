"""只读评估离线动作快照；不连接源程序、不安装触发器、不运行动作。"""

import argparse
import collections
import hashlib
import json
from pathlib import Path
from action_body_projection import action_body
from mouse_action_projection import (mouse_trigger, mouse_rule_priorities, settings_snapshot,
                                     validate_unmodified_mouse_policy, validate_ordinary_mouse_configuration)


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def normalized_id(value):
    return str(value or "").strip().lower()


def ranked_actions(counts, catalog, excluded, limit):
    """先排除已确认的功能计数；未解析 ID 仍占排名，避免偷换成已安装 Top N。"""
    rows = []
    for key, count in counts.items():
        identity = normalized_id(key)
        if type(count) is not int or count < 0:
            raise ValueError("使用次数必须为非负整数")
        if identity in excluded or count == 0:
            continue
        action = catalog.get(identity)
        rows.append({"id": identity, "count": count,
                     "title": action.get("title") if action else None,
                     "loaded_here": action is not None})
    return sorted(rows, key=lambda row: (-row["count"], row["id"]))[:limit]


def shortcut_action(data):
    """V1 modifier|VK；按源顺序显式映射泛型修饰键，不解析 SendKeys 文本。"""
    if not isinstance(data, str):
        return None
    parts = data.split("|")
    if len(parts) != 2 or not all(part.isascii() and part.isdecimal() for part in parts):
        return None
    modifiers, terminal = map(int, parts)
    supported = (terminal in (8, 9, 12, 13, 19, 20, 27, 93, 144, 145, 226)
                 or 32 <= terminal <= 57 or 65 <= terminal <= 90
                 or 95 <= terminal <= 135 or 166 <= terminal <= 183
                 or 186 <= terminal <= 192 or 219 <= terminal <= 223
                 or 246 <= terminal <= 254)
    if modifiers & ~15 or not supported:
        return None
    keys = [key for bit, key in ((2, 163), (1, 165), (4, 161), (8, 91))
            if modifiers & bit]
    return {"kind": "send_shortcut", "keys": keys + [terminal]}


def operation(row, settings=None, body_records=None):
    if (type(row.get("ActionType")) is int and row["ActionType"] == 11
            and type(row.get("Operation", 4)) is int and row.get("Operation", 4) == 4):
        brightness = {"monitor_brightness_increase":5, "monitor_brightness_decrease":-5}
        if isinstance(row.get("Data"), str) and row["Data"] in brightness:
            gaps = ["brightness_target_sampling_review", "brightness_quantization_review"]
            if row.get("Message"):
                gaps.append("notification_side_effect")
            return {"status":"native_action_available", "source_type":11,
                    "native_action":{"kind":"adjust_system", "target":"brightness",
                                     "delta_percent":brightness[row["Data"]]}, "action_gaps":gaps}
    if row.get("ActionType") == 12 and body_records is not None:
        identity = normalized_id(row.get("Data"))
        record = body_records.get(identity)
        result = (action_body(record, settings or {}) if record is not None else
                  {"status":"needs_source_review", "native_action":None, "action_gaps":["source_body_not_in_snapshot"]})
        if row.get("Message"):
            result["action_gaps"].append("notification_side_effect")
            result["status"] = "needs_implementation"
        result.update(source_type=12, action_ref=row.get("Data"))
        if record is not None:
            result["body_source"] = {"action_type":record.get("ActionType"), "resolution":record.get("Resolution"),
                                     "uses_template":record.get("UseTemplate"),
                                     "record_sha256":hashlib.sha256(json.dumps(record, sort_keys=True, separators=(",", ":")).encode()).hexdigest()}
        return result
    if row.get("ActionType") == 6:
        text = row.get("Data")
        gaps = ["notification_side_effect"] if row.get("Message") else []
        native = None
        if not isinstance(text, str) or not text or "\0" in text or any(0xD800 <= ord(c) <= 0xDFFF for c in text):
            gaps.append("invalid_literal_text")
        elif any(marker in text for marker in ("{cliptext}", "{selection}", "{0")) or text.upper().startswith("FILE:"):
            gaps.append("source_text_expansion_required")
        elif "\n" in text:
            # 源 LF/CRLF 使用物理 Return；现有文本 API 使用 Unicode CR，不能声称输入契约相同。
            gaps.append("source_text_newline_key_semantics")
        elif any(0xE000 <= ord(c) <= 0xE0FF for c in text):
            gaps.append("source_text_extended_flag_semantics")
        else:
            escaped = text.replace("$", "$$")
            if len(escaped.encode("utf-16-le")) // 2 > 65536:
                gaps.append("unsupported_literal_text_size")
            else:
                native = {"kind": "local", "version": 1,
                          "steps": [{"kind": "input_text", "text": escaped}]}
                payload = {key: value for key, value in native.items() if key != "kind"}
                if len(json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")) > 256 * 1024:
                    gaps.append("unsupported_literal_text_size")
                    native = None
        return {"status": "native_action_available" if native and not gaps else "needs_implementation",
                "native_action": native, "source_type": 6, "action_gaps": gaps}
    if row.get("ActionType") == 1:
        native = shortcut_action(row.get("Data"))
        gaps = ["notification_side_effect"] if row.get("Message") else []
        delay = (settings or {}).get("DefaultModifiedKeyDownDelay")
        if type(delay) is not int:
            gaps.append("source_key_down_delay_unknown")
            native = None
        elif delay > 1000:
            gaps.append("unsupported_key_down_delay")
            native = None
        elif native:
            native["timing"] = ({"key_down_delay_ms": delay} if delay > 0 else
                                {"process_overrides": {name: 10 for name in ("wps", "et", "wpp", "wpspdf", "mstsc")}})
        return {"status": "native_action_available" if native and not gaps else "needs_implementation",
                "native_action": native, "source_type": 1, "action_gaps": gaps}
    return {"status": "needs_source_review" if row.get("ActionType") == 12
            else "needs_implementation", "source_type": row.get("ActionType"),
            "action_ref": row.get("Data") if row.get("ActionType") == 12 else None}


def button(code):
    if type(code) is not int or not 1 <= code <= 254:
        return None
    mouse = {1: "left", 2: "right", 4: "middle", 5: "x1", 6: "x2"}
    return {"kind": mouse[code]} if code in mouse else {"kind": "key", "value": code}


def normalized_process_name(name):
    return name.lower().removesuffix(".exe")


def process_scope(leader, rule):
    result = {}
    gaps = []
    notes = []
    for name, value in (("allow", rule.get("BindingProcessName")), ("deny", leader.get("BlackList"))):
        if not value:
            continue
        if not isinstance(value, str):
            raise ValueError("进程名单必须为字符串")
        names = []
        for item in (item.strip() for item in value.split(";")):
            if not item:
                continue
            if item in names:
                notes.append({"kind": "duplicate_process_entry", "list": name, "name": item})
            else:
                names.append(item)
        result[name] = names
        canonical = [normalized_process_name(item) for item in names]
        if (any(not item or len(item.encode("utf-8")) > 260
                or any(c in '/\\:*?;\"<>|,；，' or ord(c) < 32 or 127 <= ord(c) <= 159 for c in item) for item in names)
                or any(not item or item in (".", "..") for item in canonical)
                or len(set(canonical)) != len(canonical)):
            gaps.append("unsupported_process_list")
    if sum(len(names) for names in result.values()) > 64:
        gaps.append("unsupported_process_list")
    allowed = {normalized_process_name(item) for item in result.get("allow", [])}
    denied = {normalized_process_name(item) for item in result.get("deny", [])}
    if allowed and allowed <= denied:
        gaps.append("empty_process_scope")
    if result:
        result["identity"] = "shell_context"
    return result, gaps, notes


def same_process_scope(left, right):
    def signature(value):
        lists = tuple(tuple(sorted(normalized_process_name(item) for item in value.get(key, [])))
                      for key in ("allow", "deny"))
        return (value.get("identity", "executable") if any(lists) else "global", lists)
    return signature(left) == signature(right)


def same_keyboard_policy(left, right):
    return (left.get("dedicated_prefix") == right.get("dedicated_prefix")
            and left.get("retained_prefix") == right.get("retained_prefix")
            and left.get("repeat", False) == right.get("repeat", False)
            and left.get("matching", {"kind": "exact"}) == right.get("matching", {"kind": "exact"})
            and left.get("restore_on_tap", False) == right.get("restore_on_tap", False)
            and left.get("tap_timeout_ms") == right.get("tap_timeout_ms")
            and same_process_scope(left.get("tap_scope", {}), right.get("tap_scope", {})))


def leader_tap_policy(leader, settings):
    rows = [(index, row) for index, row in enumerate(leader.get("KeyActions") or [])
            if not row.get("IsDisabled") and row.get("SecondaryKey") is None]
    if leader.get("KeepOriginKeyFunc") is not False or len(rows) != 1:
        return {}, [], None
    index, row = rows[0]
    if (row.get("ActionType") != 1 or shortcut_action(row.get("Data")) != {"kind": "send_shortcut", "keys": [leader.get("Key")]}
            or any(row.get(key) for key in ("AdornKey", "Shift", "Control", "Alt", "Message", "LongPressActionType"))):
        return {}, [], None
    policy = {"restore_on_tap": True}
    tap_scope, gaps, _ = process_scope({}, row)
    if tap_scope:
        policy["tap_scope"] = tap_scope
    timeout = settings.get("PowerKeys_HintWindowDelay")
    if type(timeout) is not int or timeout > 60_000:
        return policy, gaps + ["legacy_standalone_timing_unknown"], index
    if timeout > 0:
        policy["tap_timeout_ms"] = timeout
    return policy, gaps, index


def power_trigger(leader, rule, tap_policy=None, tap_gaps=(), priority=0, settings=None):
    scope, gaps, notes = process_scope(leader, rule)
    gaps.extend(tap_gaps)
    dedicated = leader.get("KeepOriginKeyFunc") is False
    settings = settings or {}
    retained = None
    if not dedicated:
        quiet = settings.get("PowerKeys_DelayBeforeCtrlKey")
        fast = settings.get("PowerKeys_ContinuousInputCheckTime")
        cancel = settings.get("PowerKeys_CancelLongPressKeyDelayMs")
        hint = settings.get("PowerKeys_HintWindowDelay")
        if (leader.get("KeepOriginKeyFunc") is True
                and all(type(value) is int and 0 <= value <= 60_000 for value in (quiet, fast))
                and type(cancel) is int and 0 < cancel <= 60_000
                and type(hint) is int and (hint <= 0 or hint > cancel)
                and settings.get("PowerKeys_IgnoreUnknownKey") is False):
            retained = {"key": button(leader.get("Key")), "quiet_ms": quiet, "fast_ms": fast}
            capture_scope, capture_gaps, _ = process_scope(leader, {})
            gaps.extend(capture_gaps)
            if capture_scope:
                retained["capture_scope"] = capture_scope
            if leader.get("Key") in (160, 161, 162, 163):
                retained["block_with_primary_mouse"] = True
        else:
            gaps.append("legacy_retained_prefix_timing")
    if any(rule.get(key) for key in ("Shift", "Control", "Alt")):
        gaps.append("either_side_modifier_matching")
    if rule.get("LongPressActionType", 0) != 0:
        gaps.append("short_and_long_press_dispatch")
    keys = [leader.get("Key")]
    if rule.get("AdornKey"):
        keys.append(rule["AdornKey"])
    if any(code in (16, 17, 18) for code in keys + [rule.get("SecondaryKey")]):
        gaps.append("either_side_modifier_matching")
    held = [button(code) for code in keys]
    terminal = button(rule.get("SecondaryKey"))
    if any(key is None for key in held) or terminal is None:
        gaps.append("unsupported_trigger_key")
        return None, sorted(set(gaps)), notes
    if "either_side_modifier_matching" in gaps:
        return None, sorted(set(gaps)), notes
    keyboard = {"repeat": terminal["kind"] == "key"}
    if retained is not None:
        if retained["key"] and retained["key"]["kind"] == "key" and terminal["kind"] == "key" and priority < 512:
            keyboard.update(retained_prefix=retained, matching={"kind":"captured", "priority":priority},
                            restore_on_tap=True, tap_timeout_ms=cancel)
        else:
            gaps.append("unsupported_retained_prefix")
    if dedicated:
        prefix = button(leader.get("Key"))
        if prefix["kind"] == "key":
            keyboard["dedicated_prefix"] = prefix
            if terminal["kind"] == "key" and priority < 512:
                keyboard["matching"] = {"kind": "captured", "priority": priority}
            else:
                gaps.append("unsupported_captured_matching")
        else:
            gaps.append("dedicated_mouse_prefix")
        keyboard.update(tap_policy or {})
    trigger = {"held": held, "activation": {"kind": "press", "value": terminal}, "keyboard": keyboard}
    if scope:
        trigger["process_scope"] = scope
    return trigger, sorted(set(gaps)), notes


def same_trigger(left, right):
    sort_key = lambda item: json.dumps(item, sort_keys=True)
    return (sorted(left.get("held", []), key=sort_key) == sorted(right.get("held", []), key=sort_key)
            and left.get("activation") == right.get("activation"))


def process_scopes_overlap(left, right):
    def matches(scope, process, role):
        allow = {normalized_process_name(name) for name in scope.get("allow", [])}
        deny = {normalized_process_name(name) for name in scope.get("deny", [])}
        logical = role if scope.get("identity") == "shell_context" and role != "application" else process
        return (not allow or logical in allow) and logical not in deny
    names = {normalized_process_name(name) for scope in (left, right)
             for key in ("allow", "deny") for name in scope.get(key, [])}
    names.update(("desktop", "taskbar", "explorer"))
    outside = "__other_process__"
    while outside in names:
        outside += "_"
    names.add(outside)
    return any(matches(left, name, role) and matches(right, name, role)
               for name in names for role in ("application", "desktop", "taskbar"))


def project_source_precedence(bindings, current):
    """源按列表先后选应用规则；投影成互斥范围，保留被遮蔽的原规则。"""
    groups = collections.defaultdict(list)
    for row in bindings:
        if row["kind"] != "power_key" or row["source"]["rule"].get("SecondaryKey") is None:
            continue
        rule = row["source"]["rule"]
        signature = (row["source"]["leader"]["Key"], rule.get("AdornKey"), rule["SecondaryKey"],
                     rule.get("Shift", False), rule.get("Control", False), rule.get("Alt", False))
        groups[signature].append(row)
    for rows in groups.values():
        if any(row["trigger"] is None or set(row["trigger_gaps"]) & {
                "unsupported_process_list", "empty_process_scope"} for row in rows):
            if len(rows) > 1:
                for row in rows:
                    row["trigger_gaps"] = sorted(set(row["trigger_gaps"] + ["source_process_precedence_review"]))
            continue
        specific = {}
        for row in rows:
            for name in row["trigger"].get("process_scope", {}).get("allow", []):
                specific.setdefault(normalized_process_name(name), name)
        seen = {}
        default_owner = None
        for row in rows:
            scope = row["trigger"].setdefault("process_scope", {})
            allowed = scope.get("allow", [])
            if allowed:
                excluded = {normalized_process_name(name): seen[normalized_process_name(name)]
                            for name in allowed if normalized_process_name(name) in seen}
                effective = [name for name in allowed if normalized_process_name(name) not in seen]
                for name in allowed:
                    seen.setdefault(normalized_process_name(name), row["key"])
                if excluded:
                    row["source_normalizations"].append({"kind": "ordered_process_scope", "excluded": excluded})
                if not effective:
                    row["trigger"] = None
                    row["status"] = "shadowed_source_rule"
                    continue
                scope["allow"] = effective
            elif default_owner is not None:
                row["source_normalizations"].append({"kind": "shadowed_default_rule", "owner": default_owner})
                row["trigger"] = None
                row["status"] = "shadowed_source_rule"
                continue
            else:
                default_owner = row["key"]
                denied = scope.get("deny", [])
                existing = {normalized_process_name(name) for name in denied}
                added = [name for canonical, name in specific.items() if canonical not in existing]
                if added:
                    scope["deny"] = denied + added
                    scope["identity"] = "shell_context"
                    row["source_normalizations"].append({"kind": "default_excludes_specific_rules", "names": added})
            if sum(len(scope.get(key, [])) for key in ("allow", "deny")) > 64:
                row["trigger_gaps"] = sorted(set(row["trigger_gaps"] + ["unsupported_process_list"]))
    candidates = [row for row in bindings if row["kind"] == "power_key" and row.get("trigger")]
    for terminal in candidates:
        activation = terminal["trigger"]["activation"]
        if activation["kind"] != "press":
            continue
        for prefix in candidates:
            if (activation["value"] not in prefix["trigger"]["held"]
                    or not all(held in prefix["trigger"]["held"] for held in terminal["trigger"]["held"])
                    or not process_scopes_overlap(terminal["trigger"].get("process_scope", {}),
                                                  prefix["trigger"].get("process_scope", {}))):
                continue
            if (terminal["trigger"]["keyboard"].get("matching", {}).get("kind") == "captured"
                    and prefix["trigger"]["keyboard"].get("matching", {}).get("kind") == "captured"
                    and terminal["trigger"]["keyboard"].get("dedicated_prefix") == prefix["trigger"]["keyboard"].get("dedicated_prefix")):
                for row, other in ((terminal, prefix), (prefix, terminal)):
                    row["source_normalizations"].append({"kind": "source_terminal_preempts_capture", "other": other["key"]})
                continue
            for row, other in ((terminal, prefix), (prefix, terminal)):
                row["trigger_gaps"] = sorted(set(row["trigger_gaps"] + ["terminal_prefix_overlap"]))
                row["source_normalizations"].append({"kind": "terminal_prefix_review", "other": other["key"]})
    current_by_id = {row["id"]: row for row in current["bindings"]}
    for row in bindings:
        if row["kind"] != "power_key" or row["trigger"] is None:
            continue
        equivalents = []
        if not row["trigger_gaps"] and not row["operation"].get("action_gaps"):
            for identity in row["matching_action_configuration_ids"]:
                existing = current_by_id[identity]
                if (existing.get("enabled") and not existing.get("conditions")
                        and same_process_scope(existing.get("process_scope", {}), row["trigger"].get("process_scope", {}))
                        and same_keyboard_policy(existing.get("keyboard", {}), row["trigger"]["keyboard"])):
                    equivalents.append(identity)
        row["equivalent_configuration_ids"] = equivalents
        row["status"] = ("configuration_equivalent_unverified" if equivalents else "needs_trigger_support"
                         if row["trigger_gaps"] else "needs_action_support"
                         if row["operation"]["status"] != "native_action_available" else "needs_action_semantics_review"
                         if row["operation"].get("action_gaps") else "needs_runtime_verification")


def build_plan(source, catalog_payload, usage, counters, current, limit=20, body_records=None, mouse_settings=None,
               *, unmodified_mouse_policy="strict", ordinary_mouse_prefixes=None):
    validate_unmodified_mouse_policy(unmodified_mouse_policy)
    selected_ordinary = validate_ordinary_mouse_configuration(ordinary_mouse_prefixes, current)
    mouse_settings = settings_snapshot(mouse_settings if mouse_settings is not None else source.get("MouseSettings"))
    if body_records is not None:
        if isinstance(body_records, dict) and "resultJson" in body_records:
            if body_records.get("success") is not True:
                raise ValueError("动作正文快照未成功导出")
            body_records = json.loads(body_records["resultJson"])
        if isinstance(body_records, dict):
            body_records = body_records.get("Items")
        if not isinstance(body_records, list) or any(not isinstance(record, dict) for record in body_records):
            raise ValueError("动作正文快照需要 Items 数组或记录数组")
        identities = [normalized_id(record.get("Id")) for record in body_records]
        if any(not identity for identity in identities) or len(set(identities)) != len(identities):
            raise ValueError("动作正文快照包含空身份或重复身份")
        body_records = dict(zip(identities, body_records))
    payload = catalog_payload.get("payload", catalog_payload)
    items = payload["items"]
    if payload.get("hasMore") or payload.get("totalCount", len(items)) != len(items):
        raise ValueError("动作目录不完整；应合并同一个 snapshotId 的全部分页后重试")
    catalog = {normalized_id(row["id"]): row for row in items}
    if len(catalog) != len(items):
        raise ValueError("动作目录有重复 ID")
    for key in ("Days30", "Days0"):
        if usage[key].get("IsSuccess") is not True or not isinstance(usage[key].get("Data"), dict):
            raise ValueError("缺少成功的使用统计响应：" + key)
    usage = {key: {"Data": {normalized_id(identity): count for identity, count in usage[key]["Data"].items()}}
             for key in ("Days30", "Days0")}
    excluded = {normalized_id(identity) for row in counters for identity in row["Ids"]}
    recent = ranked_actions(usage["Days30"]["Data"], catalog, excluded, limit)
    historical = ranked_actions(usage["Days0"]["Data"], catalog, excluded, limit)
    bindings = []
    references = collections.defaultdict(list)
    for leader_key, leader in source["PowerKeys"].items():
        if not leader.get("IsEnabled"):
            continue
        tap_policy, tap_gaps, folded_index = leader_tap_policy(leader, source.get("InputSettings") or {})
        ordered = sorted((index for index, row in enumerate(leader.get("KeyActions") or []) if not row.get("IsDisabled")),
                         key=lambda index: -(leader["KeyActions"][index].get("AdornKey") or 0))
        priorities = {index: priority for priority, index in enumerate(ordered)}
        for index, rule in enumerate(leader.get("KeyActions") or []):
            if rule.get("IsDisabled"):
                continue
            identity = f"power:{leader_key}:{index}"
            trigger, gaps, notes = power_trigger(leader, rule, tap_policy, tap_gaps, priorities[index], source.get("InputSettings"))
            folded = index == folded_index
            if folded:
                gaps = [gap for gap in gaps if gap != "unsupported_trigger_key"]
                notes.append({"kind": "standalone_key_folded_into_prefix", "leader": leader["Key"], "policy": tap_policy})
            native = operation(rule, source.get("InputSettings"), body_records)
            overlap = [row for row in current["bindings"] if trigger and same_trigger(trigger, row)]
            action_matches = [row["id"] for row in overlap
                              if row.get("action") == native.get("native_action")
                              and native["status"] == "native_action_available"]
            equivalents = [row["id"] for row in overlap if not gaps and not native.get("action_gaps") and row.get("enabled")
                           and not row.get("conditions") and row["id"] in action_matches
                           and same_process_scope(row.get("process_scope", {}), trigger.get("process_scope", {}))
                           and same_keyboard_policy(row.get("keyboard", {}), trigger["keyboard"])]
            bindings.append({"key": identity, "kind": "power_key", "title": rule.get("Title"),
                             "source": {"leader": {key: value for key, value in leader.items() if key != "KeyActions"},
                                        "rule_index": index, "rule": rule},
                             "trigger": trigger, "trigger_gaps": gaps, "operation": native,
                             "source_normalizations": notes,
                             "folded_into_prefix": leader["Key"] if folded else None,
                             "existing_trigger_ids": [row["id"] for row in overlap],
                             "matching_action_configuration_ids": action_matches,
                             "equivalent_configuration_ids": equivalents,
                             "status": "prefix_policy_projected_unverified" if folded and not gaps
                             else "configuration_equivalent_unverified" if equivalents
                             else "needs_trigger_support" if gaps
                             else "needs_action_support" if native["status"] != "native_action_available"
                             else "needs_action_semantics_review" if native.get("action_gaps")
                             else "needs_runtime_verification"})
            if rule.get("ActionType") == 12:
                references[normalized_id(rule.get("Data"))].append(identity)
    # 鼠标 ID 在实际快照中可能重复；路径位置才是本次快照内稳定的行标识。
    mouse_scopes = [("global", source.get("MouseActions") or [])]
    mouse_scopes += [(scope, value.get("MouseActions") or [])
                     for scope, value in source.get("ExeSettings", {}).items()]
    for scope, rows in mouse_scopes:
        priorities = mouse_rule_priorities(rows) if scope == "global" else {}
        for index, rule in enumerate(rows):
            if not rule.get("IsEnabled"):
                continue
            identity = f"mouse:{scope}:{index}"
            projection = mouse_trigger(rule, mouse_settings, priority=priorities[index], unmodified_mouse_policy=unmodified_mouse_policy,
                                       ordinary_keys=selected_ordinary) if scope == "global" else {"trigger":None,"trigger_gaps":["mouse_application_profile_selection"]}
            if projection["trigger"] and projection["trigger"].get("keyboard", {}).get("matching", {}).get("kind") in ("mouse_exact", "mouse_captured_leader", "mouse_unmodified"):
                projection["source_normalizations"].append({"kind":"mouse_rule_order", "scope":scope,
                    "source_index":index, "priority":priorities[index], "missing_source_priority_default":0,
                    "pointer_whitelist_is_filter_only":True,
                    "runtime_builtin_rules_included":False})
            matching = [item for item in current["bindings"] if projection["trigger"] is not None
                        and item.get("held")==projection["trigger"]["held"]
                        and item.get("activation")==projection["trigger"]["activation"]]
            bindings.append({"key": identity, "kind": "mouse", "title": rule.get("Description"),
                             "source": {"scope": scope, "rule": rule},
                             "operation": operation(rule, source.get("InputSettings"), body_records), "status": "needs_trigger_semantics_review",
                             **projection,
                             "matching_trigger_configuration_ids": [item["id"] for item in matching],
                             "equivalent_configuration_ids": []})
            if rule.get("ActionType") == 12:
                references[normalized_id(rule.get("Data"))].append(identity)
    project_source_precedence(bindings, current)
    selected = set(references) | {row["id"] for row in recent + historical}
    actions = []
    recent_ids = {row["id"] for row in recent}
    historical_ids = {row["id"] for row in historical}
    for identity in sorted(selected):
        entry = catalog.get(identity)
        reasons = (["enabled_binding"] if identity in references else [])
        reasons += [label for label, ids in (("recent30_top", recent_ids), ("historical_top", historical_ids))
                    if identity in ids]
        recent_count = usage["Days30"]["Data"].get(identity, 0)
        defer = not references.get(identity) and recent_count == 0
        actions.append({"id": identity, "title": entry.get("title") if entry else None,
                        "source_type": entry.get("actionType") if entry else None,
                        "reasons": reasons, "binding_keys": references.get(identity, []),
                        "recent30_count": recent_count,
                        "historical_count": usage["Days0"]["Data"].get(identity, 0),
                        "status": "unresolved_identity" if entry is None else "deferred_unused"
                        if defer else "needs_source_review"})
        if entry is not None and not defer and body_records is not None:
            implementation = operation({"ActionType": 12, "Data": identity}, source.get("InputSettings"), body_records)
            actions[-1]["implementation"] = implementation
            if implementation["status"] == "native_action_available" and not implementation.get("action_gaps"):
                actions[-1]["status"] = "needs_runtime_verification"
    plan = {"version": 1, "complete": False,
            "criteria": {"top_limit": limit, "rank_before_catalog_filter": True,
                         "excluded_feature_counters": sorted(excluded),
                         "zero_recent_does_not_defer_enabled_bindings": True,
                         "historical_source": "days=0", "automatic_deletion": False},
            "summary": {"loaded_actions": len(items), "enabled_bindings": len(bindings),
                        "selected_actions": len(actions),
                        "selected_action_available": sum(row.get("implementation", {}).get("status") == "native_action_available" for row in actions),
                        "selected_action_subprogram_calls": sum(len(row.get("implementation", {}).get("dependency_review", {}).get("calls", [])) for row in actions),
                        "selected_action_dependency_inventory_incomplete": sum(row.get("implementation", {}).get("dependency_review", {}).get("inventory_complete") is False for row in actions),
                        "binding_status": dict(collections.Counter(row["status"] for row in bindings)),
                        "trigger_gaps": dict(collections.Counter(gap for row in bindings for gap in row["trigger_gaps"])),
                        "normalizations": dict(collections.Counter(note["kind"] for row in bindings for note in row.get("source_normalizations", []))),
                        "native_action_available": sum(row["operation"]["status"] == "native_action_available" for row in bindings),
                        "native_action_requires_semantics_review": sum(row["operation"]["status"] == "native_action_available"
                            and bool(row["operation"].get("action_gaps")) for row in bindings)},
            "recent30_top": recent, "historical_top": historical,
            "bindings": bindings, "actions": actions}
    if unmodified_mouse_policy == "passed":
        plan["criteria"]["unmodified_mouse_policy"] = "passed"
        plan["criteria"]["minimum_configuration_version"] = 22
        plan["summary"]["mouse_unmodified_projected"] = sum(
            row.get("trigger", {}).get("keyboard", {}).get("matching", {}).get("kind") == "mouse_unmodified"
            for row in bindings if row.get("trigger") is not None)
    if selected_ordinary:
        plan["criteria"]["ordinary_mouse_prefixes"] = sorted(selected_ordinary)
        plan["criteria"]["ordinary_mouse_configuration"] = "preserve_current_keyboard_configuration"
        plan["criteria"]["source_power_key_rows_are_review_only"] = True
        plan["summary"]["ordinary_mouse_prefix_adapted"] = sum(
            any(note.get("kind") == "mouse_ordinary_prefix_adaptation" for note in row.get("source_normalizations", []))
            for row in bindings)
    return plan


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "catalog", "usage", "counters", "current", "out"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--bodies", type=Path, help="可选的已导出正文/模板来源快照，只读")
    parser.add_argument("--mouse-settings", type=Path, help="可选的已导出鼠标全局设置，只读")
    parser.add_argument("--unmodified-mouse-policy", choices=("strict", "passed"), default="strict",
                        help="无控制键鼠标投影：strict 保持原严格策略（默认）；passed 使用配置版本22的已透传输入策略，不启用绑定")
    parser.add_argument("--ordinary-mouse-key", type=int, action="append", default=None,
                        help="显式将该非修饰键的鼠标规则改编为普通Exact；保留当前键盘习惯，拒绝同键捕获owner，不启用绑定；可重复指定不同键")
    args = parser.parse_args(argv)
    paths = {name: getattr(args, name) for name in ("source", "catalog", "usage", "counters", "current")}
    if args.bodies:
        paths["bodies"] = args.bodies
    if args.mouse_settings:
        paths["mouse_settings"] = args.mouse_settings
    plan = build_plan(**{{"catalog":"catalog_payload", "bodies":"body_records"}.get(name, name): load(path)
                         for name, path in paths.items()}, unmodified_mouse_policy=args.unmodified_mouse_policy,
                      ordinary_mouse_prefixes=args.ordinary_mouse_key)
    plan["evidence"] = {name: {"path": str(path.resolve()), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                        for name, path in paths.items()}
    # 独占新建，防止误覆盖输入文件或上次人工审阅结果。
    with args.out.open("x", encoding="utf-8", newline="\n") as file:
        json.dump(plan, file, ensure_ascii=False, indent=2)
        file.write("\n")
    print(json.dumps(plan["summary"], ensure_ascii=False))


if __name__ == "__main__":
    main()
