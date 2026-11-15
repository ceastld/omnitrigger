"""用已有热键动作生成一份轮盘迁移候选，保留每个布局的位置、场景和名称。"""

import argparse
import json
import sys
from copy import deepcopy
from pathlib import Path

from snapshot_action_candidates import decode, guard


def identity(value):
    # 动作输入可以包含布尔值，不能把 JSON true 与数字 1 当作相同参数。
    return json.dumps(value, ensure_ascii=False, sort_keys=True, allow_nan=False)


def unique(rows, key, value):
    matches = [row for row in rows if row[key] == value]
    if len(matches) != 1:
        raise ValueError(f"需要唯一的 {key}={value}，实际找到 {len(matches)} 项。")
    return matches[0]


def position_of(slot):
    position = slot.get("position")
    if position is None:
        return None
    return (position["ring"], position["sector"], position["sectors"], position.get("half_step", False))


def migrate(bindings, wheels, binding_id, wheel_id, base_slot):
    action = unique(bindings["bindings"], "id", binding_id)["action"]
    if action["kind"] in {"show_panel", "show_wheel"}:
        raise ValueError("轮盘槽位不能打开面板或另一个轮盘。")
    candidate = deepcopy(wheels)
    wheel = unique(candidate["wheels"], "id", wheel_id)
    base = wheel["slots"]
    if not 1 <= base_slot <= len(base):
        raise ValueError("通用布局中不存在该槽位，序号从 1 开始。")
    selected = base[base_slot - 1]
    old = identity(selected["action"])
    position = position_of(selected)
    scene = selected.get("scene")
    changed = []
    if identity(action) == old:
        return candidate, changed

    def replace(slot, layout):
        slot["action"] = deepcopy(action)
        changed.append(layout)

    replace(selected, None)
    for override in wheel.get("app_overrides", []):
        slots = override["slots"]
        for index, slot in enumerate(slots, 1):
            if slot.get("scene") != scene or position_of(slot) != position:
                continue
            # 旧格式均分整个圆。只有格数和序号都相同，才是同一个方位。
            if position is None and (len(slots) != len(base) or index != base_slot):
                continue
            if identity(slot["action"]) == old:
                replace(slot, override["process"])
    return candidate, changed


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bindings", type=Path, required=True)
    parser.add_argument("--wheels", type=Path, required=True)
    parser.add_argument("--binding-id", type=int, required=True)
    parser.add_argument("--wheel-id", required=True)
    parser.add_argument("--base-slot", type=int, required=True, help="通用布局 slots 数组中的序号，从 1 开始")
    parser.add_argument("--output", type=Path, required=True, help="新建候选文件的路径")
    args = parser.parse_args(argv)
    try:
        bindings = decode(guard(args.bindings).read_bytes())
        source = guard(args.wheels).read_bytes()
        wheels = decode(source)
        candidate, changed = migrate(bindings, wheels, args.binding_id, args.wheel_id, args.base_slot)
        data = source
        if changed:
            text = json.dumps(candidate, ensure_ascii=False, indent=2, allow_nan=False)
            if source.endswith(b"\n"):
                text += "\n"
            if b"\r\n" in source:
                text = text.replace("\n", "\r\n")
            data = text.encode("utf-8-sig" if source.startswith(b"\xef\xbb\xbf") else "utf-8")
        output_path = guard(args.output, missing=True)
        output = output_path.open("xb")
        try:
            with output:
                output.write(data)
        except OSError:
            output_path.unlink()
            raise
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        print(f"生成轮盘候选失败：{error}", file=sys.stderr)
        return 1
    print(json.dumps({"wheel": args.wheel_id, "changed": len(changed), "layouts": changed}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
