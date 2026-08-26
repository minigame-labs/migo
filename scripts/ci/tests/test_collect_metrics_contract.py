import unittest
from pathlib import Path


class CollectMetricsContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.script = Path("scripts/ci/collect_metrics.sh").read_text(encoding="utf-8")

    def test_package_is_required_instead_of_guessed(self):
        self.assertIn("--package is required", self.script)
        self.assertNotIn("Auto-detect package", self.script)

    def test_cpu_uses_unit_tested_core_equivalent_math(self):
        self.assertIn('metric_math.py" cpu', self.script)
        self.assertNotIn("get_cpu_count", self.script)

    def test_battery_uses_charge_counter(self):
        self.assertIn("charge counter", self.script.lower())
        self.assertIn('metric_math.py" battery', self.script)

    def test_process_exit_fails_collection(self):
        self.assertIn("process exited during measurement", self.script)
        self.assertNotIn("skipping sample", self.script)

    def test_runtime_and_artifact_bindings_are_required(self):
        for option in (
            "--runtime-metrics",
            "--artifact",
            "--source-revision",
            "--artifact-sha256",
            "--installed-native-sha256",
            "--device-abi",
            "--profile",
        ):
            self.assertIn(option, self.script)


if __name__ == "__main__":
    unittest.main()
