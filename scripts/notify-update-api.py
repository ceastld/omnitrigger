"""验证已公开的 OmniTrigger 发行，并通知统一更新服务。"""
import argparse
import json
import os
import re
import sys
from urllib.request import Request, urlopen

REPOSITORY = "ceastld/omnitrigger"
BASE = f"https://github.com/{REPOSITORY}/releases"


def notification(release, tag):
    if (not isinstance(tag, str) or not re.fullmatch(r"v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?", tag)
            or release.get("tag_name") != tag or release.get("draft") is not False
            or release.get("html_url") != f"{BASE}/tag/{tag}" or not release.get("published_at")):
        raise ValueError("发布通知必须指向已公开的 OmniTrigger 发行标签")
    archive = f"OmniTrigger-{tag[1:]}-windows-x86_64.zip"
    for name, limit in [(archive, 512 * 1024 * 1024), ("SHA256SUMS.txt", 64 * 1024)]:
        matches = [asset for asset in release.get("assets", []) if asset.get("name") == name]
        if len(matches) != 1:
            raise ValueError(f"发行资产缺少或重复：{name}")
        asset = matches[0]
        if (asset.get("state") != "uploaded" or not 0 < asset.get("size", 0) <= limit
                or asset.get("browser_download_url") != f"{BASE}/download/{tag}/{name}"):
            raise ValueError(f"发行资产尚未完整上传：{name}")
        if name == archive and not re.fullmatch(r"sha256:[0-9a-fA-F]{64}", asset.get("digest") or ""):
            raise ValueError("发行 ZIP 缺少 SHA-256 摘要")
    return {"event_type": "release-published", "client_payload": {"product": "omnitrigger", "repository": REPOSITORY, "tag": tag}}


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", default=os.environ.get("RELEASE_TAG"))
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    # 先验证参数，不能让标签改变请求路径。
    if not args.tag or not re.fullmatch(r"v[0-9A-Za-z.-]+", args.tag):
        raise ValueError("需要有效的 v<版本> 标签")
    headers = {"User-Agent": "OmniTrigger-Update-Notification", "Accept": "application/vnd.github+json"}
    request = Request(f"https://api.github.com/repos/{REPOSITORY}/releases/tags/{args.tag}", headers=headers)
    with urlopen(request, timeout=30) as response:
        raw = response.read(2 * 1024 * 1024 + 1)
        if len(raw) > 2 * 1024 * 1024:
            raise ValueError("发行信息超限")
        payload = notification(json.loads(raw), args.tag)
    if args.dry_run:
        print(json.dumps(payload, ensure_ascii=False))
        return
    token = os.environ.get("APP_UPDATES_DISPATCH_TOKEN")
    if not token:
        raise ValueError("请配置 APP_UPDATES_DISPATCH_TOKEN，仅需对 ceastld/app-updates 具有 Contents: write 权限")
    headers.update({"Authorization": "Bearer " + token, "Content-Type": "application/json", "X-GitHub-Api-Version": "2022-11-28"})
    request = Request("https://api.github.com/repos/ceastld/app-updates/dispatches", data=json.dumps(payload).encode(), headers=headers, method="POST")
    with urlopen(request, timeout=30) as response:
        if response.status != 204:
            raise ValueError("更新服务未接受发布通知")
    print(f"已通知统一更新服务：{args.tag}")


if __name__ == "__main__":
    main()
