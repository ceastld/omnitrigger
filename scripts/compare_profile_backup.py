"""只读比对配置 ZIP 和本机档案中的热键、轮盘、托管动作及扩展持久状态。"""
import argparse
from collections import deque
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
from typing import NamedTuple
import zipfile

from snapshot_action_candidates import decode, guard


CONFIG_FILES = (
    "profiles.json", "profile.json", "bindings.json", "wheels.json",
    "appearance.json", "plugins.json", "plugins/raishot/manifest.json",
)
COPY_ORIGIN = ".copy-origin.json"
PROFILE_RECORDS = ("backups/seed-source.json", "backups/restored-revisions.json")
STAGE = re.compile(r"\.install-legacy-(?:upgrade-)?[0-9]+-[0-9]+-[0-9]+", re.I)
PROFILE_ID = re.compile(r"t[0-9a-f]{12}")
STATE_DOCUMENT = re.compile(r"[0-9a-f]{64}\.json", re.IGNORECASE | re.ASCII)


def catalog_identity(root):
    """旧测试档单独导出时，按安装版的字段顺序生成 ZIP 顶层的身份文件。"""
    if (root.parent.name.casefold() != "profiles" or not PROFILE_ID.fullmatch(root.name.lower())
            or guard(root / "profile.json", missing=True).exists()):
        return None
    path = guard(root.parent.parent / "profiles.json", missing=True)
    if not path.exists():
        return None
    catalog = decode(path.read_bytes())
    if catalog["version"] != 1:
        raise ValueError(f"不支持的配置档登记版本：{path}")
    keys = ("id", "alias", "kind", "name", "seed", "created_ms")
    for entry in catalog["profiles"]:
        if entry["kind"] == "test" and os.path.normcase(entry["id"]) == os.path.normcase(root.name):
            if (set(entry) != set(keys) or not all(isinstance(entry[k], str) for k in keys[:-1])
                    or type(entry["created_ms"]) is not int or not 0 <= entry["created_ms"] < 2**64
                    or not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,39}", entry["alias"]) or entry["alias"] == "default"):
                raise ValueError(f"测试档登记无效：{path}")
            return json.dumps({k: entry[k] for k in keys}, indent=2, ensure_ascii=False).encode("utf-8")
    return None


def profile_dirs(app, root):
    app = guard(app)
    directories = []
    pending = deque([root])
    while pending:
        parent = pending.popleft()
        directories.append(parent)
        catalog = guard(parent / "profiles.json", missing=True)
        children = guard(parent / "profiles", missing=True)
        if parent != root and not catalog.exists() and not children.exists():
            # 父档也可能在缺少总登记时从中断的重置记录找回子档身份。
            if not guard(parent / "backups/profile-reset", missing=True).exists():
                continue
        pending.extend(registered_children(app, parent))
    return directories


def registered_children(app, parent):
    result = subprocess.run(
        [str(app), "--data-root", str(parent), "--profile-list"],
        capture_output=True, timeout=60,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    if result.returncode:
        raise ValueError(f"安装版读取配置档登记失败：{result.stderr.decode('utf-8', 'replace').strip()}")
    listing = json.loads(result.stdout)
    if not guard(listing["data_root"]).samefile(parent):
        raise ValueError("安装版返回了不同的数据根。")
    for entry in listing["profiles"]:
        if entry["default"]:
            continue
        directory = Path(entry["directory"])
        if directory.parent != parent / "profiles" or directory.name != entry["id"]:
            raise ValueError(f"配置档目录与登记不符：{entry['id']}")
        directory = guard(directory)
        if not directory.is_dir() or not directory.resolve().parent.samefile(parent / "profiles"):
            raise ValueError(f"配置档目录与登记不符：{entry['id']}")
        yield directory


class FileVersion(NamedTuple):
    device: int
    inode: int
    size: int
    modified_ns: int


def stamp(meta):
    # Windows 的路径和文件句柄可报告不同的 ctime。
    return FileVersion(meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns)


def register_file(root, files, path):
    meta = guard(path).stat()
    if not stat.S_ISREG(meta.st_mode):
        raise ValueError(f"配置或动作文件类型不符：{path}")
    name = path.relative_to(root).as_posix()
    key = name.casefold()
    if key in files:
        raise ValueError(f"配置含有同名文件：{name}")
    files[key] = (name, stamp(meta))
    return key


def live_files(root, directories, *, action_directories=None):
    files = {}
    for directory in directories:
        paths = [directory / name for name in CONFIG_FILES]
        paths.extend(directory / name for name in PROFILE_RECORDS)
        if directory != root:
            paths.append(directory.with_suffix(COPY_ORIGIN))
        for path in paths:
            path = guard(path, missing=True)
            if path.exists():
                register_file(root, files, path)
        state = guard(directory / "action-state", missing=True)
        if state.exists():
            for path in state.iterdir():
                if STATE_DOCUMENT.fullmatch(path.name):
                    register_file(root, files, path)
        actions = guard(directory / "actions", missing=True)
        pending = [actions] if actions.exists() else []
        while pending:
            folder = guard(pending.pop())
            if action_directories is not None:
                action_directories.add(folder.relative_to(root).as_posix())
            for path in folder.iterdir():
                if folder == actions and STAGE.fullmatch(path.name):
                    continue
                if guard(path).is_dir():
                    pending.append(path)
                else:
                    register_file(root, files, path)
    return files


def in_scope(parts, current_profiles, is_directory, root_records):
    relative = parts
    profile = ()
    while len(relative) >= 2 and relative[0].casefold() == "profiles":
        profile_id = relative[1].casefold()
        if len(relative) == 2:
            if not profile_id.endswith(COPY_ORIGIN):
                return False
            profile_id = profile_id.removesuffix(COPY_ORIGIN)
        profile += ("profiles", profile_id)
        if profile not in current_profiles and not PROFILE_ID.fullmatch(profile_id):
            return False
        relative = relative[2:]
        if not relative:
            return not is_directory
    if relative[0].casefold() == "action-state":
        if len(relative) == 1:
            if not is_directory:
                raise ValueError(f"ZIP 内的扩展状态目录类型不符：{'/'.join(parts)}")
            return False
        if not STATE_DOCUMENT.fullmatch(relative[1]):
            return False
        if is_directory or len(relative) != 2:
            raise ValueError(f"ZIP 内的扩展状态文档须为普通文件：{'/'.join(parts)}")
        return True
    if is_directory:
        return False
    if len(relative) >= 2 and relative[0].casefold() == "actions":
        return not STAGE.fullmatch(relative[1])
    name = "/".join(relative).casefold()
    return name in CONFIG_FILES or name in (PROFILE_RECORDS if profile else root_records)


def archived_files(archive, root, directories, root_records):
    current_profiles = {tuple(part.casefold() for part in folder.relative_to(root).parts)
                        for folder in directories if folder != root}
    files = {}
    for info in archive.infolist():
        name = info.filename.replace("\\", "/")
        is_directory = name.endswith("/")
        parts = name.removesuffix("/").split("/")
        if not is_directory and any(part in ("", ".", "..") or ":" in part for part in parts):
            raise ValueError(f"ZIP 内的路径无效：{name}")
        if not in_scope(parts, current_profiles, is_directory, root_records):
            continue
        key = name.casefold()
        if stat.S_IFMT(info.external_attr >> 16) not in (0, stat.S_IFREG):
            raise ValueError(f"ZIP 内的配置须为普通文件：{name}")
        if key in files:
            raise ValueError(f"ZIP 内有重复的配置路径：{name}")
        files[key] = info
    return files


def digest(stream):
    h = hashlib.sha256()
    for chunk in iter(lambda: stream.read(256 * 1024), b""):
        h.update(chunk)
    return h.digest()


def compare(root, directories, archive_path):
    live = live_files(root, directories)
    identity = catalog_identity(root)
    # 单独导出的测试档可能只在父档登记。普通数据根只核对本机实际保存的来源记录。
    root_records = PROFILE_RECORDS if "profile.json" in live or identity is not None else live.keys() & PROFILE_RECORDS
    archive_path = guard(archive_path)
    archive_version = stamp(archive_path.stat())
    with zipfile.ZipFile(archive_path) as archive:
        saved = archived_files(archive, root, directories, root_records)
        if not live and not saved and identity is None:
            raise ValueError("数据根和 ZIP 中均未找到可比对的配置文件。")
        changed = []
        identity_missing = identity is not None and "profile.json" not in saved
        if identity is not None and not identity_missing:
            with archive.open(saved.pop("profile.json")) as copy:
                if digest(copy) != hashlib.sha256(identity).digest():
                    changed.append("profile.json")
        missing = [live[k][0] for k in sorted(live.keys() - saved.keys())]
        if identity_missing:
            missing.append("profile.json")
        extra = [saved[k].filename.replace("\\", "/") for k in sorted(saved.keys() - live.keys())]
        for key in sorted(live.keys() & saved.keys()):
            name, original = live[key]
            info = saved[key]
            if info.file_size != original.size:
                changed.append(name)
                continue
            with guard(root / name).open("rb") as source, archive.open(info) as copy:
                if stamp(os.fstat(source.fileno())) != original:
                    raise ValueError(f"比对期间文件已变化：{name}")
                if digest(source) != digest(copy):
                    changed.append(name)
                if stamp(os.fstat(source.fileno())) != original:
                    raise ValueError(f"比对期间文件已变化：{name}")
    if stamp(guard(archive_path).stat()) != archive_version:
        raise ValueError("比对期间 ZIP 文件有改动，请重新运行。")
    if live != live_files(root, directories) or identity != catalog_identity(root):
        raise ValueError("比对期间配置文件有改动，请重新运行。")
    return {"matches": not (missing or extra or changed), "checked_profiles": len(directories),
            "missing_from_backup": missing, "only_in_backup": extra, "changed": changed}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True, help="安装版 omnitrigger.exe 路径")
    parser.add_argument("--data-root", type=Path, required=True, help="日常配置数据根")
    parser.add_argument("--archive", type=Path, required=True, help="以该数据根为顶层的 ZIP")
    args = parser.parse_args(argv)
    try:
        root = guard(args.data_root)
        directories = profile_dirs(args.app, root)
        result = compare(root, directories, args.archive)
        if directories != profile_dirs(args.app, root):
            raise ValueError("比对期间配置档登记有改动，请重新运行。")
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.TimeoutExpired, zipfile.BadZipFile) as error:
        print(f"无法核对配置备份：{error}", file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=not sys.stdout.isatty(), indent=2))
    return 0 if result["matches"] else 1


if __name__ == "__main__":
    sys.exit(main())
