import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from scripts.ci.compare_baseline import compare, write_summary


class CompareBaselineRequiredMetricsTest(unittest.TestCase):
    def test_missing_required_metric_fails(self):
        results, failed, warned = compare(
            {},
            {"metrics": {"fps": {"required": True, "direction": "higher_is_better"}}},
        )

        self.assertTrue(failed)
        self.assertFalse(warned)
        self.assertEqual(results[0]["status"], "fail")

    def test_missing_optional_metric_is_skipped(self):
        results, failed, warned = compare(
            {},
            {"metrics": {"diagnostic": {"required": False}}},
        )

        self.assertFalse(failed)
        self.assertFalse(warned)
        self.assertEqual(results[0]["status"], "skipped")

    def test_non_numeric_required_metric_fails(self):
        results, failed, _ = compare(
            {"fps": "unknown"},
            {"metrics": {"fps": {"required": True, "direction": "higher_is_better"}}},
        )

        self.assertTrue(failed)
        self.assertEqual(results[0]["status"], "fail")

    def test_unknown_direction_fails_instead_of_passing(self):
        results, failed, _ = compare(
            {"fps": 60},
            {"metrics": {"fps": {"required": True, "direction": "sideways"}}},
        )

        self.assertTrue(failed)
        self.assertEqual(results[0]["status"], "fail")

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
                str(output), [], False, False, "baseline.json", "current.json", bindings
            )

            document = json.loads(output.read_text())

        for field, value in bindings.items():
            self.assertEqual(document[field], value)


if __name__ == "__main__":
    unittest.main()
