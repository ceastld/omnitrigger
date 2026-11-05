"""只读候选和动作包，创建全新完整包快照；静态引用核对不执行脚本或 CLI。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat

BLOCKED_ATTRIBUTES = 0x400 | 0x1000 | 0x40000 | 0x400000
MAX_FILE_BYTES = 1024 * 1024
MAX_PACKAGE_BYTES = 16 * 1024 * 1024
MAX_FILES = 512


class SnapshotError(ValueError):
    pass


def absolute(path):
    # 不先 resolve：必须在跟随路径前拒绝符号链接、联接点和占位文件。
    return Path(os.path.abspath(path))


def guard(path, *, missing=False, metadata=os.lstat):
    path = absolute(path)
    current = Path(path.anchor)
    for part in path.parts[1:]:
        current /= part
        lowered = part.casefold()
        if lowered.startswith(("onedrive", "icloud")) or lowered == "dropbox":
            raise SnapshotError("拒绝云盘路径")
        try:
            value = metadata(current)
        except FileNotFoundError:
            if missing:
                continue
            raise SnapshotError("所需路径不存在") from None
        if stat.S_ISLNK(value.st_mode) or getattr(value, "st_file_attributes", 0) & BLOCKED_ATTRIBUTES:
            raise SnapshotError("拒绝链接、联接点或云端占位文件")
    return path


def relative(value):
    if (not isinstance(value, str) or not value or "\\" in value or ":" in value
            or any(character in value for character in "\0?#")
            or any(part in ("", ".", "..") for part in value.split("/"))):
        raise SnapshotError("包内路径必须是未越界的相对路径")
    return value


def within(root, path):
    root, path = absolute(root), absolute(path)
    try:
        path.relative_to(root)
    except ValueError:
        raise SnapshotError("路径超出明确根目录") from None
    return path


def read_bytes(path, maximum=MAX_FILE_BYTES):
    path = guard(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_size > maximum:
        raise SnapshotError("仅接受有界普通单链接文件")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
    with os.fdopen(descriptor, "rb") as file:
        opened = os.fstat(file.fileno())
        if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
            raise SnapshotError("读取前文件身份已变化")
        value = file.read(maximum + 1)
        after = os.fstat(file.fileno())
    guard(path)
    final = path.lstat()
    if (len(value) > maximum or (after.st_size, after.st_mtime_ns) != (before.st_size, before.st_mtime_ns)
            or (final.st_dev, final.st_ino, final.st_size, final.st_mtime_ns)
            != (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)):
        raise SnapshotError("读取期间文件变化或大小超限")
    return value


def decode(value):
    def invalid_constant(_):
        raise SnapshotError("JSON 数字无效")

    def unique(pairs):
        result = {}
        for key, item in pairs:
            if key in result:
                raise SnapshotError("JSON 字段重复")
            result[key] = item
        return result
    try:
        return json.loads(value.decode("utf-8-sig"), object_pairs_hook=unique,
                          parse_constant=invalid_constant)
    except (UnicodeError, json.JSONDecodeError):
        raise SnapshotError("需要有效 UTF-8 JSON") from None


def sha(value):
    return hashlib.sha256(value).hexdigest()


def inventory(root):
    root = guard(root)
    result = {}
    total = 0
    directories = 0
    pending = [root]
    while pending:
        directory = guard(pending.pop())
        directories += 1
        if directories > MAX_FILES:
            raise SnapshotError("动作包目录数超限")
        with os.scandir(directory) as entries:
            children = sorted(entries, key=lambda item: item.name)
        for item in children:
            path = guard(within(root, directory / item.name))
            metadata = path.lstat()
            if stat.S_ISDIR(metadata.st_mode):
                pending.append(path)
            elif stat.S_ISREG(metadata.st_mode):
                name = relative(path.relative_to(root).as_posix())
                value = read_bytes(path)
                result[name] = value
                total += len(value)
                if len(result) > MAX_FILES or total > MAX_PACKAGE_BYTES:
                    raise SnapshotError("动作包文件数或总大小超限")
            else:
                raise SnapshotError("动作包包含不受支持的文件类型")
    return result


def manifest(value):
    if (not isinstance(value, dict) or type(value.get("schema_version")) is not int
            or value["schema_version"] != 1 or not isinstance(value.get("id"), str) or not value["id"]):
        raise SnapshotError("动作包清单无效")
    actions = value.get("actions")
    if not isinstance(actions, list) or not actions:
        raise SnapshotError("动作包没有动作")
    ids = set()
    for action in actions:
        if (not isinstance(action, dict) or not isinstance(action.get("id"), str) or not action["id"]
                or action["id"] in ids or type(action.get("version")) is not int or action["version"] < 1
                or action.get("runtime") not in ("node", "script")):
            raise SnapshotError("动作定义无效或重复")
        relative(action.get("entry"))
        ids.add(action["id"])
    return value


def prepare(candidate_dir, reference_result, packages_root):
    candidate_dir, reference_result, packages_root = map(guard, (candidate_dir, reference_result, packages_root))
    reference_bytes = read_bytes(reference_result)
    reference = decode(reference_bytes)
    if not isinstance(reference, dict) or reference.get("passed") is not True or not isinstance(reference.get("results"), list):
        raise SnapshotError("需要明确通过的历史静态引用结果")
    previous = {}
    for row in reference["results"]:
        key = row.get("source_id")
        if not isinstance(key, str) or key in previous:
            raise SnapshotError("历史引用身份缺失或重复")
        previous[key] = row
    candidates = inventory(candidate_dir)
    if any("/" in name or not name.endswith(".json") for name in candidates):
        raise SnapshotError("候选目录只接受平铺 JSON")
    if {Path(name).stem for name in candidates} != set(previous):
        raise SnapshotError("候选与历史引用集合不一致")

    packages = {}
    with os.scandir(packages_root) as entries:
        children = sorted(entries, key=lambda item: item.name)
    for item in children:
        path = guard(packages_root / item.name)
        if not path.is_dir():
            continue
        clear_manifest = guard(path / "action.json", missing=True)
        if not clear_manifest.exists():
            continue
        data = manifest(decode(read_bytes(clear_manifest)))
        if data["id"] in packages:
            raise SnapshotError("动作包身份重复")
        packages[data["id"]] = (path, data)

    calls = []
    selected = {}
    for name, value in candidates.items():
        candidate = decode(value)
        if not isinstance(candidate, dict):
            raise SnapshotError("候选必须为 JSON 对象")
        old = previous[Path(name).stem]
        if old.get("kind") == "ActionSpec":
            if candidate.get("kind") != "plugin":
                raise SnapshotError("原生 ActionSpec 与 ActionCall 不可混用")
            calls.append({"candidate": name, "kind": "ActionSpec", "static_reference": "not_an_ActionCall",
                          "module_check": "not_applicable", "sha256": sha(value), "executed": False})
            continue
        if old.get("kind") != "ActionCall" or old.get("call_sha256") != sha(value):
            raise SnapshotError("候选调用与原证据哈希不一致")
        if not isinstance(candidate, dict) or set(candidate) != {"package_id", "action_id", "action_version", "input"}:
            raise SnapshotError("ActionCall 字段无效")
        identity = candidate["package_id"]
        if identity not in packages:
            raise SnapshotError("没有对应的本项目动作包")
        package_root, package_manifest = packages[identity]
        action = next((row for row in package_manifest["actions"] if row["id"] == candidate["action_id"]), None)
        if (action is None or type(candidate["action_version"]) is not int
                or action["version"] != candidate["action_version"] or not isinstance(candidate["input"], dict)):
            raise SnapshotError("动作身份或版本不匹配")
        if identity not in selected:
            files = inventory(package_root)
            if decode(files.get("action.json", b"null")) != package_manifest:
                raise SnapshotError("清单在扫描期间变化")
            if any(row["entry"] not in files for row in package_manifest["actions"]):
                raise SnapshotError("包内入口缺失")
            selected[identity] = {"source": package_root, "manifest": package_manifest, "files": files}
        calls.append({"candidate": name, "kind": "ActionCall", "package_id": identity,
                      "action_id": action["id"], "entry": action["entry"], "runtime": action["runtime"],
                      "static_reference": "identity_version_entry_present", "sha256": sha(value), "executed": False})
    return candidates, selected, calls, {"path": str(reference_result), "sha256": sha(reference_bytes)}


def snapshot(candidate_dir, reference_result, packages_root, output):
    output = guard(output, missing=True)
    if output.exists():
        raise SnapshotError("输出目录必须全新，不覆盖已有证据")
    for source in (candidate_dir, packages_root):
        try:
            output.relative_to(absolute(source))
        except ValueError:
            pass
        else:
            raise SnapshotError("输出不能位于输入目录中")
    if not guard(output.parent).is_dir():
        raise SnapshotError("输出父目录必须已存在")
    candidates, packages, calls, previous = prepare(candidate_dir, reference_result, packages_root)
    output.mkdir(exist_ok=False)

    def write(name, value):
        path = guard(within(output, output / relative(name)), missing=True)
        path.parent.mkdir(parents=True, exist_ok=True)
        guard(path.parent)
        with path.open("xb") as file:
            file.write(value)

    for name, value in candidates.items():
        write("candidates/" + name, value)
    copied = []
    for identity, package in sorted(packages.items()):
        folder = "packages/" + package["source"].name
        for name, value in package["files"].items():
            write(folder + "/" + name, value)
        copied.append({"id": identity, "source_root": str(package["source"]), "snapshot_root": folder,
                       "manifest": package["manifest"], "files": [{"path": name, "sha256": sha(value), "bytes": len(value)}
                           for name, value in sorted(package["files"].items())]})
    report = {"version": 1, "previous_reference": previous, "packages": copied, "candidates": calls,
              "static_reference_passed": True, "module_graph_checked": False, "trusted_node_import_checked": False,
              "scope": "完整文件快照和身份/版本/入口存在性；不解析或执行 JS，不调用 check-action",
              "actions_executed": False, "activated": False}
    write("snapshot.json", (json.dumps(report, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))
    return report


def verify_snapshot(path, trusted_root=None):
    path = guard(path)
    root = path.parent
    report = decode(read_bytes(path))
    if report.get("version") != 1 or report.get("static_reference_passed") is not True:
        raise SnapshotError("快照报告无效")
    candidates = inventory(root / "candidates")
    if {name: sha(value) for name, value in candidates.items()} != {
            relative(row["candidate"]): row["sha256"] for row in report["candidates"]}:
        raise SnapshotError("快照候选集合或哈希已变化")
    for package in report["packages"]:
        package_root = within(root, root / relative(package["snapshot_root"]))
        current = inventory(package_root)
        expected = {relative(row["path"]): row["sha256"] for row in package["files"]}
        if len(expected) != len(package["files"]):
            raise SnapshotError("快照文件清单重复")
        if {name: sha(value) for name, value in current.items()} != expected:
            raise SnapshotError("快照文件集合或哈希已变化")
        if manifest(decode(current["action.json"])) != package["manifest"]:
            raise SnapshotError("快照清单与报告不一致")
        if trusted_root is not None:
            trusted_root = guard(trusted_root)
            original = guard(package["source_root"])
            if original.parent != trusted_root:
                raise SnapshotError("不是明确本项目的直接动作包")
            trusted_files = inventory(original)
            if {name: sha(value) for name, value in trusted_files.items()} != expected:
                raise SnapshotError("快照不再匹配可信项目完整包")
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-snapshot", type=Path, help="只核对新快照完整性，不执行模块")
    parser.add_argument("--trusted-root", type=Path, help="验证完整包与明确的可信本项目目录一致")
    for name in ("candidate-dir", "reference-result", "packages-root", "out"):
        parser.add_argument("--" + name, type=Path)
    args = parser.parse_args(argv)
    if args.verify_snapshot:
        if any((args.candidate_dir, args.reference_result, args.packages_root, args.out)):
            parser.error("验证与创建快照参数不能混用")
        report = verify_snapshot(args.verify_snapshot, args.trusted_root)
        print(json.dumps({"verified": True, "packages": len(report["packages"]), "actions_executed": False}))
        return
    if args.trusted_root or not all((args.candidate_dir, args.reference_result, args.packages_root, args.out)):
        parser.error("创建快照需要四个明确输入/输出路径")
    report = snapshot(args.candidate_dir, args.reference_result, args.packages_root, args.out)
    print(json.dumps({"candidates": len(report["candidates"]), "packages": len(report["packages"]),
                      "actions_executed": False}))


if __name__ == "__main__":
    main()
