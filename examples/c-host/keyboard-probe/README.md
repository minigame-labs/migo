# Keyboard probe

Content that renders the soft-keyboard round trip as pixels.

The whole screen is one colour, and it changes only when a keyboard event
arrives, so any pixel difference is attributable to the round trip and nothing
else. The value is drawn as text as well, which proves the whole string crossed
the boundary rather than merely that some event fired.

| colour  | meaning                                            |
|---------|----------------------------------------------------|
| red     | idle, nothing requested yet                        |
| yellow  | `showKeyboard` called, the host has not answered   |
| green   | an input event arrived                             |
| blue    | confirm arrived                                    |
| magenta | complete arrived                                   |

It asks for the keyboard once on its own, at frame 30, so an automated run needs
nobody at the window. A touch re-triggers it for driving the same path by hand.

Run it against the Linux host, whose scripted "IME" types `m`, `mi`, `mig`,
`migo` and then confirms:

```sh
MIGO_CAPI_LOG=info bash scripts/dev-run-c-host.sh examples/c-host/keyboard-probe 15
```

Expect, in the host's output, `show keyboard: max_length=140 type=0 confirm=0
flags=0x0 default='seed'`, four `update keyboard` lines echoing the growing
value, and one `hide keyboard`. Expect, on screen, magenta with `value=migo`,
`height=0` and `inputs=4`.

A yellow frame means the show callback never came back: check that all three
keyboard callbacks were installed, since a subset is refused at install time
with `MIGO_ERROR_INVALID_ARGUMENT`.
