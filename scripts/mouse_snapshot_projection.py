"""显式离线投影鼠标修饰快照策略；不注册、不启用、不执行动作。"""
import argparse
import copy
from dataclasses import dataclass
import json
import pathlib

MINIMUM_INPUT_VERSION = 22
SNAPSHOT_CONFIG_VERSION = 23


@dataclass(frozen=True)
class ModifierFlags:
    allow_other_modifiers: bool
    allow_extra_passed_keys: bool
    allow_extra_passed_mouse: bool

    def validate(self):
        if any(type(value) is not bool for value in self.values()):
            raise ValueError("鼠标快照放行选项必须是显式布尔值")

    def values(self):
        return (self.allow_other_modifiers, self.allow_extra_passed_keys, self.allow_extra_passed_mouse)

    def policy(self, required):
        self.validate()
        return {"required": required, "allow_other_modifiers": self.allow_other_modifiers,
                "allow_extra_passed_keys": self.allow_extra_passed_keys, "allow_extra_passed_mouse": self.allow_extra_passed_mouse}


PROFILES = {
    "required-only": ModifierFlags(False, True, False),
    "passed-extras": ModifierFlags(True, True, True),
}
POSITIVE_REQUIREMENTS = {"shift", "control", "alt", "left_shift", "right_shift", "left_control", "right_control", "left_alt", "right_alt", "left_windows", "right_windows"}


def project_mouse_snapshots(document, *, modifier_flags, unmodified_flags=None):
    """显式转换已表达的Ctrl/左Alt及无控制键候选，其他记录逐字段保留。

    unmodified_flags=None保留旧观察模式的额外输入能力：mouse_unmodified全放行，
    空mouse_exact全排除；两者均新增快照无CSA要求。此独立策略不声明来源等价。
    """
    if not isinstance(modifier_flags, ModifierFlags) or unmodified_flags is not None and not isinstance(unmodified_flags, ModifierFlags):
        raise ValueError("快照投影须提供明确的ModifierFlags策略")
    modifier_flags.validate()
    if unmodified_flags is not None:
        unmodified_flags.validate()
    if not isinstance(document, dict) or type(document.get("version")) is not int or not MINIMUM_INPUT_VERSION <= document["version"] <= SNAPSHOT_CONFIG_VERSION:
        raise ValueError(f"显式快照投影只接受版本{MINIMUM_INPUT_VERSION}至{SNAPSHOT_CONFIG_VERSION}的配置对象")
    bindings = document.get("bindings")
    if not isinstance(bindings, list) or len(bindings) > 512:
        raise ValueError("配置须包含最多512项bindings数组")
    ids = []
    for binding in bindings:
        if not isinstance(binding, dict) or type(binding.get("id")) is not int or not 0 <= binding["id"] <= 2**64 - 1:
            raise ValueError("每项候选须有u64编号")
        ids.append(binding["id"])
    if len(ids) != len(set(ids)):
        raise ValueError("候选编号不能重复")
    result = copy.deepcopy(document)
    changes = []
    skipped = []
    for binding in result["bindings"]:
        keyboard = binding.get("keyboard", {})
        if not isinstance(keyboard, dict):
            raise ValueError("keyboard须为对象")
        matching = keyboard.get("matching", {"kind": "exact"})
        if not isinstance(matching, dict):
            raise ValueError("matching须为对象")
        kind = matching.get("kind")
        held = binding.get("held")
        if not isinstance(held, list):
            raise ValueError("held须为数组")
        required = None
        flags = None
        if kind == "mouse_unmodified" or kind == "mouse_exact" and not held:
            required = "no_control_shift_alt"
            flags = unmodified_flags or (ModifierFlags(True, True, True) if kind == "mouse_unmodified" else ModifierFlags(False, False, False))
        elif kind == "mouse_exact" and len(held) == 1 and isinstance(held[0], dict):
            button = held[0]
            if button.get("kind") == "key" and type(button.get("value")) is int:
                required = {17: "control", 164: "left_alt"}.get(button["value"])
                flags = modifier_flags
                if required is not None and set(button) != {"kind", "value"}:
                    raise ValueError("待转换held包含未知字段，未自动丢弃")
        elif kind == "mouse_sampled":
            previous = matching.get("policy")
            if not isinstance(previous, dict):
                raise ValueError("已有快照匹配必须包含policy对象")
            if set(previous) != {"required", "allow_other_modifiers", "allow_extra_passed_keys", "allow_extra_passed_mouse"}:
                raise ValueError("已有快照policy包含未知或缺失字段")
            if type(previous.get("required")) is not str or any(type(previous.get(field)) is not bool for field in ("allow_other_modifiers", "allow_extra_passed_keys", "allow_extra_passed_mouse")):
                raise ValueError("已有快照策略须包含字符串required及三项布尔放行选项")
            required = previous.get("required")
            if required == "no_control_shift_alt":
                if unmodified_flags is None:
                    flags = ModifierFlags(previous["allow_other_modifiers"], previous["allow_extra_passed_keys"], previous["allow_extra_passed_mouse"])
                else:
                    flags = unmodified_flags
            elif required in ("control", "left_alt"):
                flags = modifier_flags
            else:
                required = None
        if required is None:
            if isinstance(kind, str) and kind.startswith("mouse_"):
                skipped.append(binding["id"])
            continue
        activation = binding.get("activation", {})
        supported = activation.get("kind") in ("wheel_up", "wheel_down", "wheel_left", "wheel_right") or activation.get("kind") == "press" and isinstance(activation.get("value"), dict) and activation["value"].get("kind") in ("left", "right", "middle", "x1", "x2")
        if not supported:
            raise ValueError("快照投影只支持鼠标按下或滚轮")
        if any(keyboard.get(field) is not None for field in ("dedicated_prefix", "retained_prefix", "tap_timeout_ms")) or any(keyboard.get(field, False) is not False for field in ("repeat", "restore_on_tap")) or not _global_tap_scope(keyboard.get("tap_scope", {})):
            raise ValueError("快照候选不能带捕获层、重复或单按恢复配置")
        if kind in ("mouse_unmodified", "mouse_sampled") and held:
            raise ValueError("无控制键或已有快照候选的held必须为空")
        priority = matching.get("priority", 0)
        pointer_priority = matching.get("pointer_scope_priority", True)
        if type(priority) is not int or not 0 <= priority <= 511 or type(pointer_priority) is not bool:
            raise ValueError("鼠标顺序须为0至511，指针优先级选项须为布尔值")
        allowed_fields = {"kind", "priority", "pointer_scope_priority"} | ({"policy"} if kind == "mouse_sampled" else set())
        if set(matching) - allowed_fields:
            raise ValueError("待转换matching包含未知字段，未自动丢弃")
        before = copy.deepcopy(matching)
        old_held = copy.deepcopy(held)
        replacement = {key: value for key, value in matching.items() if key != "policy"}
        replacement["kind"] = "mouse_sampled"
        replacement["policy"] = flags.policy(required)
        binding["keyboard"]["matching"] = replacement
        binding["held"] = []
        if replacement != before or old_held:
            changes.append({"id": binding["id"], "required": required, "old_held": old_held, "priority": priority, "pointer_scope_priority": pointer_priority, "flags": flags.policy(required)})
    result["version"] = SNAPSHOT_CONFIG_VERSION
    sampled = [binding for binding in result["bindings"] if binding.get("keyboard", {}).get("matching", {}).get("kind") == "mouse_sampled"]
    prefix_relations = _prefix_entry_relations(result["bindings"])
    report = {"version": SNAPSHOT_CONFIG_VERSION, "bindings": len(bindings), "enabled": sum(binding.get("enabled") is True for binding in bindings),
              "changed": changes, "sampled_count": len(sampled), "unsupported_mouse_candidates_preserved": skipped,
              "sampled_unmodified_count": sum(binding["keyboard"]["matching"]["policy"].get("required") == "no_control_shift_alt" for binding in sampled),
              "actions_preserved": all(left.get("action") == right.get("action") for left, right in zip(bindings, result["bindings"])),
              "potential_prefix_entry_relations": prefix_relations,
              "priorities_preserved": True, "personal_configuration_applied": False, "registration_performed": False, "source_equivalence": False}
    return result, report


def _prefix_entry_relations(bindings):
    """只报告形状上的入口关系；实际范围/区域和可达性仍由核心注册校验。"""
    relations = []
    for terminal in bindings:
        matching = terminal.get("keyboard", {}).get("matching", {})
        activation = terminal.get("activation", {})
        button = activation.get("value")
        if matching.get("kind") != "mouse_sampled" or activation.get("kind") != "press" or not isinstance(button, dict) or button.get("kind") not in ("left", "right", "middle", "x1", "x2"):
            continue
        required = matching["policy"].get("required")
        for prefix in bindings:
            keyboard = prefix.get("keyboard", {})
            target = prefix.get("activation", {})
            if keyboard.get("matching", {"kind": "exact"}).get("kind") != "exact" or prefix.get("held") != [button] or keyboard.get("dedicated_prefix") is not None or keyboard.get("retained_prefix") is not None:
                continue
            mouse_target = target.get("kind") == "press" and isinstance(target.get("value"), dict) and target["value"].get("kind") in ("left", "right", "middle", "x1", "x2")
            if target.get("kind") not in ("wheel_up", "wheel_down", "wheel_left", "wheel_right") and not mouse_target:
                continue
            relations.append({"terminal_id": terminal["id"], "prefix_id": prefix["id"], "required": required,
                              "relation": "conditional_preemption" if required in POSITIVE_REQUIREMENTS else "canonical_prefix_shadow_or_review",
                              "scope_region_and_registration_review_required": True})
    return relations


def _strict_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("JSON包含重复字段")
        result[key] = value
    return result


def _global_tap_scope(scope):
    return isinstance(scope, dict) and not set(scope) - {"identity", "allow", "deny"} and scope.get("identity", "executable") in ("executable", "shell_context") and scope.get("allow", []) == [] and scope.get("deny", []) == []


def _reject_constant(_):
    raise ValueError("JSON不能包含非有限数值")


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--input", required=True, type=pathlib.Path)
    result.add_argument("--output", required=True, type=pathlib.Path)
    result.add_argument("--policy", required=True, choices=[*PROFILES, "custom"], help="required-only排除其他修饰键/鼠标但允许透传普通键；passed-extras允许其他修饰键及透传键鼠；custom须给出三项选项。无CSA配套规则保留其原额外输入能力。")
    for field in ("other-modifiers", "extra-passed-keys", "extra-passed-mouse"):
        result.add_argument("--allow-" + field, action=argparse.BooleanOptionalAction, default=None)
    return result


def main(argv=None):
    arguments = parser().parse_args(argv)
    values = [arguments.allow_other_modifiers, arguments.allow_extra_passed_keys, arguments.allow_extra_passed_mouse]
    if arguments.policy == "custom":
        if any(value is None for value in values):
            raise ValueError("custom策略须明确指定三项allow/no-allow选项")
        flags = ModifierFlags(*values)
    else:
        if any(value is not None for value in values):
            raise ValueError("命名策略不能混用自定义选项，请选择custom")
        flags = PROFILES[arguments.policy]
    # 先检查祖先与文件占位属性，再解析最终路径或读取内容。
    for candidate in [*reversed(arguments.input.parents), arguments.input]:
        attributes = getattr(candidate.lstat(), "st_file_attributes", 0)
        if attributes & (0x1000 | 0x40000 | 0x400000 | 0x400) or candidate.is_symlink():
            raise ValueError("跳过云端占位或链接输入")
    input_path, output_path = arguments.input.resolve(), arguments.output.resolve()
    if input_path == output_path:
        raise ValueError("离线输出不能覆盖输入配置")
    document = json.loads(input_path.read_text(encoding="utf-8"), object_pairs_hook=_strict_pairs, parse_constant=_reject_constant)
    converted, report = project_mouse_snapshots(document, modifier_flags=flags)
    payload = (json.dumps(converted, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    # x模式使并发创建或已有输出明确失败，不悄悄覆盖另一个审阅结果。
    with output_path.open("xb") as stream:
        stream.write(payload)
    print(json.dumps(report, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        raise SystemExit(f"快照投影失败：{error}") from error
