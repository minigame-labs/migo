import unittest

from scripts.ci.metric_math import (
    MetricError,
    battery_drain_pct_per_hour,
    estimate_full_charge_uah,
    process_cpu_pct,
)


class MetricMathTest(unittest.TestCase):
    def test_process_cpu_is_core_equivalent_not_device_normalized(self):
        self.assertEqual(
            process_cpu_pct(delta_ticks=200, clock_ticks=100, elapsed_seconds=2),
            100.0,
        )

    def test_estimates_full_charge_from_counter_and_scaled_level(self):
        self.assertEqual(
            estimate_full_charge_uah(charge_uah=3_000_000, level=60, scale=100),
            5_000_000.0,
        )

    def test_battery_drain_uses_charge_units(self):
        self.assertEqual(
            battery_drain_pct_per_hour(
                charge_start_uah=3_000_000,
                charge_end_uah=2_990_000,
                full_charge_uah=5_000_000,
                elapsed_seconds=3600,
            ),
            0.2,
        )

    def test_charging_device_is_rejected(self):
        with self.assertRaisesRegex(MetricError, "increased"):
            battery_drain_pct_per_hour(
                charge_start_uah=3_000_000,
                charge_end_uah=3_010_000,
                full_charge_uah=5_000_000,
                elapsed_seconds=3600,
            )

    def test_invalid_sampling_interval_is_rejected(self):
        with self.assertRaises(MetricError):
            process_cpu_pct(delta_ticks=10, clock_ticks=100, elapsed_seconds=0)


if __name__ == "__main__":
    unittest.main()
