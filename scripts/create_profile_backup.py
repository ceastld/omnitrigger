"""将已保存的日常和测试配置、托管动作、编辑历史和本档脚本运行时写入新 ZIP。"""
import argparse
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import zipfile

from compare_profile_backup import digest, live_files, profile_dirs, register_file, stamp
from snapshot_action_candidates import decode, guard


SHA256 = re.compile(r"[0-9a-fA-F]{64}")
REVISION = re.compile(r"edit-([0-9]{16,20})\.json")
PACKAGE_PATH = re.compile(r"[A-Za-z0-9_.+-]+(?:/[A-Za-z0-9_.+-]+)*")
DEVICE = re.compile(r"(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\..*)?", re.IGNORECASE | re.ASCII)


def metadata(path, limit):
    path = guard(path)
    if not stat.S_ISREG(path.stat().st_mode):
        raise ValueError(f"运行时登记须为普通文件：{path}")
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"运行时登记文件超出大小限制：{path}")
    return decode(data), hashlib.sha256(data).hexdigest()


def runtime_files(directory):
    runtime = directory / "runtimes/script"
    pointer = guard(runtime / "current.json", missing=True)
    if not pointer.exists():
        program = guard(runtime / ("host.exe" if os.name == "nt" else "host"), missing=True)
        if program.exists():
            yield program, None
        return
    active, pointer_hash = metadata(pointer, 4096)
    package_id = active["archive_sha256"]
    if active["schema_version"] != 1 or not SHA256.fullmatch(package_id):
        raise ValueError(f"脚本运行时登记无效：{pointer}")
    yield pointer, pointer_hash
    package = runtime / "packages" / package_id
    manifest = package / "manifest.json"
    listing, manifest_hash = metadata(manifest, 256 * 1024)
    if manifest_hash != active["manifest_sha256"]:
        raise ValueError(f"脚本运行时清单摘要不符：{manifest}")
    yield manifest, manifest_hash
    members = listing["files"]
    if not any(row["path"] == "host.exe" for row in members):
        raise ValueError(f"脚本运行时清单缺少程序：{manifest}")
    for row in members:
        name = row["path"]
        if (not PACKAGE_PATH.fullmatch(name)
                or any(part.endswith(".") or DEVICE.fullmatch(part) for part in name.split("/"))):
            raise ValueError(f"脚本运行时清单含有无效相对路径：{name}")
        path = guard(package / name)
        if not SHA256.fullmatch(row["sha256"]) or path.stat().st_size != row["size"]:
            raise ValueError(f"脚本运行时文件与清单不符：{path}")
        yield path, row["sha256"].lower()


def inventory(root, directories):
    folders = {path.relative_to(root).as_posix() for path in directories if path != root}
    files = live_files(root, directories, action_directories=folders)
    expected = {}
    for directory in directories:
        for marker in ("legacy-upgrade-in-progress.json", "backups/external-import.pending/plan.json"):
            if guard(directory / marker, missing=True).exists():
                raise ValueError(f"配置档存在未完成的升级或导入：{directory / marker}")
        for name in ("bindings.json", "wheels.json"):
            history = guard(directory / "backups/edits" / name, missing=True)
            if history.exists():
                for path in history.iterdir():
                    match = REVISION.fullmatch(path.name)
                    if match:
                        number = int(match[1])
                        if 0 < number < 2**64 and match[1] == f"{number:016d}":
                            register_file(root, files, path)
        for path, checksum in runtime_files(directory):
            key = register_file(root, files, path)
            if checksum is not None:
                expected[key] = checksum
    if not files:
        raise ValueError(f"找不到可备份的配置：{root}")
    return files, expected, folders


def write_archive(stage, root, folders, files):
    with zipfile.ZipFile(stage, "x") as archive:
        for folder in sorted(folders):
            archive.writestr(folder + "/", b"")
        for name, version in sorted(files.values()):
            path = guard(root / name)
            with path.open("rb") as source:
                if stamp(os.fstat(source.fileno())) != version:
                    raise ValueError(f"备份期间文件已变化：{name}")
                info = zipfile.ZipInfo.from_file(path, name, strict_timestamps=False)
                info.compress_type = zipfile.ZIP_DEFLATED
                with archive.open(info, "w") as target:
                    shutil.copyfileobj(source, target, 256 * 1024)


def verify_archive(stage, root, files, expected):
    with zipfile.ZipFile(stage) as archive:
        for key, (name, version) in files.items():
            with guard(root / name).open("rb") as source, archive.open(name) as saved:
                copied = digest(saved)
                if stamp(os.fstat(source.fileno())) != version or digest(source) != copied:
                    raise ValueError(f"备份期间文件已变化：{name}")
                if key in expected and copied.hex() != expected[key]:
                    raise ValueError(f"脚本运行时文件摘要不符：{name}")


def create_backup(app, root, output):
    root = guard(root)
    output = guard(output, missing=True)
    if output.suffix.casefold() != ".zip":
        raise ValueError("备份文件名须以 .zip 结尾。")
    if output.exists():
        raise FileExistsError(f"备份已存在：{output}")
    directories = profile_dirs(app, root)
    for directory in directories:
        if any(output.is_relative_to(directory / subdir) for subdir in ("actions", "runtimes", "action-state")):
            raise ValueError("请选择动作包、运行时和扩展状态目录之外的位置保存备份。")
    files, expected, folders = inventory(root, directories)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".omnitrigger-backup-", dir=guard(output.parent)) as temp:
        stage = Path(temp) / "config.zip"
        write_archive(stage, root, folders, files)
        verify_archive(stage, root, files, expected)
        if (inventory(root, directories) != (files, expected, folders)
                or profile_dirs(app, root) != directories):
            raise ValueError("备份期间配置档有改动，请重新运行。")
        size = stage.stat().st_size
        # 同一卷内一次发布。两个导出者同时指定一个新文件名时只有一个能成功。
        guard(output, missing=True)
        if os.name == "nt":
            os.rename(stage, output)
        else:
            os.link(stage, output)
    return {"archive": str(output), "profiles": len(directories), "files": len(files), "bytes": size}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    local = os.environ.get("LOCALAPPDATA")
    parser.add_argument("--app", type=Path, required=not local,
                        default=Path(local) / "Programs/OmniTrigger/omnitrigger.exe" if local else None)
    parser.add_argument("--data-root", type=Path, required=not local,
                        default=Path(local) / "OmniTrigger" if local else None)
    parser.add_argument("--output", type=Path, help="新 ZIP 路径，默认保存到数据根的 backups 目录")
    args = parser.parse_args(argv)
    output = args.output or args.data_root / "backups" / f"OmniTrigger-config-{datetime.now():%Y%m%d-%H%M%S-%f}.zip"
    try:
        result = create_backup(args.app, args.data_root, output)
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.TimeoutExpired, zipfile.BadZipFile) as err:
        print(f"无法生成配置备份：{err}", file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=not sys.stdout.isatty(), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
