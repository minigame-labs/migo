import hashlib
import json
import unittest
import zipfile
from pathlib import Path
from tempfile import TemporaryDirectory

from scripts.ci.release_evidence import (
    EvidenceError,
    create_evidence,
    verify_evidence,
)


class ReleaseEvidenceTest(unittest.TestCase):
    def setUp(self):
        self.temp = TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.artifact = self.root / "migo-test-android.aar"
        self.native = b"current release native bytes"
        with zipfile.ZipFile(self.artifact, "w") as archive:
            archive.writestr("jni/arm64-v8a/libmigo.so", self.native)

        self.revision = "a" * 40
        self.native_hash = hashlib.sha256(self.native).hexdigest()
        self.artifact_hash = hashlib.sha256(self.artifact.read_bytes()).hexdigest()
        common_binding = {
            "_source_revision": self.revision,
            "_artifact_sha256": self.artifact_hash,
            "_profile": "full",
        }
        device_binding = {
            **common_binding,
            "_installed_native_sha256": self.native_hash,
            "_device_abi": "arm64-v8a",
            "_package": "com.example.host",
        }
        documents = {
            "perf_metrics": {**device_binding, "_samples": 3, "fps": 60},
            "perf_summary": {**device_binding, "overall": "pass"},
            "power_metrics": {**device_binding, "_samples": 3, "cpu_avg_pct": 12},
            "power_summary": {**device_binding, "overall": "pass"},
            "render_summary": {**common_binding, "pass": True},
            "suite_summary": {**device_binding, "overall": "pass"},
        }
        self.reports = {}
        for kind, document in documents.items():
            path = self.root / f"{kind}.json"
            path.write_text(json.dumps(document) + "\n", encoding="utf-8")
            self.reports[kind] = path
        self.output = self.root / "release-evidence.json"

    def tearDown(self):
        self.temp.cleanup()

    def create(self):
        create_evidence(
            revision=self.revision,
            artifact=self.artifact,
            profile="full",
            device_abi="arm64-v8a",
            installed_native_sha256=self.native_hash,
            package="com.example.host",
            device_model="test-device",
            android_api=35,
            device_serial="serial-for-hashing",
            reports=self.reports,
            output=self.output,
        )

    def test_valid_evidence_round_trips(self):
        self.create()

        document = verify_evidence(
            evidence=self.output,
            revision=self.revision,
            artifact=self.artifact,
            reports_dir=self.root,
        )

        self.assertEqual(document["source_revision"], self.revision)
        self.assertTrue(document["installation"]["verified_against_artifact"])

    def test_wrong_revision_is_rejected(self):
        self.create()
        with self.assertRaisesRegex(EvidenceError, "revision"):
            verify_evidence(
                evidence=self.output,
                revision="b" * 40,
                artifact=self.artifact,
                reports_dir=self.root,
            )

    def test_installed_native_must_match_artifact_slice(self):
        with self.assertRaisesRegex(EvidenceError, "installed native"):
            create_evidence(
                revision=self.revision,
                artifact=self.artifact,
                profile="full",
                device_abi="arm64-v8a",
                installed_native_sha256="0" * 64,
                package="com.example.host",
                device_model="test-device",
                android_api=35,
                device_serial="serial-for-hashing",
                reports=self.reports,
                output=self.output,
            )

    def test_report_tampering_is_rejected(self):
        self.create()
        self.reports["perf_metrics"].write_text('{"fps": 1}\n', encoding="utf-8")

        with self.assertRaisesRegex(EvidenceError, "report hash"):
            verify_evidence(
                evidence=self.output,
                revision=self.revision,
                artifact=self.artifact,
                reports_dir=self.root,
            )

    def test_failed_gate_summary_is_rejected(self):
        document = json.loads(self.reports["power_summary"].read_text())
        document["overall"] = "fail"
        self.reports["power_summary"].write_text(json.dumps(document) + "\n")
        with self.assertRaisesRegex(EvidenceError, "power_summary"):
            self.create()

    def test_report_from_different_artifact_is_rejected(self):
        document = json.loads(self.reports["suite_summary"].read_text())
        document["_artifact_sha256"] = "0" * 64
        self.reports["suite_summary"].write_text(json.dumps(document) + "\n")

        with self.assertRaisesRegex(EvidenceError, "binding mismatch"):
            self.create()


if __name__ == "__main__":
    unittest.main()
