"""The publishing entry point must reject invalid requests before calling R2."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


PUBLISHER = Path(__file__).resolve().parents[1] / "publish-native-update.sh"


class NativePublishGateTests(unittest.TestCase):
    def test_dry_run_and_branch_requests_make_no_network_calls(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            called = root / "network-called"
            for tool in ("aws", "curl"):
                stub = root / tool
                stub.write_text('#!/bin/sh\ntouch "$FIXTURE_CALLED"\nexit 9\n')
                stub.chmod(0o700)
            (root / "evidence").mkdir()
            (root / "evidence" / "verification.json").write_text('{"mode":"publish"}')
            for name in ("Powermove-Native-2.0.0-mac-arm64.dmg",
                         "Powermove-Native-2.0.0-mac-arm64.zip", "appcast.xml", "policy.json"):
                (root / name).write_text("synthetic release data")
            env = {
                "PATH": f"{root}:{os.environ['PATH']}",
                "FIXTURE_CALLED": str(called),
                "PM_NATIVE_STAGE": scratch,
                "PM_NATIVE_VERSION": "2.0.0",
                "AWS_ACCESS_KEY_ID": "fixture",
                "AWS_SECRET_ACCESS_KEY": "fixture",
                "AWS_DEFAULT_REGION": "auto",
                "GITHUB_EVENT_NAME": "workflow_dispatch",
            }
            for publish, ref in (("false", "refs/tags/native-v2.0.0"),
                                 ("true", "refs/heads/chike/native-release-dry"),
                                 ("true", "refs/tags/native-v2.0.1")):
                with self.subTest(publish=publish, ref=ref):
                    result = subprocess.run(
                        ["/bin/bash", PUBLISHER],
                        env={**env, "PM_NATIVE_PUBLISH": publish, "GITHUB_REF": ref},
                        capture_output=True,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(called.exists())
