"""Check that Apple responses yield only allowlisted public evidence."""
import importlib.util
import json
from pathlib import Path
from subprocess import CompletedProcess
import sys
import tempfile
import unittest
from unittest.mock import patch

PACKAGING = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("verify_native_release", PACKAGING / "verify-native-release.py")
verify = importlib.util.module_from_spec(spec)
with patch.object(sys, "path", [str(PACKAGING), *sys.path]):
    spec.loader.exec_module(verify)

REQUEST = "7e4fb8d0-3457-4c84-a7be-66dc68352fa8"


def result(payload, status=0, stderr=b""):
    return CompletedProcess([], status, json.dumps(payload).encode(), stderr)


class NotarizationTests(unittest.TestCase):
    def test_transport_failure_preserves_only_numeric_status(self):
        error = result({}, 1, b"HTTP status code: 401 for /private/fixture/notary.p8?key=fixture")
        self.assertEqual(verify.notary_failure(error), "HTTP 401")

    def test_accepted_proof_requires_both_responses_to_accept(self):
        accepted = result({"id": REQUEST, "status": "Accepted"})
        unknown = result({"id": REQUEST, "status": "In Progress"})
        log = result({"jobId": REQUEST, "status": "Accepted", "statusCode": 0, "issues": None})
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory) / "notarization.json"
            with patch.object(verify, "capture", side_effect=[accepted, log, unknown, log]):
                verify.notarize("app", "app.zip", [], evidence)
                with self.assertRaisesRegex(ValueError, "incomplete acceptance or log"):
                    verify.notarize("dmg", "app.dmg", [], evidence)
            self.assertEqual(json.loads(evidence.read_text())["app"],
                             {"id": REQUEST, "status": "Accepted", "logRetrieved": True, "issues": []})

    def test_rejected_proof_omits_paths_and_messages(self):
        rejected = result({"id": REQUEST, "status": "Invalid"})
        log = result({"jobId": REQUEST, "status": "Invalid", "issues": [{
            "message": "Missing secure timestamp at /Users/fixture-user",
            "path": "/private/fixture",
            "severity": "error",
            "docUrl": "https://example.invalid?signature=fixture",
        }]})
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory) / "notarization.json"
            with patch.object(verify, "capture", side_effect=[rejected, log]):
                with self.assertRaisesRegex(ValueError, "incomplete acceptance or log"):
                    verify.notarize("app", "app.zip", [], evidence)
            self.assertEqual(json.loads(evidence.read_text()), {"app": {
                "id": REQUEST,
                "status": "Invalid",
                "logRetrieved": True,
                "issues": [{"category": "missing secure timestamp", "severity": "error"}],
            }})
