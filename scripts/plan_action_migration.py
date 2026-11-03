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


def power_trigger(leader, rule):
    gaps = []
    if rule.get("BindingProcessName"):
        gaps.append("foreground_process_scope")
    if leader.get("BlackList"):
        gaps.append("leader_process_exclusions")
    if leader.get("KeepOriginKeyFunc") is not True:
        gaps.append("suppress_unused_prefix")
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
        return None, sorted(set(gaps))
    if "either_side_modifier_matching" in gaps:
        return None, sorted(set(gaps))
    return {"held": held, "activation": {"kind": "press", "value": terminal}}, sorted(set(gaps))


def same_trigger(left, right):
    sort_key = lambda item: json.dumps(item, sort_keys=True)
    return (sorted(left.get("held", []), key=sort_key) == sorted(right.get("held", []), key=sort_key)
            and left.get("activation") == right.get("activation"))


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
        for index, rule in enumerate(leader.get("KeyActions") or []):
            if rule.get("IsDisabled"):
                continue
            identity = f"power:{leader_key}:{index}"
            trigger, gaps = power_trigger(leader, rule)
            native = operation(rule)
            overlap = [row for row in current["bindings"] if trigger and same_trigger(trigger, row)]
            action_matches = [row["id"] for row in overlap
                              if row.get("action") == native.get("native_action")
                              and native["status"] == "native_action_available"]
            equivalents = [row["id"] for row in overlap if not gaps and row.get("enabled")
                           and not row.get("conditions") and row["id"] in action_matches]
            bindings.append({"key": identity, "kind": "power_key", "title": rule.get("Title"),
                             "source": {"leader": {key: value for key, value in leader.items() if key != "KeyActions"},
                                        "rule_index": index, "rule": rule},
                             "trigger": trigger, "trigger_gaps": gaps, "operation": native,
                             "existing_trigger_ids": [row["id"] for row in overlap],
                             "matching_action_configuration_ids": action_matches,
                             "equivalent_configuration_ids": equivalents,
                             "status": "configuration_equivalent_unverified" if equivalents
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
