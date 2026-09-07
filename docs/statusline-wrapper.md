# `nazar-statusline` — the status-line wrapper

The second of the two readers, and the only part of nazar-tray that writes a file
belonging to another program. It is a separate binary, shipped in the nazar-tray bundle,
and it is **owned by this repository** — [Nazar](https://github.com/xfurqan0/nazar) reads
its output rather than growing a second implementation of it.

> One owner, one contract, two consumers. What the two projects share is the file format,
> not the code: making the wrapper an npm package would put Node in front of nazar-tray's
> "one binary, no prerequisites" claim, and Node's start-up alone would spend the whole
> time budget.

```text
 Claude Code ──payload──▶ nazar-statusline ──▶ ~/.nazar/statusline/<session_id>.json
                                │                          │
                                │                          ├──▶ nazar-tray  (this repo)
                                │                          └──▶ Nazar       (the canvas)
                                └──same payload──▶ the status line you already had
```

## What it does, in order

1. Reads the JSON payload Claude Code writes to its standard input.
2. Writes it **whole**, atomically, to `~/.nazar/statusline/<session_id>.json`.
3. Runs the status-line command that was configured before it was installed, with the
   same bytes on standard input, forwarding its standard output, its standard error and
   its exit code.
4. If there was no status line before, prints a minimal one of its own:
   `Fable 5.1 · high · 5h 12% · 7d 31%` — model, effort, and whichever quota windows the
   payload carried.

Measured on the maintainer's machine, release build, ten runs, no chained command:
**median 9.6 ms**, min 9.2, max 12.9 — against a budget of 50 ms. The chained command's
own time is its own; the wrapper adds a file write and a process start to it.

## Commands

```
nazar-statusline                       the wrapper itself (this is what Claude Code runs)
nazar-statusline install [--dry-run] [--config-dir <path>]
nazar-statusline uninstall [--dry-run] [--config-dir <path>]
nazar-statusline status [--config-dir <path>]
nazar-statusline --help | --version
```

`--config-dir` wins, then `CLAUDE_CONFIG_DIR`, then `~/.claude`. `NAZAR_HOME` moves
`~/.nazar`, which is how the whole test suite runs without touching the machine it is on.

## Where the files are

| Path | What it is |
|---|---|
| `~/.nazar/statusline/<session_id>.json` | One capture per Claude Code session. Overwritten on every refresh, removed after seven days of silence. |
| `~/.nazar/statusline/chain.json` | The `statusLine` object that was in `settings.json` before the install, copied verbatim, plus where the settings file and its backup are. |
| `<config dir>/settings.json.nazar-bak-<stamp>` | An untouched copy of the settings file, taken before the edit. **Never written over**, and left in place by `uninstall`. |

**Keyed by session id, never one fixed path.** Three concurrent Claude Code sessions each
refresh their own status line; a wrapper that wrote one file would have them overwrite
each other, which is the mistake the data-layer audit found in the prototype this replaces.
A session id that is not a plain identifier — anything with a path separator in it, say —
is not used as a file name; such a payload goes to `unknown.json`.

## What a capture contains, and why it lives where it does

The capture is the payload, whole, inside a four-field envelope:

```json
{
  "schemaVersion": 1,
  "updatedAt": "2026-09-07T07:57:12Z",
  "wrapper": "nazar-statusline/0.1.0",
  "sessionId": "…",
  "payload": { …exactly the bytes Claude Code sent… }
}
```

The whole payload rather than a selection of it, because the alternative is for the
wrapper to decide on every refresh which of Claude Code's fields its two consumers will
ever want, and to be wrong the first time one of them wants another. `updatedAt` is when
the wrapper captured it, in UTC.

**There is no token and no account identifier in a status-line payload** — checked field
by field against the live payload, and a test in `tests/hygiene.rs` fails the build if a
fixture ever grows one. There *are* paths: `cwd`, `transcript_path`, `scratchpad_dir`, the
workspace directories and the repository name. That is why:

- captures live under `~/.nazar`, inside the home directory, with its permissions;
- **nothing is ever uploaded**, by this program or by nazar-tray — there is no network
  code in either;
- `limits.json`, the file that *is* meant to be safe to paste into a bug report, gets four
  numbers out of a capture and nothing else. Cost, context usage, model name and paths
  stay in the capture file for Nazar, and never reach `limits.json`;
- a capture is deleted seven days after its session stopped writing to it.

## What the install does to `settings.json`

Only `statusLine`. Every other key, and the order of all of them, survives:

```diff
--- a/settings.json
+++ b/settings.json
@@ -12,7 +12,7 @@
   "defaultShell": "powershell",
   "statusLine": {
     "type": "command",
-    "command": "node \"…/statusline.js\"",
+    "command": "…/nazar-statusline.exe",
     "padding": 0,
     "refreshInterval": 30
   },
```

`padding`, `refreshInterval` and any other key the existing `statusLine` carried are kept:
they configure the status line, not the command. The path written is absolute and quoted
if it contains a space, because Claude Code hands the command to a shell.

The install **refuses**, changing nothing, when:

- the settings file is not valid JSON — it is reported, never "repaired", and never
  replaced by `{}`;
- the top level is not a JSON object;
- a `settings.json.lock`-like file is in the directory;
- it is already installed (it says so and stops — running it twice is safe).

The order it writes in is deliberate: backup, then `chain.json`, then `settings.json`. A
crash between the second and the third leaves a record of a status line that was never
replaced, which is inert. The other order would leave the wrapper installed with no record
of what it displaced, which is the one outcome the files on disk could not undo.

## Uninstalling

```
nazar-statusline uninstall
```

restores the exact `statusLine` object from `chain.json` — or removes the key, if there
was none — checks the result against the backup taken at install time, and leaves the
backup where it is. If the record and the backup disagree, it stops and says so rather
than writing either.

**By hand, if the binary is gone.** Open `~/.nazar/statusline/chain.json`, copy the
`previous` object, and paste it over `statusLine` in `settings.json`. If `previous` is
absent, delete the `statusLine` key. The backup beside the settings file
(`settings.json.nazar-bak-…`) is the same thing in whole-file form.

## Known limits

- **Claude numbers only move while a session refreshes its status line.** Between
  sessions the tray shows the last value with its age and a locally computed countdown.
  Quota does not burn while you are not using it, so this is honest rather than stale.
- **`rate_limits` is not always there.** It is present for Pro and Max subscribers, and
  only after a session's first API response; Claude Code also drops a window once its
  `resets_at` has passed. Each of those produces a window with no percentage and a
  `state` of `error` in `limits.json` — never a reassuring `0 %`.
- **The payload has no plan name.** `model.id` says which model and `version` says which
  build; neither says which subscription, and guessing one from the other would be an
  invented number. `providers.claude.plan` is therefore absent on the passive path. The
  opt-in detailed-windows mode (WP2b) is where a real plan name comes from.
- **The payload has no model-scoped weekly windows.** Verified live: the status line
  reported a global weekly of 18 % on a machine whose Fable-only weekly was 23 %. Those
  windows come only from the official usage endpoint, behind the opt-in mode.
- **A status line goes quiet when a session is idle.** Claude Code's own answer is
  `refreshInterval`, which the install preserves if you had one.
