import hashlib
import json
import unittest
import zipfile
from pathlib import Path
from tempfile import TemporaryDirectory

from scripts.ci.android_install_receipt import InstallReceiptError, create_receipt


class AndroidInstallReceiptTest(unittest.TestCase):
    def setUp(self):
        self.temp = TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.native = b"native bytes built for this release"
        self.artifact = self.root / "migo-test-android.aar"
        with zipfile.ZipFile(self.artifact, "w") as archive:
            archive.writestr("jni/arm64-v8a/libmigo.so", self.native)
        self.base = self.root / "base.apk"
        with zipfile.ZipFile(self.base, "w") as archive:
            archive.writestr("classes.dex", b"host")
        self.split = self.root / "split_config.arm64_v8a.apk"
        with zipfile.ZipFile(self.split, "w") as archive:
            archive.writestr("lib/arm64-v8a/libmigo.so", self.native)
        self.output = self.root / "install-receipt.json"

    def tearDown(self):
        self.temp.cleanup()

    def create(self, installed_apks=None):
        return create_receipt(
            revision="a" * 40,
            artifact=self.artifact,
            package="com.example.host",
            device_abi="arm64-v8a",
            device_serial="physical-device-serial",
            installed_apks=installed_apks or [self.base, self.split],
            output=self.output,
        )

    def test_split_apk_native_is_bound_to_current_aar(self):
        document = self.create()

        self.assertTrue(document["installation"]["verified_against_artifact"])
        self.assertEqual(
            document["installation"]["installed_native_sha256"],
            hashlib.sha256(self.native).hexdigest(),
        )
        self.assertEqual(json.loads(self.output.read_text()), document)

    def test_different_installed_native_is_rejected(self):
        wrong = self.root / "wrong.apk"
        with zipfile.ZipFile(wrong, "w") as archive:
            archive.writestr("lib/arm64-v8a/libmigo.so", b"old release")

        with self.assertRaisesRegex(InstallReceiptError, "does not match"):
            self.create([wrong])

    def test_missing_native_is_rejected(self):
        with self.assertRaisesRegex(InstallReceiptError, "exactly one"):
            self.create([self.base])

    def test_duplicate_native_slices_are_rejected(self):
        duplicate = self.root / "duplicate.apk"
        with zipfile.ZipFile(duplicate, "w") as archive:
            archive.writestr("lib/arm64-v8a/libmigo.so", self.native)

        with self.assertRaisesRegex(InstallReceiptError, "exactly one"):
            self.create([self.split, duplicate])

    def test_revision_must_be_full_and_lowercase(self):
        with self.assertRaisesRegex(InstallReceiptError, "revision"):
            create_receipt(
                revision="HEAD",
                artifact=self.artifact,
                package="com.example.host",
                device_abi="arm64-v8a",
                device_serial="serial",
                installed_apks=[self.split],
                output=self.output,
            )


if __name__ == "__main__":
    unittest.main()
