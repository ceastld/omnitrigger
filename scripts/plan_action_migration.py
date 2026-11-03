"""只读评估离线动作快照；不连接源程序、不安装触发器、不运行动作。"""

import argparse
import collections
import hashlib
import json
from pathlib import Path


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
    """V1 modifier|VK；输出按键使用明确左侧修饰键，不解析 SendKeys 文本。"""
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
    keys = [key for bit, key in ((2, 162), (4, 160), (1, 164), (8, 91))
            if modifiers & bit]
    return {"kind": "send_shortcut", "keys": keys + [terminal]}


def operation(row):
    if row.get("ActionType") == 1:
        native = shortcut_action(row.get("Data"))
        gaps = ["notification_side_effect"] if row.get("Message") else []
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
        if set(canonical) & {"explorer", "desktop", "taskbar"}:
            gaps.append("legacy_shell_window_role")
    if sum(len(names) for names in result.values()) > 64:
        gaps.append("unsupported_process_list")
    allowed = {normalized_process_name(item) for item in result.get("allow", [])}
    denied = {normalized_process_name(item) for item in result.get("deny", [])}
    if allowed and allowed <= denied:
        gaps.append("empty_process_scope")
    return result, gaps, notes


def same_process_scope(left, right):
    def signature(value):
        return tuple(tuple(sorted(normalized_process_name(item) for item in value.get(key, [])))
                     for key in ("allow", "deny"))
    return signature(left) == signature(right)


def same_keyboard_policy(left, right):
    return (left.get("dedicated_prefix") == right.get("dedicated_prefix")
            and left.get("repeat", False) == right.get("repeat", False)
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


def power_trigger(leader, rule, tap_policy=None, tap_gaps=()):
    scope, gaps, notes = process_scope(leader, rule)
    gaps.extend(tap_gaps)
    dedicated = leader.get("KeepOriginKeyFunc") is False
    if not dedicated:
        # V1 的保留原功能模式包含快速输入保护、松开顺序和超时，不能按普通回退宣称等价。
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
    if dedicated:
        prefix = button(leader.get("Key"))
        if prefix["kind"] == "key":
            keyboard["dedicated_prefix"] = prefix
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
    def names(scope, field):
        return {normalized_process_name(name) for name in scope.get(field, [])}
    left_allow, right_allow = names(left, "allow"), names(right, "allow")
    denied = names(left, "deny") | names(right, "deny")
    if left_allow and right_allow:
        return bool((left_allow & right_allow) - denied)
    if left_allow or right_allow:
        return bool((left_allow or right_allow) - denied)
    return True


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
                "unsupported_process_list", "empty_process_scope", "legacy_shell_window_role"} for row in rows):
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
                    row["source_normalizations"].append({"kind": "default_excludes_specific_rules", "names": added})
            if sum(len(names) for names in scope.values()) > 64:
                row["trigger_gaps"] = sorted(set(row["trigger_gaps"] + ["unsupported_process_list"]))
    candidates = [row for row in bindings if row.get("trigger")]
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
            for row, other in ((terminal, prefix), (prefix, terminal)):
                row["trigger_gaps"] = sorted(set(row["trigger_gaps"] + ["terminal_prefix_overlap"]))
                row["source_normalizations"].append({"kind": "terminal_prefix_review", "other": other["key"]})
    current_by_id = {row["id"]: row for row in current["bindings"]}
    for row in bindings:
        if row["kind"] != "power_key" or row["trigger"] is None:
            continue
        equivalents = []
        if not row["trigger_gaps"]:
            for identity in row["matching_action_configuration_ids"]:
                existing = current_by_id[identity]
                if (existing.get("enabled") and not existing.get("conditions")
                        and same_process_scope(existing.get("process_scope", {}), row["trigger"].get("process_scope", {}))
                        and same_keyboard_policy(existing.get("keyboard", {}), row["trigger"]["keyboard"])):
                    equivalents.append(identity)
        row["equivalent_configuration_ids"] = equivalents
        row["status"] = ("configuration_equivalent_unverified" if equivalents else "needs_trigger_support"
                         if row["trigger_gaps"] else "needs_action_support"
                         if row["operation"]["status"] != "native_action_available" else "needs_runtime_verification")


def build_plan(source, catalog_payload, usage, counters, current, limit=20):
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
        for index, rule in enumerate(leader.get("KeyActions") or []):
            if rule.get("IsDisabled"):
                continue
            identity = f"power:{leader_key}:{index}"
            trigger, gaps, notes = power_trigger(leader, rule, tap_policy, tap_gaps)
            folded = index == folded_index
            if folded:
                gaps = [gap for gap in gaps if gap != "unsupported_trigger_key"]
                notes.append({"kind": "standalone_key_folded_into_prefix", "leader": leader["Key"], "policy": tap_policy})
            native = operation(rule)
            overlap = [row for row in current["bindings"] if trigger and same_trigger(trigger, row)]
            action_matches = [row["id"] for row in overlap
                              if row.get("action") == native.get("native_action")
                              and native["status"] == "native_action_available"]
            equivalents = [row["id"] for row in overlap if not gaps and row.get("enabled")
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
                             else "needs_runtime_verification"})
            if rule.get("ActionType") == 12:
                references[normalized_id(rule.get("Data"))].append(identity)
    # 鼠标 ID 在实际快照中可能重复；路径位置才是本次快照内稳定的行标识。
    mouse_scopes = [("global", source.get("MouseActions") or [])]
    mouse_scopes += [(scope, value.get("MouseActions") or [])
                     for scope, value in source.get("ExeSettings", {}).items()]
    for scope, rows in mouse_scopes:
        for index, rule in enumerate(rows):
            if not rule.get("IsEnabled"):
                continue
            identity = f"mouse:{scope}:{index}"
            bindings.append({"key": identity, "kind": "mouse", "title": rule.get("Description"),
                             "source": {"scope": scope, "rule": rule},
                             "operation": operation(rule), "status": "needs_trigger_semantics_review",
                             "trigger_gaps": ["mouse_rule_semantics"],
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
    return {"version": 1, "complete": False,
            "criteria": {"top_limit": limit, "rank_before_catalog_filter": True,
                         "excluded_feature_counters": sorted(excluded),
                         "zero_recent_does_not_defer_enabled_bindings": True,
                         "historical_source": "days=0", "automatic_deletion": False},
            "summary": {"loaded_actions": len(items), "enabled_bindings": len(bindings),
                        "selected_actions": len(actions),
                        "binding_status": dict(collections.Counter(row["status"] for row in bindings)),
                        "trigger_gaps": dict(collections.Counter(gap for row in bindings for gap in row["trigger_gaps"])),
                        "normalizations": dict(collections.Counter(note["kind"] for row in bindings for note in row.get("source_normalizations", []))),
                        "native_action_available": sum(row["operation"]["status"] == "native_action_available" for row in bindings)},
            "recent30_top": recent, "historical_top": historical,
            "bindings": bindings, "actions": actions}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "catalog", "usage", "counters", "current", "out"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args(argv)
    paths = {name: getattr(args, name) for name in ("source", "catalog", "usage", "counters", "current")}
    plan = build_plan(**{name if name != "catalog" else "catalog_payload": load(path)
                         for name, path in paths.items()})
    plan["evidence"] = {name: {"path": str(path.resolve()), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                        for name, path in paths.items()}
    # 独占新建，防止误覆盖输入文件或上次人工审阅结果。
    with args.out.open("x", encoding="utf-8", newline="\n") as file:
        json.dump(plan, file, ensure_ascii=False, indent=2)
        file.write("\n")
    print(json.dumps(plan["summary"], ensure_ascii=False))


if __name__ == "__main__":
    main()
