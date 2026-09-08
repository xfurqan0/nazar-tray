# `limits.json` — the contract

The one file nazar-tray writes for other programs to read. Frozen at `schemaVersion: 1`.
Nazar's quota strip reads this file and nothing else from nazar-tray.

- **Location:** `~/.nazar/limits.json` (`%USERPROFILE%\.nazar\limits.json` on Windows).
- **The data directory and the settings directory are two different directories, and that
  is the contract rather than an accident.** Everything a consumer reads lives under
  `~/.nazar` — `limits.json`, `limits.lock`, `tray.request` and `statusline/` — while the
  files that are the *user's* rather than a consumer's live in the platform's own settings
  location, `%APPDATA%\nazar` on Windows and `$XDG_CONFIG_HOME/nazar` or `~/.config/nazar`
  elsewhere: `config.json`, which the user edits, and `alerts.json`, which only nazar-tray
  writes. `NAZAR_HOME` overrides **both**, which is what makes a whole installation
  pointable at a throwaway directory. So a consumer that resolves `~/.nazar` for data and
  never opens `%APPDATA%` is reading exactly the right places, and two resolvers that
  disagree about Windows are not in disagreement if one of them is resolving settings.
  (`crates/nazar-core/src/paths.rs`, `data_dir` against `settings_dir`.)
- **Written by:** the nazar-tray process, and only it. One writer, many readers.
- **Sample:** [`fixtures/limits.sample.json`](../fixtures/limits.sample.json). Consumers
  are expected to copy that file into their own test suite rather than hand-write one.
- **Implemented by:** `crates/nazar-core/src/limits.rs`. The tests in that file are the
  executable half of this document; if the two ever disagree, the tests are right.

> The plan's section 1.2 refers to this document as `LIMITS-CONTRACT.md`. It is
> `docs/limits-contract.md`; the contents are what was described there.

## Six rules that shape the format

1. **No credentials, ever.** No tokens, no account identifiers, no e-mail addresses, no
   session ids. The file is safe to paste into a bug report. If a future field cannot
   meet that bar, it does not go in this file.
2. **No invented numbers.** `percent` is **optional**. A window that could not be read
   omits it entirely and carries `"state": "error"`. A consumer renders such a window as
   *unknown* — grey, a question mark, the word — and never as `0 %`. "I do not know" and
   "you have used nothing" are opposite messages to someone about to start a long task.
3. **Atomic writes.** The writer builds a temporary file in the same directory and
   renames it over the target. A reader sees the previous document or the new one, never
   a prefix of either. A write that dies half way leaves no partial file and no leftover
   temporary.
4. **Unknown fields survive.** A reader keeps fields it does not understand and writes
   them back unchanged, and an unrecognised `state` or `source` value is preserved as
   written. An older tray next to a newer file loses nothing.
5. **Rounding happens at display time.** `percent` is stored as reported. A consumer that
   wants `18 %` rounds it itself — **downwards**. 99.6 % is not 100 %, and a display that
   says a window is spent when it is not is wrong at the exact moment it matters most.
6. **Times are UTC.** Every timestamp in this file is RFC 3339 with a `Z`, and a consumer
   renders local time itself. See the section at the end for why.
7. **Derived fields are computed by consumers; the file stores only measured values.**
   Which window binds, how long until it resets, how old the reading is and how alarming it
   is all change with the clock rather than with the data, so none of them is stored as a
   fact about a moment that has passed. See the next section.

## Derived fields are computed by consumers

The file stores **only measured values**: what a source reported, and when it reported it.
Everything a display actually wants is worked out from those two things plus the current
time, and worked out again a second later, which is why none of it is written down:

| Derived | From | Rule |
|---|---|---|
| `binding` | the windows' `percent` | The highest percentage. Ties go to the shorter `windowMinutes`, then to the smaller key. A window with **no** `percent` never binds; a provider whose windows are all unknown has no binding window at all. |
| remaining | `resetsAt` − now | Milliseconds, and **negative when the reset is already due** — which is what a reading taken before a long sleep looks like. |
| age | now − `sourceAt` | Milliseconds. Negative when the source's clock is ahead of the consumer's, which is not an error. |
| freshness | age | `fresh` ≤ 5 min, `aging` ≤ 45 min, `stale` beyond that, `unknown` with no `sourceAt`. |
| severity | `percent` | `ok` below 60, `warn` at 60, `critical` at 85, `exhausted` at 100, `unknown` with no `percent`. |

The thresholds in the last two rows are the shipped defaults and live in the user's
settings (`%APPDATA%\nazar\config.json`, keys `freshness` and `thresholds`), so that every
display answers the same question the same way. The retired prototype had three displays
with three different definitions of "stale" — 45 minutes in the tray, 30 in the status line
and none at all in the fetcher — and a user watching two of them at once could not tell
which to believe.

`binding` **is** written into the file, because it is a summary a consumer should not have
to recompute to render one row. It is the one exception, it is computed by the writer from
the same rule, and a consumer that recomputes it must get the same answer. A consumer that
disagrees with the file should trust its own arithmetic: the file may have been written by
an older build or by hand.

nazar-tray's own implementation is `crates/nazar-core/src/state.rs`, and its property tests
are the executable form of the table above.

## Document

| Field | Type | Required | Notes |
|---|---|---|---|
| `schemaVersion` | integer | yes | `1`. A bump is a breaking change for Nazar. |
| `updatedAt` | string | yes | RFC 3339 **in UTC** (`…Z`). When the tray last wrote the file — which is **when the content last changed**, not when the tray last looked. The writer compares the document it is about to write with the one it wrote last, ignoring this field, and writes nothing when they are the same. A consumer that wants to know whether the *tray* is alive reads the heartbeat in `limits.lock` instead; the two questions are different. See "One writer" below. |
| `providers` | object | yes | See below. |

### `providers.<name>`

`claude` and `codex` in v1. A provider whose files are absent on this machine is present
with `"configured": false` and no windows — the key never disappears.

| Field | Type | Required | Notes |
|---|---|---|---|
| `configured` | boolean | yes | `false` when the provider's files are not on this machine. |
| `plan` | string | no | `max_20x`, `plus`, … Codex's comes through as the rollout log reported it. Claude's is normalised from the usage endpoint's tier (`default_claude_max_20x` → `max_20x`) and exists only in detailed mode; the passive path derives none. |
| `source` | string | no | `statusline` \| `endpoint` for Claude, `rollout` for Codex. `endpoint` whenever the opt-in mode produced the block, even if a newer status-line capture replaced some of its windows — see "Which source wins" below. |
| `sourceAt` | string | no | RFC 3339. When the *source* produced the numbers, which is older than `updatedAt` by design. |
| `binding` | string | no | Key of the window with the **highest percentage** among this provider's windows. Never taken from a flag the source does not provide. |
| `windows` | object | no | Window key to window object. Absent or empty when nothing is known. |

Window keys are provider-specific and open-ended:

- Claude: `five_hour`, `seven_day`, and model-scoped weeklies named
  `seven_day_<model>` (`seven_day_fable`), which appear only in detailed mode.
- Codex: `primary` (5 hours) and `secondary` (7 days), named after the fields in the
  rollout log.

A consumer must not hard-code the set. Iterate the object; use `windowMinutes` when it
needs to know how long a window is.

### `providers.<name>.windows.<key>`

| Field | Type | Required | Notes |
|---|---|---|---|
| `percent` | number | **no** | 0–100, unrounded. **Absent when the value is unknown.** See rule 2. |
| `resetsAt` | string | no | RFC 3339, as the source reported it. Countdowns are computed locally in the user's time zone. |
| `windowMinutes` | integer | no | `300` for five-hour windows, `10080` for weekly ones. Written for **both** providers so consumers need no provider-specific logic. |
| `state` | string | **yes** | `ok` \| `stale` \| `error`. There is no safe default, so it is required rather than assumed. |
| `error` | string | no | Short reason the window is stale or in error. Never file contents, never a response body. |
| `model` | string | no | Model a weekly window is scoped to. Absent for global windows. |
| `detailed` | boolean | no | `true` when the window came from the opt-in detailed-windows mode and nothing has replaced it since. Read it as "no status line produced this". |

## Sample

```json
{
  "schemaVersion": 1,
  "updatedAt": "2026-09-06T21:12:34Z",
  "providers": {
    "claude": {
      "configured": true,
      "plan": "max_20x",
      "source": "statusline",
      "sourceAt": "2026-09-06T21:12:30Z",
      "binding": "seven_day_fable",
      "windows": {
        "five_hour": {
          "percent": 12,
          "resetsAt": "2026-09-07T03:10:00Z",
          "windowMinutes": 300,
          "state": "ok"
        },
        "seven_day": {
          "percent": 18,
          "resetsAt": "2026-09-12T02:00:00Z",
          "windowMinutes": 10080,
          "state": "ok"
        },
        "seven_day_fable": {
          "percent": 23,
          "resetsAt": "2026-09-12T02:00:00Z",
          "windowMinutes": 10080,
          "state": "ok",
          "model": "Fable",
          "detailed": true
        }
      }
    },
    "codex": {
      "configured": true,
      "plan": "plus",
      "source": "rollout",
      "sourceAt": "2026-09-06T21:11:58Z",
      "binding": "secondary",
      "windows": {
        "primary": {
          "percent": 54,
          "resetsAt": "2026-09-07T01:41:00Z",
          "windowMinutes": 300,
          "state": "ok"
        },
        "secondary": {
          "percent": 70,
          "resetsAt": "2026-09-11T09:00:00Z",
          "windowMinutes": 10080,
          "state": "ok"
        }
      }
    }
  }
}
```

`source` is `endpoint` because the opt-in detailed-windows mode laid this block down —
and `five_hour` and `seven_day` carry **no** `detailed` flag because a newer status-line
capture replaced them afterwards, which is exactly what the precedence rules below say
happens. `seven_day_fable` keeps its flag because nothing else can produce that window.
`binding` is `seven_day_fable` because 23 is the highest of 12, 18 and 23 — the constraint
a Fable-heavy Max user actually hits, and the one the passive path cannot see.

## Which source wins (detailed-windows mode)

With the opt-in mode off there is one source and nothing to decide: `source` is
`statusline`, the two passive windows are the whole block, and none of this applies.

With it on there are two, and they do not overlap neatly:

| | `five_hour` | `seven_day` | `seven_day_<model>` |
|---|---|---|---|
| status line (passive, always on) | yes | yes | **no** |
| usage endpoint (opt-in) | yes | yes | yes |

1. **The endpoint lays down the block.** Its windows, its `plan`, and `source: "endpoint"`.
   Every window it produced carries `detailed: true`.
2. **A newer status-line capture wins on the two windows they share.** The status line is
   rewritten every few seconds while a session is open; the endpoint is asked on a timer
   and backed off from when it says no. So if `providers.claude.sourceAt` from the passive
   path is later than the endpoint's fetch, `five_hour` and `seven_day` are replaced with
   the passive readings — and lose `detailed`, because they are no longer that window.
   A passive window with **no** `percent` never replaces one that has one: "I could not
   read it" is not fresher information than a number.
3. **Model-scoped weeklies are never replaced.** Nothing else produces them.
4. **`sourceAt` is the newest source that actually contributed a window**, so it never
   claims to be fresher than the numbers it stands over.
5. **The endpoint reading goes stale after fifteen minutes**, or immediately when it is a
   remembered value from before a failure. Stale means `state: "stale"` with a short
   `error` saying why — never a dropped number, and never a `0`.
6. **With the mode on but the endpoint never having answered, the passive block passes
   through untouched.** The fallback is the default path, not an empty document.

`detailed: true` is therefore readable as "no status line produced this", which is the
question a consumer actually has. What produced the block as a whole is `source`.

## A window that could not be read

```json
{
  "percent": 54,
  "windowMinutes": 300,
  "state": "stale",
  "error": "no rollout log written since 2026-09-06T18:40:00Z"
}
```

```json
{
  "windowMinutes": 10080,
  "state": "error",
  "error": "rollout log is not valid JSON lines"
}
```

The second window has no `percent` at all. That is the point.

## Schema

JSON Schema draft 2020-12. Validation is a convenience, not the contract: a document that
this schema accepts and the rules above reject is still wrong.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://github.com/xfurqan0/nazar-tray/docs/limits-contract.md",
  "title": "nazar-tray limits.json",
  "type": "object",
  "required": ["schemaVersion", "updatedAt", "providers"],
  "properties": {
    "schemaVersion": { "type": "integer", "const": 1 },
    "updatedAt": { "type": "string", "format": "date-time" },
    "providers": {
      "type": "object",
      "additionalProperties": { "$ref": "#/$defs/provider" }
    }
  },
  "$defs": {
    "provider": {
      "type": "object",
      "required": ["configured"],
      "properties": {
        "configured": { "type": "boolean" },
        "plan": { "type": "string" },
        "source": { "type": "string" },
        "sourceAt": { "type": "string", "format": "date-time" },
        "binding": { "type": "string" },
        "windows": {
          "type": "object",
          "additionalProperties": { "$ref": "#/$defs/window" }
        }
      }
    },
    "window": {
      "type": "object",
      "required": ["state"],
      "properties": {
        "percent": { "type": "number", "minimum": 0, "maximum": 100 },
        "resetsAt": { "type": "string", "format": "date-time" },
        "windowMinutes": { "type": "integer", "exclusiveMinimum": 0 },
        "state": { "type": "string" },
        "error": { "type": "string" },
        "model": { "type": "string" },
        "detailed": { "type": "boolean" }
      }
    }
  }
}
```

`state` and `source` are typed as plain strings rather than enumerations on purpose. A
validator that rejects a value a newer writer introduced would turn a forward-compatible
document into a hard failure, which is exactly what rule 4 exists to prevent. The known
values are in the tables above.

## Times are written in UTC

`updatedAt`, `sourceAt` and every window's `resetsAt` are RFC 3339 with a `Z`, never a
local offset. The standard library has no time-zone database and no way to ask the
operating system for the current offset, so a local offset would cost either a runtime
dependency or hand-written daylight-saving code in the crate that is meant to be boring.
A UTC timestamp names the same instant, a countdown — which is what a consumer actually
draws — is offset-independent, and rendering local time is one call in every language that
reads this file. `resetsAt` is written as the source reported it, converted to UTC: both
Codex and Claude Code report Unix seconds.

Decided in WP1 and applied to the samples in WP2; the reasoning in full is in
[`pinned-internal-formats.md`](pinned-internal-formats.md).

## One writer

`limits.json` has exactly one writer at a time, and `~/.nazar/limits.lock` is how that is
enforced rather than promised. The full mechanism is in
[`pinned-internal-formats.md`](pinned-internal-formats.md); what a consumer needs to know is
three sentences:

- The lock is an ordinary JSON file with a process id, a start time and a **heartbeat** that
  the writer rewrites on every refresh — roughly once a minute, whether or not anything
  changed.
- A reader may look at it to answer "is the tray running", which `updatedAt` cannot answer
  since the writer only writes on change. A heartbeat older than five minutes means nobody
  is maintaining the file.
- **Nothing but nazar-tray should write either file.** A consumer that wants a refresh runs
  `nazar-tray --print --write`, which takes the same lock and defers to the tray if it is
  already running.

## Reserved for v2

`~/.nazar/limits/<profile>.json`, one file per account, for multi-account support
(decision K22). **v1 consumers may assume the single file at `~/.nazar/limits.json`.**
This is written down now so that adding the second file later is a v2 change in two
repositories rather than a surprise in one.

## What changing this costs

Two repositories read it, so a change is a two-repository change:

1. Update `crates/nazar-core/src/limits.rs` and `fixtures/limits.sample.json`.
2. Update this document.
3. Copy the fixture into Nazar and run its schema test.
4. Bump `schemaVersion` only for a change that removes or repurposes a field. Adding an
   optional field is not breaking — rule 4 is what makes that true.
