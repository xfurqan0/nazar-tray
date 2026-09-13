# The usage store — the contract

The second file nazar-tray writes, and the first one that is about **what you used** rather
than **what is left**. `limits.json` answers *how much of my window is gone?*; this one
answers *how many tokens did I actually spend, and on which model?*

It is written here **before** the code exists, because the decision it rests on — that
nazar-tray reads Claude Code's transcripts at all — reverses a decision this repository had
already written down, and a reversal that arrives as a surprise in a diff is not a decision.
The reversal itself is in [`PROJECT.md`](PROJECT.md) §8 and the fields are pinned in
[`pinned-internal-formats.md`](pinned-internal-formats.md); this page is the file that comes
out the other end.

- **Location:** `%APPDATA%\nazar\usage\YYYY-MM.json` on Windows;
  `$XDG_CONFIG_HOME/nazar/usage/` or `~/.config/nazar/usage/` elsewhere;
  `$NAZAR_HOME/usage/` when that override is set. **One file per UTC calendar month.**
- **Written by:** the nazar-tray process, and only it. One writer, many readers.
- **Written how:** temp file in the same directory, then rename — the same
  `crates/nazar-core/src/atomic.rs` that writes `limits.json`. The whole month is rewritten
  every time. Nothing in this repository appends to a file.
- **Schema:** `version: 1`. Unknown keys survive a rewrite.
- **Read by:** nobody yet. This document is what would make it safe to read.

## Why it is not in `~/.nazar`

[`limits-contract.md`](limits-contract.md) draws the line and this file lands on the other
side of it: everything a *consumer* reads lives under `~/.nazar`, and the files that are the
**user's own** live where the platform keeps settings — `config.json`, `alerts.json`, and now
this.

Two reasons, and the first is rule 1 of that contract. **`limits.json` is safe to paste into
a bug report**: two percentages and two reset times say nothing about what anyone was doing.
A month of hourly token counts is a usage profile — when this machine works, how long the
sessions are, which model does the heavy lifting. It breaks no rule about credentials, and it
is still not a thing to hand over by reflex, so it sits with the user's own files rather than
in the directory this project tells other programs to read.

The second is scope. **Nazar does not read this file**, and no other program does either. If
Nazar ever should, that is its own work package on both sides — this page is what that package
would be written against, which is the whole reason it exists now rather than then.

**The consequence, stated rather than discovered:** `%APPDATA%\nazar` goes when an uninstall
is told to *delete application data*, and that takes the history with it — the same tick
already takes the settings and the notification log. `~/.nazar` is the directory no uninstall
path touches, and `limits.json` is what lives there. A user who wants the history to outlive
the application copies the directory; a user who ticks the box meant it.

## The document

```json
{
  "version": 1,
  "month": "2026-09",
  "since": "2026-09-07T04:13:52Z",
  "scanned_at": "2026-09-13T01:22:09Z",
  "providers": {
    "claude": {
      "buckets": {
        "2026-09-13T00": {
          "claude-opus-5":   { "input": 118, "output": 9412, "cache_create": 184203, "cache_read": 41118902, "requests": 61 },
          "claude-fable-5-1": { "input": 12, "output": 1877, "cache_create": 24843,  "cache_read": 3561302,  "requests": 9 }
        },
        "2026-09-13T01": {
          "claude-opus-5": { "input": 44, "output": 3110, "cache_create": 61044, "cache_read": 9330112, "requests": 22 }
        }
      }
    },
    "codex": {
      "buckets": {
        "2026-09-13T00": {
          "gpt-5.6-sol": { "input": 2043, "output": 8801, "cache_create": 0, "cache_read": 1988416, "requests": 14 }
        }
      }
    }
  }
}
```

| Field | Type | Meaning |
|---|---|---|
| `version` | integer | `1`. Its own number, unrelated to `limits.json`'s `schemaVersion` and to `config.json`'s. Bumped only by a change that **removes or repurposes** a field; adding an optional one is not breaking, because unknown keys survive. |
| `month` | string | `YYYY-MM`, **UTC**, and the same value as the file name. Written into the document so a file that was renamed or copied still says what it is. |
| `since` | string | RFC 3339 `…Z`. The **earliest instant any bucket in this store came from** — not the earliest in this file. It is what the panel's *since {date}* line reads, and it is the honest boundary of the words "all time": the first scan, plus however far back the transcripts still reached on the day it ran. |
| `scanned_at` | string | RFC 3339 `…Z`. When the scan that produced this document finished. A diagnostic: it answers "is this history being kept up to date" the way `limits.lock`'s heartbeat answers "is the tray alive". |
| `providers` | object | Keys are `claude` and `codex` — the same two spellings `limits.json` uses. A provider that has never been read has no key at all, rather than an empty object. |
| `providers.<p>.buckets` | object | Keys are **UTC hours**, `YYYY-MM-DDTHH` (13 characters, no minutes, no offset, no `Z` — it is an hour, not an instant). An hour in which nothing happened is **absent**, never a row of zeroes. |
| `…<hour>.<model>` | object | The model id **exactly as the source reported it**. Five counters, below. |

### The five counters

| Counter | Claude source | Codex source |
|---|---|---|
| `input` | `message.usage.input_tokens` | `last_token_usage.input_tokens` **minus** `cached_input_tokens` |
| `output` | `message.usage.output_tokens` | `last_token_usage.output_tokens` |
| `cache_create` | `message.usage.cache_creation_input_tokens` | `last_token_usage.cache_write_input_tokens` when present, else `0` |
| `cache_read` | `message.usage.cache_read_input_tokens` | `last_token_usage.cached_input_tokens` |
| `requests` | deduplicated assistant messages that carried a `usage` object | `token_count` events counted |

All five are non-negative integers. **Nothing nested is added to them**: not
`output_tokens_details.thinking_tokens`, not `usage.iterations[]`, not
`cache_creation.ephemeral_5m/1h`, not Codex's `reasoning_output_tokens` — every one of those
is already inside a counter above, and adding it is how a total silently doubles.

`requests` is a count of *records that carried usage*, not of your prompts: one turn can be
several assistant messages, and a subagent's messages are its own. It is there so a reader can
say "14 responses" instead of implying a session count it does not have.

### Model ids are stored as reported, never canonicalised

`claude-opus-5` and the bare `opus` both appear in real transcripts, sometimes in the same
file, and they are the same model. The store **does not merge them**, because merging means a
table of aliases that has to be right about a name nobody here controls, and a wrong merge is
unrecoverable once it has been written. A reader that wants to group them may; it will be
grouping something it can still see, which is the difference.

Ids are never translated either. `claude-opus-5` is a name, not a string to localise, and the
six locale files have nothing to say about it.

**The one id this file writes itself is `unknown`**, for records whose source never named a
model — a Codex `token_count` event before the first `turn_context` in its log. Those tokens
were spent and are counted; which model spent them is a thing nobody here knows, and saying so
is cheaper than attributing them to the model that happened to come next.

### Why these key names

`limits.json` renames everything it reads into camelCase, because it is a contract with
another program and it has one spelling for an idea two sources spell differently. This file
does the opposite on purpose: its counters keep **the shape the usage fields already have at
both sources**, so a reader comparing this document with a raw `message.usage` block does not
have to hold a rename in their head. The keys that are about the document rather than about
tokens — `version`, `month`, `since`, `scanned_at`, `providers`, `buckets` — are this file's
own, and there is exactly one of them (`scanned_at`) whose `limits.json` cousin is spelled the
other way.

If that trade ever stops being worth it, it is a `version` bump and a two-line mapping, not a
migration: nothing outside this repository reads the file.

## No local time, anywhere in this file

Every instant here is UTC, and every bucket key is a **UTC hour**. There is no offset field,
no time-zone name, and no local date — and there will not be one without a decision in front
of it.

This is enforced rather than promised:
`crates/nazar-core/tests/hygiene.rs::nothing_in_the_workspace_asks_the_machine_what_time_zone_it_is_in`
greps every `.rs` file in the workspace for the names a local-time conversion would have to
use and fails the build on a hit. The reasoning is the one at the end of
[`pinned-internal-formats.md`](pinned-internal-formats.md): the standard library has no
time-zone database, so a local offset in the Rust core costs either a dependency or
hand-written daylight-saving code in the crate that is meant to be boring, while the panel is
JavaScript, where a local date is one call.

**So the grain is an hour, not a day, and that is the whole reason.** A UTC *day* cannot be
split into local days at +03:00 — the boundary falls inside it, and no arithmetic afterwards
can put it back. UTC *hours* can: a reader at any whole-hour offset re-buckets them into local
days exactly, by shifting the key.

**The panel derives, the store stores.** Local days, and **weeks that start on Monday in local
time**, are computed by the reader from these hours, at read time, every time — the same rule
`limits-contract.md` already applies to countdowns and binding windows: the file holds what was
measured, the display holds what changes with the clock.

**The one honest limit:** an offset that is not a whole number of hours — `+05:30`, `+05:45` —
has a local day boundary inside a bucket. Such a bucket is attributed to the local day it
*starts* in, so at most one hour of a day's tokens can land on the neighbouring day. The
alternative is splitting a bucket by a ratio, which invents numbers, and rule 2 of
`limits-contract.md` is older than this file.

## The headline number

**`input + output + cache_create`.** That is the number a bar is drawn from, the number the
tray tooltip carries, and the number "this week" means without a qualifier.

**`cache_read` is reported beside it and never folded into it.** Measured on the maintainer's
machine over six days of real work, deduplicated:

| | tokens | share |
|---|---|---|
| `cache_read` | 1 491 769 695 | **98.5 %** |
| `cache_create` | 20 109 628 | 1.3 % |
| `output` | 2 145 844 | 0.14 % |
| `input` | 43 724 | 0.003 % |

A raw total is 98.5 % cache reads, so a chart of raw totals is a chart of cache behaviour with
the work hidden inside the rounding. `input + output + cache_create` is the part that was
actually produced or newly processed — 22.3 M against 1.5 B — and it is the one that moves when
a day was busy. Both are in the file; only one is the headline.

## No cost, in v1

There is **no cost field**, and this is not an omission to be fixed by adding one quietly.

A subscription's list price is not a bill. Multiplying these counters by a published rate
produces a dollar figure that is not what anyone paid, that has to be footnoted on every row,
and that is wrong the week a price changes — while a price table is exactly the kind of upkeep
that goes stale first and loudly. Claude Code's status-line payload does carry a real
cumulative `cost.total_cost_usd`, and it is still a local estimate from list prices rather than
an invoice; Codex publishes nothing comparable at all.

If cost is ever added it is a `pricing.json` **data file**, a `version` bump here, an explicit
"estimated from list prices, not your bill" beside every figure, and a model that is not in the
table showing **no cost** rather than a guess. That is T-WP19, and it is deliberately after v1.

## Merging a scan, and why a bucket never goes down

A scan recomputes the buckets it can see. It **merges** into the store rather than replacing
it, taking the **larger** of the two values per counter.

This is not caution, it is the point of the file. Claude Code prunes transcripts (30 days by
default; six days of history survived on the maintainer's machine) and Codex sessions are the
user's to delete. A scan run after a prune computes *smaller* numbers for an old hour, and a
store that took the newest answer would quietly erase the history it exists to keep. Largest
wins, so a pruned transcript costs nothing that was already counted.

Two consequences worth naming:

- **A full rescan is safe and idempotent.** Deduplicating by `(message.id, requestId)` makes a
  second scan of the same lines produce the same numbers, and largest-wins makes merging them a
  no-op. "Rebuild from scratch" is therefore a real escape hatch: delete the month file and
  rescan, and everything the transcripts still hold comes back.
- **Numbers are never revised downwards, including a wrong one.** If a scan ever over-counts, a
  later correct scan does not undo it; the fix is to delete the month and rebuild. That is the
  price of not losing pruned history, and it is the right way round — an honest total that
  cannot shrink beats a shrinking one nobody can explain.

## A damaged month is left alone

A file that does not parse is **reported and kept**, never replaced by an empty one and never
"repaired". The months beside it still load; the damaged one displays as absent, with the error
visible rather than swallowed.

That is the opposite of `alerts.json`'s rule, and for the opposite reason: a lost alert record
costs one extra toast, while a lost month costs a month that may no longer exist anywhere else.
Deleting it is a thing the user does, once they know.

## One writer

The scan runs inside the tray process, on the thread that already owns every reader, and it
writes under the advisory lock in `~/.nazar/limits.lock` that already makes "one writer" true
for `limits.json` — no second lock, no second discipline. A tray whose lock was reclaimed while
its machine slept stops writing this file at the same moment it stops writing that one.

**The scan is not on the refresh path.** Quota is the reason this application exists and it
reads two small files in milliseconds; a usage scan reads hundreds of megabytes. It runs once
at start-up, when the usage view is opened, and at most once every five minutes — and a quota
reading never waits behind one.

## What this file never contains

No prompt text. No response text. No reasoning. No tool input or output. No file path, project
name, working directory, git branch, session id, message id, request id, or account
identifier. Nothing that is a name of anything on this machine.

What is in it is what the table above lists: five integers, a model id, a UTC hour. The reader
that produces it builds those values into a new object instead of filtering a parsed line, and
a leak test feeds it records whose every text field is a sentinel and fails if the sentinel
turns up in the store or in anything the store serialises. The full inventory of what is read
and what is not is [`pinned-internal-formats.md`](pinned-internal-formats.md).

## `limits.json` does not change

**Nothing on this page touches `limits.json`.** It stays frozen at `schemaVersion: 1`, it keeps
the same fields, and Nazar's quota strip needs no change on account of any of this — which is
the reason this is a new file rather than a field added to that one. The contract there is
explicit that `cost.*` and `context_window.*` have no field and are "not growing one", and this
is what growing somewhere else looks like.

`~/.nazar/limits/<profile>.json` is still reserved for v2 multi-account support, and this file
would gain the same split in the same release if it ever does.

## What changing this costs

Today, one repository: this one.

1. Update the writer and its tests in `crates/nazar-core/src/usage/`.
2. Update this document, in the same commit.
3. Bump `version` only for a change that removes or repurposes a field.
4. The day a second program reads this file, add the step that copies a sample into it — and
   that day, this page stops being a plan and becomes a promise.
