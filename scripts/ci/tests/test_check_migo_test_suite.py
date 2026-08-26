import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from scripts.ci.check_migo_test_suite import validate_report_bindings, write_summary


class CheckMigoTestSuiteBindingTest(unittest.TestCase):
    def test_summary_preserves_release_bindings(self):
        bindings = {
            "_source_revision": "a" * 40,
            "_artifact_sha256": "b" * 64,
            "_installed_native_sha256": "c" * 64,
            "_device_abi": "arm64-v8a",
            "_profile": "full",
            "_package": "com.example.host",
        }
        with TemporaryDirectory() as temporary:
            output = Path(temporary) / "summary.json"
            write_summary(
                output,
                {"summary": {"total": 100, "passed": 100}},
                [],
                False,
                bindings,
            )
            document = json.loads(output.read_text())

        for field, value in bindings.items():
            self.assertEqual(document[field], value)

    def test_report_binding_mismatch_is_rejected(self):
        bindings = {"_source_revision": "a" * 40, "_profile": "full"}
        with self.assertRaisesRegex(ValueError, "_source_revision"):
            validate_report_bindings(
                {"_source_revision": "b" * 40, "_profile": "full"}, bindings
            )


if __name__ == "__main__":
    unittest.main()
