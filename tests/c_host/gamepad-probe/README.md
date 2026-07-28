# Gamepad probe

Content that renders what `getGamepads()` reports, as pixels.

The JS Gamepad implementation is state held in JS and driven from native, so it
has no unit tests — this content is where it is actually exercised.

| colour  | meaning                                  |
|---------|------------------------------------------|
| red     | no pad has connected                     |
| yellow  | connected, no sample yet                 |
| green   | axes and buttons arrived                 |
| blue    | the pad was withdrawn                    |

The Linux host plays a scripted pad: it announces a standard 4-axis, 17-button
pad at 1s, sweeps axis 0 (and axis 1 as its negation) while holding button 0 and
resting button 6 at quarter travel, then withdraws it at 6s.

```sh
MIGO_CAPI_LOG=info bash scripts/dev-run-c-host.sh tests/c_host/gamepad-probe 12
```

Two things on screen are load-bearing rather than decorative:

- `ax0` and `ax1` move in opposite directions, so an axis landing in the wrong
  slot shows up as two values that agree instead of two that mirror.
- `b0=down` with `b6=0.25` and *not* pressed is the case that proves `pressed`
  is carried across the boundary rather than derived from `value`. A runtime
  that derived it would report b6 as pressed or b0 as up, depending on the
  threshold it guessed.

The connect log line reports `axes=4 buttons=17` read inside the
`gamepadconnected` listener, which is where content decides its layout — those
lengths have to be right before the first sample arrives, not after.
