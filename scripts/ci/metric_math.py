#!/usr/bin/env python3
"""Unit-safe calculations used by Android device metric collection."""

from __future__ import annotations

import argparse
import math
import sys


class MetricError(ValueError):
    pass


def _positive(value: float, name: str) -> float:
    if not math.isfinite(value) or value <= 0:
        raise MetricError(f"{name} must be positive and finite")
    return value


def process_cpu_pct(*, delta_ticks: int, clock_ticks: int, elapsed_seconds: float) -> float:
    """Return process CPU as core-equivalent percent (one saturated core = 100)."""
    if delta_ticks < 0:
        raise MetricError("CPU tick delta must not be negative")
    clock_ticks = _positive(float(clock_ticks), "clock ticks")
    elapsed_seconds = _positive(float(elapsed_seconds), "elapsed seconds")
    return round(delta_ticks / clock_ticks / elapsed_seconds * 100.0, 1)


def estimate_full_charge_uah(*, charge_uah: int, level: int, scale: int) -> float:
    """Estimate full capacity from a charge counter and scaled state of charge."""
    charge_uah = _positive(float(charge_uah), "charge counter")
    scale = _positive(float(scale), "battery scale")
    if level <= 0 or level > scale:
        raise MetricError("battery level must be within the reported scale")
    return charge_uah * scale / level


def battery_drain_pct_per_hour(
    *,
    charge_start_uah: int,
    charge_end_uah: int,
    full_charge_uah: float,
    elapsed_seconds: float,
) -> float:
    """Convert a µAh counter delta to percent of full capacity per hour."""
    charge_start_uah = _positive(float(charge_start_uah), "start charge counter")
    charge_end_uah = _positive(float(charge_end_uah), "end charge counter")
    full_charge_uah = _positive(float(full_charge_uah), "full charge capacity")
    elapsed_seconds = _positive(float(elapsed_seconds), "elapsed seconds")
    used_uah = charge_start_uah - charge_end_uah
    if used_uah < 0:
        raise MetricError("charge counter increased; device was charging during measurement")
    return round(used_uah / full_charge_uah * 100.0 * 3600.0 / elapsed_seconds, 1)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    cpu = subparsers.add_parser("cpu")
    cpu.add_argument("--delta-ticks", type=int, required=True)
    cpu.add_argument("--clock-ticks", type=int, required=True)
    cpu.add_argument("--elapsed-seconds", type=float, required=True)

    capacity = subparsers.add_parser("full-charge")
    capacity.add_argument("--charge-uah", type=int, required=True)
    capacity.add_argument("--level", type=int, required=True)
    capacity.add_argument("--scale", type=int, required=True)

    battery = subparsers.add_parser("battery")
    battery.add_argument("--charge-start-uah", type=int, required=True)
    battery.add_argument("--charge-end-uah", type=int, required=True)
    battery.add_argument("--full-charge-uah", type=float, required=True)
    battery.add_argument("--elapsed-seconds", type=float, required=True)
    args = parser.parse_args(argv)

    try:
        if args.command == "cpu":
            value = process_cpu_pct(
                delta_ticks=args.delta_ticks,
                clock_ticks=args.clock_ticks,
                elapsed_seconds=args.elapsed_seconds,
            )
        elif args.command == "full-charge":
            value = estimate_full_charge_uah(
                charge_uah=args.charge_uah, level=args.level, scale=args.scale
            )
        else:
            value = battery_drain_pct_per_hour(
                charge_start_uah=args.charge_start_uah,
                charge_end_uah=args.charge_end_uah,
                full_charge_uah=args.full_charge_uah,
                elapsed_seconds=args.elapsed_seconds,
            )
    except MetricError as error:
        print(f"metric calculation failed: {error}", file=sys.stderr)
        return 2
    print(f"{value:.1f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
