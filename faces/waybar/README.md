# nazar-tray in Waybar

Your binding quota window, as a number in the bar.

```
nazar 70%
```

Hover it for every window of both providers, when each resets and how old the reading is.
Amber at 60 %, red at 85 %, dimmed when nazar-tray is not running, `?` when nothing could be
read — never a reassuring `0 %`.

## What this is

Three files and nothing else: a POSIX shell script, an example module and an example
stylesheet. The script reads `~/.nazar/limits.json`, the file nazar-tray already writes, and
prints one line of JSON for Waybar. It is **60 lines**, and the reason it can be that short is
that it has nothing to work out: the contract in
[`docs/limits-contract.md`](../../docs/limits-contract.md) has already decided what a window
is, what "unknown" means and which of them binds.

It does not run `nazar-tray`, or any other program but `jq`. It does not open a socket. It
does not write a file — not a cache, not a "last good value", not a lock. **`~/.nazar` has one
writer and this is not it**, which is the whole design: one process measures, any number of
faces read.

## Install

```sh
mkdir -p ~/.config/waybar/scripts
cp nazar-waybar.sh ~/.config/waybar/scripts/
chmod +x ~/.config/waybar/scripts/nazar-waybar.sh
```

Then copy the `custom/nazar` block out of [`config.jsonc`](config.jsonc) into your own
`~/.config/waybar/config.jsonc`, add `"custom/nazar"` to one of the `modules-*` lists, and
take what you want from [`style.css`](style.css). Reload Waybar.

Needs `jq`, and nazar-tray running somewhere on the same machine. On a desktop with no tray
host — a stock GNOME, most often — nazar-tray runs as the engine and writes `limits.json` all
the same; `nazar-tray --headless` asks for that explicitly, which is what you want beside a
bar module you would rather read than a second icon.

## The one thing to get wrong

**Never put `{percentage}` in `format`.**

Waybar's `percentage` field has to be a number — it is what indexes an *array* of
`format-icons` — so a script that knows nothing still has to put something there, and the only
something available is `0`. A bar reading `nazar 0%` tells a user about to start a long task
that they have a full week of quota in hand, which is the exact opposite of the truth and the
one lie this whole contract is written to prevent (rule 2: *"I do not know" and "you have used
nothing" are opposite messages*).

So the module prints the truth in two places that cannot be a number: `text` says `?` and
`class` says `unknown`. Use `{}` or `{text}` in `format`, and if you want an icon, key it off
`alt` rather than off the percentage:

```jsonc
"format": "{icon} {}",
"format-icons": { "ok": "", "warn": "", "crit": "", "stale": "", "unknown": "" }
```

An **object** `format-icons` is indexed by `alt`, which the script sets to the same five names
as the class. An **array** is indexed by `percentage`, and walks into the same trap.

## What it prints

| Field | Value |
|---|---|
| `text` | `nazar 70%`, the binding window rounded **down** — 99.6 % is `99%`, because a bar that says a window is spent when it is not is wrong at the moment it matters most. `nazar ?` when no window carries a number. |
| `alt`, `class` | One of `ok`, `warn`, `crit`, `stale`, `unknown`. |
| `tooltip` | One line per window — `codex secondary 70 % · resets in 4 d 2 h` — plus how old each provider's reading is, `\r` between them. A window that could not be read is `?`. When nazar-tray is not running, the first line says so. |
| `percentage` | The same number as `text`, or `0` when there is none. **For `format-icons` only**; see above. |

The percentage in the bar is the **highest** of both providers' binding windows, because the
one that binds you is the one closest to full. `binding` is a summary nazar-tray writes into
the file; where it names a window that carries no number, the script recomputes it from the
windows themselves, which is what the contract tells a consumer that disagrees to do.

## When the tray is not running

`class` becomes `stale` and the tooltip says `nazar-tray is not running`. The last numbers
stay in the bar, dimmed — they were true when they were written, and quota does not burn while
nothing is using it.

That answer comes from `~/.nazar/limits.lock`, in the order the contract gives: a `pid` the
kernel has never heard of means the writer is gone **now**, and only a pid that cannot be asked
about falls back to the heartbeat's five minutes. Neither is guessed from the age of
`limits.json` itself, which the tray rewrites only when the numbers move — a week-old
`limits.json` beside a fresh heartbeat is a quiet week, not a broken tray.

## Refreshing on demand

`interval` is a poll. To push instead:

```sh
nazar-tray --print --write && pkill -RTMIN+8 waybar
```

`--print --write` takes the same advisory lock as the tray and defers to it if it is already
running, so this is safe to bind to a key. `signal: 8` in the module is what makes
`RTMIN+8` reach it.

## If the module does not appear

Waybar hides a custom module whose script prints nothing or prints something it cannot parse,
which is the worst possible failure shape — so this script has no path that ends in silence.
Every one of them ends in a line: a missing `limits.json`, a damaged one, a missing `jq`, no
home directory at all. Run it by hand to see which:

```sh
sh ~/.config/waybar/scripts/nazar-waybar.sh
```

`NAZAR_HOME` moves `~/.nazar` somewhere else, for both nazar-tray and this script, which is
also how the tests point it at a throwaway directory.

## Tested

`ui/test/waybar.test.mjs`, in the same CI run as everything else, against
[`fixtures/limits.sample.json`](../../fixtures/limits.sample.json) — the repository's own
sample document — and variants derived from it: 99.6 %, a provider whose windows are all
`error`, a provider that is not configured, a damaged file, no file at all, and the three ways
the lock can say the tray is gone. Deriving the variants rather than committing them is what
makes contract drift impossible: a change to the sample that this script mishandles fails the
build.

## What this is not

It is not a popover, a settings page or a second copy of the panel. Waybar has a `tray` module
that hosts StatusNotifier icons, and where that works nazar-tray's own bead appears in it with
its panel a click away; this module goes **beside** that, not instead of it, and gives you the
one thing a tray icon cannot: the number, in the bar, without hovering.
