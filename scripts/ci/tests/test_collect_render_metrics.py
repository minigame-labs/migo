import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from scripts.ci.collect_render_metrics import main, normalize_result


class CollectRenderMetricsTest(unittest.TestCase):
    def test_normalize_result_keeps_required_fields(self):
        raw = {
            "device": "pixel-4",
            "workload": "cocos-webgl-gameplay",
            "stats": {"avg_fps": 58.7, "p95_ms": 22.4},
        }
        normalized = normalize_result(raw)
        self.assertEqual(normalized["device"], "pixel-4")
        self.assertEqual(normalized["workload"], "cocos-webgl-gameplay")
        self.assertIn("avg_fps", normalized["stats"])
        self.assertIn("p95_ms", normalized["stats"])

    def test_normalize_result_preserves_release_bindings(self):
        raw = {
            "device": "pixel-4",
            "workload": "cocos-webgl-gameplay",
            "stats": {"avg_fps": 58.7, "p95_ms": 22.4},
            "_source_revision": "a" * 40,
            "_artifact_sha256": "b" * 64,
            "_profile": "full",
        }

        normalized = normalize_result(raw)

        self.assertEqual(normalized["_source_revision"], "a" * 40)
        self.assertEqual(normalized["_artifact_sha256"], "b" * 64)
        self.assertEqual(normalized["_profile"], "full")

    def test_normalize_result_converts_metrics_to_float(self):
        raw = {
            "device": "pixel-4",
            "workload": "startup-loader",
            "stats": {"avg_fps": "60", "p95_ms": 19, "first_frame_ms": "410"},
        }
        normalized = normalize_result(raw)
        self.assertEqual(
            normalized["stats"],
            {"avg_fps": 60.0, "p95_ms": 19.0, "first_frame_ms": 410.0},
        )

    def test_normalize_result_keeps_optional_first_frame_ms_when_present(self):
        raw = {
            "device": "pixel-4",
            "workload": "startup-loader",
            "stats": {"avg_fps": 60, "p95_ms": 19, "first_frame_ms": 412},
        }

        normalized = normalize_result(raw)

        self.assertEqual(normalized["stats"]["first_frame_ms"], 412.0)

    def test_main_writes_normalized_json(self):
        with TemporaryDirectory() as tmp_dir:
            src = Path(tmp_dir) / "raw.json"
            dst = Path(tmp_dir) / "normalized.json"
            src.write_text(
                """[
  {
    \"device\": \"pixel-4\",
    \"workload\": \"cocos-webgl-gameplay\",
    \"stats\": {\"avg_fps\": \"58.7\", \"p95_ms\": 22.4}
  }
]\n""",
                encoding="utf-8",
            )

            main(src, dst)

            self.assertEqual(
                dst.read_text(encoding="utf-8"),
                """[
  {
    \"device\": \"pixel-4\",
    \"workload\": \"cocos-webgl-gameplay\",
    \"stats\": {
      \"avg_fps\": 58.7,
      \"p95_ms\": 22.4
    }
  }
]\n""",
            )

    def test_main_creates_destination_parent_directory(self):
        with TemporaryDirectory() as tmp_dir:
            src = Path(tmp_dir) / "raw.json"
            dst = Path(tmp_dir) / "nested" / "results" / "normalized.json"
            src.write_text(
                """[
  {
    \"device\": \"pixel-4\",
    \"workload\": \"cocos-webgl-gameplay\",
    \"stats\": {\"avg_fps\": 58.7, \"p95_ms\": 22.4}
  }
]\n""",
                encoding="utf-8",
            )

            main(src, dst)

            self.assertTrue(dst.is_file())

    def test_normalize_result_rejects_missing_stats(self):
        with self.assertRaisesRegex(ValueError, "missing required field: stats"):
            normalize_result({
                "device": "pixel-4",
                "workload": "cocos-webgl-gameplay",
            })

    def test_normalize_result_preserves_render_optimization_metrics(self):
        raw = {
            "device": "pixel-4",
            "workload": "cocos-webgl-gameplay",
            "stats": {
                "avg_fps": 58.0,
                "p95_ms": 22.0,
                "partial_damage_frames": 420,
                "full_surface_frames": 30,
                "damage_area_k_pixels": 1500,
                "upload_frame_rejections": 5,
                "dropped_upload_recoveries": 1,
            },
        }
        result = normalize_result(raw)
        self.assertEqual(result["stats"]["partial_damage_frames"], 420.0)
        self.assertEqual(result["stats"]["full_surface_frames"], 30.0)
        self.assertEqual(result["stats"]["damage_area_k_pixels"], 1500.0)
        self.assertEqual(result["stats"]["upload_frame_rejections"], 5.0)
        self.assertEqual(result["stats"]["dropped_upload_recoveries"], 1.0)

    def test_normalize_result_omits_absent_optional_metrics(self):
        raw = {
            "device": "pixel-4",
            "workload": "cocos-webgl-gameplay",
            "stats": {"avg_fps": 58.0, "p95_ms": 22.0},
        }
        result = normalize_result(raw)
        self.assertNotIn("partial_damage_frames", result["stats"])
        self.assertNotIn("full_surface_frames", result["stats"])


if __name__ == "__main__":
    unittest.main()
