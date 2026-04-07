import unittest

from scripts.ci.compare_render_results import compare_result, compare_results


class CompareRenderResultsTest(unittest.TestCase):
    def test_compare_result_flags_regression(self):
        baseline = {"avg_fps": 58.0, "p95_ms": 22.0}
        current = {"avg_fps": 52.0, "p95_ms": 30.0}
        result = compare_result(baseline, current)
        self.assertFalse(result["pass"])
        self.assertIn("avg_fps", result["reasons"])

    def test_compare_result_allows_small_delta_and_missing_optional_metric(self):
        baseline = {"avg_fps": 58.0, "p95_ms": 22.0, "first_frame_ms": 400.0}
        current = {"avg_fps": 56.5, "p95_ms": 24.5}
        result = compare_result(baseline, current)
        self.assertTrue(result["pass"])
        self.assertEqual(result["reasons"], [])

    def test_compare_results_matches_normalized_row_to_baseline_entry(self):
        baseline_doc = {
            "thresholds": {
                "avg_fps_min_delta": -2.0,
                "p95_ms_max_delta": 3.0,
                "first_frame_ms_max_delta": 30.0,
            },
            "results": [
                {
                    "device": "pixel-4",
                    "workload": "cocos-webgl-gameplay",
                    "stats": {"avg_fps": 58.0, "p95_ms": 22.0},
                }
            ],
        }
        current_results = [
            {
                "device": "pixel-4",
                "workload": "cocos-webgl-gameplay",
                "stats": {"avg_fps": 52.0, "p95_ms": 30.0},
            }
        ]

        report = compare_results(current_results, baseline_doc)

        self.assertFalse(report["pass"])
        self.assertEqual(len(report["results"]), 1)
        self.assertFalse(report["results"][0]["pass"])
        self.assertNotIn("skipped", report["results"][0])
        self.assertIn("avg_fps", report["results"][0]["reasons"])

    def test_compare_results_fails_when_current_results_miss_baseline_rows(self):
        baseline_doc = {
            "thresholds": {
                "avg_fps_min_delta": -2.0,
                "p95_ms_max_delta": 3.0,
                "first_frame_ms_max_delta": 30.0,
            },
            "results": [
                {
                    "device": "pixel-4",
                    "workload": "startup-loader",
                    "stats": {"avg_fps": 60.0, "p95_ms": 20.0},
                },
                {
                    "device": "pixel-4",
                    "workload": "cocos-webgl-gameplay",
                    "stats": {"avg_fps": 58.0, "p95_ms": 22.0, "first_frame_ms": 400.0},
                },
            ],
        }
        current_results = [
            {
                "device": "pixel-4",
                "workload": "startup-loader",
                "stats": {"avg_fps": 60.0, "p95_ms": 20.0},
            }
        ]

        report = compare_results(current_results, baseline_doc)

        self.assertFalse(report["pass"])
        self.assertIn("missing baseline rows in current results", report["reasons"])
        self.assertEqual(len(report["results"]), 1)

    def test_compare_results_skips_row_without_baseline_match(self):
        baseline_doc = {
            "thresholds": {
                "avg_fps_min_delta": -2.0,
                "p95_ms_max_delta": 3.0,
                "first_frame_ms_max_delta": 30.0,
            },
            "results": [],
        }
        current_results = [
            {
                "device": "pixel-4",
                "workload": "startup-loader",
                "stats": {"avg_fps": 60.0, "p95_ms": 20.0},
            }
        ]

        report = compare_results(current_results, baseline_doc)

        self.assertFalse(report["pass"])
        self.assertTrue(report["results"][0]["skipped"])
        self.assertIn("no baseline matches found", report["reasons"])

    def test_compare_results_fails_on_malformed_current_row(self):
        baseline_doc = {
            "thresholds": {
                "avg_fps_min_delta": -2.0,
                "p95_ms_max_delta": 3.0,
                "first_frame_ms_max_delta": 30.0,
            },
            "results": [
                {
                    "device": "pixel-4",
                    "workload": "startup-loader",
                    "stats": {"avg_fps": 60.0, "p95_ms": 20.0},
                }
            ],
        }
        current_results = [
            {
                "device": "pixel-4",
                "workload": "startup-loader",
            }
        ]

        report = compare_results(current_results, baseline_doc)

        self.assertFalse(report["pass"])
        self.assertIn("malformed current row", report["reasons"])
        self.assertEqual(report["results"][0]["pass"], False)
        self.assertIn("invalid_row", report["results"][0]["reasons"])

    def test_compare_results_fails_on_duplicate_baseline_key(self):
        baseline_doc = {
            "thresholds": {
                "avg_fps_min_delta": -2.0,
                "p95_ms_max_delta": 3.0,
                "first_frame_ms_max_delta": 30.0,
            },
            "results": [
                {
                    "device": "pixel-4",
                    "workload": "startup-loader",
                    "stats": {"avg_fps": 60.0, "p95_ms": 20.0},
                },
                {
                    "device": "pixel-4",
                    "workload": "startup-loader",
                    "stats": {"avg_fps": 59.0, "p95_ms": 21.0},
                },
            ],
        }

        report = compare_results([], baseline_doc)

        self.assertFalse(report["pass"])
        self.assertIn("duplicate baseline key", report["reasons"])

    def test_compare_results_fails_on_empty_current_results(self):
        baseline_doc = {
            "thresholds": {
                "avg_fps_min_delta": -2.0,
                "p95_ms_max_delta": 3.0,
                "first_frame_ms_max_delta": 30.0,
            },
            "results": [
                {
                    "device": "pixel-4",
                    "workload": "startup-loader",
                    "stats": {"avg_fps": 60.0, "p95_ms": 20.0},
                }
            ],
        }

        report = compare_results([], baseline_doc)

        self.assertFalse(report["pass"])
        self.assertIn("empty current results", report["reasons"])
        self.assertEqual(report["results"], [])

    def test_compare_results_fails_on_non_dict_current_row(self):
        baseline_doc = {
            "thresholds": {
                "avg_fps_min_delta": -2.0,
                "p95_ms_max_delta": 3.0,
                "first_frame_ms_max_delta": 30.0,
            },
            "results": [
                {
                    "device": "pixel-4",
                    "workload": "startup-loader",
                    "stats": {"avg_fps": 60.0, "p95_ms": 20.0},
                }
            ],
        }

        report = compare_results(["bad-row"], baseline_doc)

        self.assertFalse(report["pass"])
        self.assertIn("malformed current row", report["reasons"])
        self.assertEqual(report["results"][0]["pass"], False)
        self.assertIn("invalid_row", report["results"][0]["reasons"])


    def test_compare_result_flags_full_surface_frames_regression(self):
        baseline = {"avg_fps": 60.0, "p95_ms": 16.0, "full_surface_frames": 10.0}
        current = {"avg_fps": 60.0, "p95_ms": 16.0, "full_surface_frames": 80.0}
        result = compare_result(baseline, current)
        self.assertFalse(result["pass"])
        self.assertIn("full_surface_frames", result["reasons"])

    def test_compare_result_allows_small_full_surface_delta(self):
        baseline = {"avg_fps": 60.0, "p95_ms": 16.0, "full_surface_frames": 10.0}
        current = {"avg_fps": 60.0, "p95_ms": 16.0, "full_surface_frames": 40.0}
        result = compare_result(baseline, current)
        self.assertTrue(result["pass"])

    def test_compare_result_flags_upload_rejections_regression(self):
        baseline = {"avg_fps": 60.0, "p95_ms": 16.0, "upload_frame_rejections": 5.0}
        current = {"avg_fps": 60.0, "p95_ms": 16.0, "upload_frame_rejections": 30.0}
        result = compare_result(baseline, current)
        self.assertFalse(result["pass"])
        self.assertIn("upload_frame_rejections", result["reasons"])

    def test_compare_result_skips_absent_optimization_metrics(self):
        baseline = {"avg_fps": 60.0, "p95_ms": 16.0}
        current = {"avg_fps": 60.0, "p95_ms": 16.0}
        result = compare_result(baseline, current)
        self.assertTrue(result["pass"])


    def test_compare_results_gates_full_surface_frames_end_to_end(self):
        """Full pipeline: baseline has full_surface_frames, current regresses."""
        baseline_doc = {
            "thresholds": {"full_surface_frames_max_delta": 50.0},
            "results": [{
                "device": "mid-60hz",
                "workload": "cocos-webgl-gameplay",
                "stats": {"avg_fps": 58.0, "p95_ms": 22.0, "full_surface_frames": 20.0},
            }],
        }
        current = [{
            "device": "mid-60hz",
            "workload": "cocos-webgl-gameplay",
            "stats": {"avg_fps": 58.0, "p95_ms": 22.0, "full_surface_frames": 90.0},
        }]
        report = compare_results(current, baseline_doc)
        self.assertFalse(report["pass"])
        self.assertIn("full_surface_frames", report["results"][0]["reasons"])

    def test_compare_results_gates_upload_rejections_end_to_end(self):
        """Full pipeline: baseline has upload_frame_rejections, current regresses."""
        baseline_doc = {
            "thresholds": {"upload_frame_rejections_max_delta": 20.0},
            "results": [{
                "device": "mid-60hz",
                "workload": "cocos-webgl-gameplay",
                "stats": {"avg_fps": 58.0, "p95_ms": 22.0, "upload_frame_rejections": 5.0},
            }],
        }
        current = [{
            "device": "mid-60hz",
            "workload": "cocos-webgl-gameplay",
            "stats": {"avg_fps": 58.0, "p95_ms": 22.0, "upload_frame_rejections": 30.0},
        }]
        report = compare_results(current, baseline_doc)
        self.assertFalse(report["pass"])
        self.assertIn("upload_frame_rejections", report["results"][0]["reasons"])

    def test_compare_results_fails_when_threshold_declared_but_baseline_missing(self):
        """If thresholds reference a metric but baseline rows lack it,
        the report must fail — not just warn."""
        baseline_doc = {
            "thresholds": {"full_surface_frames_max_delta": 50.0},
            "results": [{
                "device": "mid-60hz",
                "workload": "cocos-webgl-gameplay",
                "stats": {"avg_fps": 58.0, "p95_ms": 22.0},
                # Note: full_surface_frames is MISSING from stats
            }],
        }
        current = [{
            "device": "mid-60hz",
            "workload": "cocos-webgl-gameplay",
            "stats": {"avg_fps": 58.0, "p95_ms": 22.0, "full_surface_frames": 100.0},
        }]
        report = compare_results(current, baseline_doc)
        self.assertFalse(report["pass"], "must fail when threshold has no baseline coverage")
        self.assertIn("uncovered_thresholds", report)
        self.assertIn("full_surface_frames", report["uncovered_thresholds"])
        self.assertIn("uncovered thresholds", report["reasons"])

    def test_compare_results_passes_when_all_thresholds_covered(self):
        """No uncovered_thresholds when baseline rows have all threshold metrics.
        DEFAULT_THRESHOLDS declares: avg_fps, p95_ms, first_frame_ms,
        full_surface_frames, upload_frame_rejections — all must be in baseline."""
        all_stats = {
            "avg_fps": 58.0,
            "p95_ms": 22.0,
            "first_frame_ms": 430.0,
            "full_surface_frames": 50.0,
            "upload_frame_rejections": 5.0,
        }
        baseline_doc = {
            "results": [{
                "device": "mid-60hz",
                "workload": "cocos-webgl-gameplay",
                "stats": dict(all_stats),
            }],
        }
        current = [{
            "device": "mid-60hz",
            "workload": "cocos-webgl-gameplay",
            "stats": dict(all_stats),
        }]
        report = compare_results(current, baseline_doc)
        self.assertTrue(report["pass"])
        self.assertNotIn("uncovered_thresholds", report)

    def test_main_exits_nonzero_on_uncovered_thresholds(self):
        """CLI must exit non-zero when uncovered thresholds exist."""
        import json
        import tempfile
        from pathlib import Path
        from scripts.ci.compare_render_results import main

        with tempfile.TemporaryDirectory() as tmp:
            baseline_path = Path(tmp) / "baseline.json"
            current_path = Path(tmp) / "current.json"
            baseline_path.write_text(json.dumps({
                "thresholds": {"full_surface_frames_max_delta": 50.0},
                "results": [{
                    "device": "d", "workload": "w",
                    "stats": {"avg_fps": 60.0, "p95_ms": 16.0},
                }],
            }))
            current_path.write_text(json.dumps([{
                "device": "d", "workload": "w",
                "stats": {"avg_fps": 60.0, "p95_ms": 16.0},
            }]))
            exit_code = main([
                "--current", str(current_path),
                "--baseline", str(baseline_path),
            ])
            self.assertEqual(exit_code, 1)


if __name__ == "__main__":
    unittest.main()
