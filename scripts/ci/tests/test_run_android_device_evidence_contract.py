import unittest
from pathlib import Path


class RunAndroidDeviceEvidenceContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.script = Path("scripts/ci/run_android_device_evidence.sh").read_text(
            encoding="utf-8"
        )

    def test_current_artifact_is_installed_and_read_back(self):
        self.assertIn('"$SUITE_DIR/install.sh"', self.script)
        self.assertIn("adb", self.script)
        self.assertIn("shell pm path", self.script)
        self.assertIn("pull", self.script)
        self.assertIn("android_install_receipt.py", self.script)

    def test_every_release_gate_is_fail_closed(self):
        for command in (
            "check_migo_test_suite.py",
            "collect_metrics.sh",
            "run_perf.sh",
            "run_power.sh",
            "run_render_matrix.sh",
            "release_evidence.py\" create",
            "release_evidence.py\" verify",
            "--fail-on-warn",
        ):
            self.assertIn(command, self.script)
        self.assertNotIn("pre-existing", self.script)
        self.assertNotIn("|| true", self.script)

    def test_real_arm64_device_and_full_profile_are_required(self):
        self.assertIn('DEVICE_ABI" == "arm64-v8a"', self.script)
        self.assertIn("exactly one authorized Android device", self.script)
        self.assertIn("--profile full", self.script)


if __name__ == "__main__":
    unittest.main()
