# Will my mini-games run on Migo?

You do not have to send anyone your games to find out. This is the tool we would
run, and it runs on your machine.

Two scripts answer two different halves of the question, and neither answers the
other's:

| | question | needs |
|---|---|---|
| `scripts/prescreen-game.sh` | which APIs does this bundle reference, and does this build publish them? | a checkout; the first run builds a host player |
| `scripts/prescreen-run.sh` | does it actually put a frame on screen, and does anything fail? | an Android device over `adb` |

Both write Markdown. Read them, then decide whether you want to talk to us.

---

## Before you start

```
git clone https://github.com/minigame-labs/migo && cd migo
```

You need `python3` and `bash`. For the run half you also need `adb` and a phone
with USB debugging on.

A "bundle" here is a directory with `game.js` at its root — the same shape your
container already loads.

---

## 1. Which APIs does it need

```sh
bash scripts/prescreen-game.sh /path/to/your/bundle --out report.md
```

If your content is written against the mainstream mini-game global (`wx.*`
rather than `migo.*`), point it at the adapter as well, or `wx` cannot be
answered at all:

```sh
bash scripts/prescreen-game.sh /path/to/your/bundle \
    --adapter /path/to/migo-wx-adapter.bundle.js --out report.md
```

The first run builds a small Linux player and runs a probe as ordinary content,
because the runtime is the authority on what it publishes — reading the sources
would report what registration *intends*, and the two have disagreed before. That
means the first run needs the host build toolchain from [BUILD.md](BUILD.md) and
takes a few minutes. It is a one-off: dump the surface once and reuse it.

```sh
bash scripts/dump-api-surface.sh --adapter <adapter.js> --out surface.json
bash scripts/prescreen-game.sh <bundle> --surface surface.json --out report.md
```

### How to read that report

**"0 gaps" means every API name this bundle references is one we publish.** It
does not mean the bundle runs. A name existing is not a call succeeding —
`wx.login` exists on every build and still fails without a host auth handler.

There is a row called **"sites this scanner cannot resolve"**. Computed keys,
reflection over the namespace, `eval` — access no static scan can follow. That
row is why the counts above it are a **lower bound rather than a promise**. A
scanner that quietly under-reported would hand you "everything is supported",
and that is the one wrong answer that costs you real money later.

**And it now answers a question it used to leave open.** Some names this build
publishes do nothing: `reportEvent` and its siblings are, in the engine's own
words, "no-op stubs that **silently succeed**". A bundle that depends on them
looks perfect in every count above and still does not work. The report has a
bucket for exactly that — *referenced, published, but a stub* — derived from the
engine sources rather than a hand-kept list, and gated so a rename cannot make a
stub quietly disappear from it. If it says none, it means it checked.

The report also separates two things people usually conflate: names your bundle
**installs itself** (a polyfill or shim it carries — not a gap, we were never
asked for them), and namespace-shaped text that only ever appears **inside
string literals** (a storage key, an Android package name — not calls). Both are
listed rather than silently dropped, so you can see what was considered.

---

## 2. Does it actually run

```sh
bash scripts/prescreen-run.sh /path/to/your/bundle \
    --package <your.host.package> \
    --activity <the activity that puts the game on screen> \
    --device <adb serial> --secs 20 --out run-report.md
```

It copies the bundle into your host's own private directory with `run-as`
(no root, and the content never leaves the phone), launches, watches for twenty
seconds, and writes what it saw plus two screenshots.

Your host needs to be a **debuggable build** and needs an entry point that can
be started with a game id. If it cannot, the script says so instead of guessing.

### What it will and will not claim

It will not say "runs" unless all of these hold: your host held the foreground,
an engine session reached `RUNNING` **in that process**, something was painted,
and the two captures differ. Miss any one and it says what it actually saw.

That strictness is not caution for its own sake. Earlier versions of this script
reported "runs" for a bundle that had never been loaded, because a host's own
menu screen is colourful and animated and looks, in a screenshot, exactly like a
running game.

A clean result means nothing broke in twenty seconds, on one device, on one
launch. It is not a certification, and it says nothing about level two.

---

## About your content

The point of shipping you the tool is that this section is short.

- **Your bundle never leaves your machine.** The scan is local; the run copies
  into your own phone's app sandbox over your own `adb`.
- **Nothing is uploaded.** Neither script has a network path for your content.
- **We never see it** unless you decide to send us the *report*, which contains
  API names, counts and your own screenshots — not your code.

If you would rather we ran it, we can do that under an NDA with the usual terms
(single-purpose use, deletion within an agreed window, never retained for
training or calibration). But you should not need to get that far to learn
whether your catalogue runs.

---

## When the report says something is missing

Send us the report. A missing API name is usually one of three things: something
we have not implemented, something your host is expected to provide through a
handler (login, payment, ads — those are yours by design, see
[COMMERCIAL.md](COMMERCIAL.md)), or an API that platform never had either.

Which one it is changes the answer completely, and it is a five-minute
conversation once the report exists.

`licensing@minigame-labs.com`
