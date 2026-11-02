import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("notification", Path(__file__).with_name("notify-update-api.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def release():
    tag = "v0.2.0"
    return {"tag_name": tag, "draft": False, "prerelease": True, "published_at": "2026-11-02T00:00:00Z",
            "html_url": f"{module.BASE}/tag/{tag}", "assets": [
                {"name": name, "size": 100, "state": "uploaded", "digest": "sha256:" + "a" * 64,
                 "browser_download_url": f"{module.BASE}/download/{tag}/{name}"}
                for name in ("OmniTrigger-0.2.0-windows-x86_64.zip", "SHA256SUMS.txt")]}


class NotificationTests(unittest.TestCase):
    def test_existing_public_preview_can_notify_expected_repository(self):
        result = module.notification(release(), "v0.2.0")
        self.assertEqual(result["client_payload"], {"product": "omnitrigger", "repository": "ceastld/omnitrigger", "tag": "v0.2.0"})

    def test_drafts_incomplete_assets_and_other_repositories_cannot_notify(self):
        cases = []
        for key, value in [("draft", True), ("tag_name", "v0.3.0"), ("html_url", "https://example.test")]:
            bad = release()
            bad[key] = value
            cases.append(bad)
        for key, value in [("state", "new"), ("size", 0), ("digest", None), ("browser_download_url", "https://example.test/app.zip")]:
            bad = release()
            bad["assets"][0][key] = value
            cases.append(bad)
        bad = release()
        bad["assets"].pop()
        cases.append(bad)
        for item in cases:
            with self.subTest(item=item), self.assertRaises(ValueError):
                module.notification(copy.deepcopy(item), "v0.2.0")


if __name__ == "__main__":
    unittest.main()
