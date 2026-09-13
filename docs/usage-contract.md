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

All five are non-negative integers, **always present, and `0` where nothing reported one**.
A bucket never omits a counter and never writes `null`: it is a sum over many records, and a
sum of nothing is zero. (The distinction between *absent* and *zero* is real one record at a
time, and it is kept there — a line that named no counter at all is skipped rather than
counted as four zeroes — but it does not survive into a total, and a document that sometimes
omitted two of five fields would make every reader write the `?? 0` the writer was avoiding.)

**Nothing nested is added to them**: not `output_tokens_details.thinking_tokens`, not
`usage.iterations[]`, not `cache_creation.ephemeral_5m/1h`, not Codex's
`reasoning_output_tokens` — every one of those is already inside a counter above, and adding
it is how a total silently doubles.

**A counter that is not a non-negative integer is not read as one.** `1.9` is not truncated
to `1` and `"7"` is not parsed to `7`: the line is counted as malformed and skipped, and the
rest of the file is read as usual. A source that changed the shape of a field has stopped
saying what it used to say, and a reader that guesses at the new meaning produces a number
that is wrong without looking wrong. `null` is not a changed shape — it is a counter the
source did not report, and it lands as `0` in the bucket like any other absence.

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

## How a scan adds to the store, and why a bucket never goes down

A scan does not recompute the store and does not rescan the logs. It reads **the bytes
nobody has read yet** — each log is followed by a cursor, `(file identity, byte offset)` —
turns them into hourly buckets, and **adds** those to the months they belong to. Nothing
already in a month is recomputed or compared away; a counter only ever grows, and it grows
by exactly what this pass read.

That is the point of the file rather than a detail of it. Claude Code prunes transcripts (30
days by default; six days of history survived on the maintainer's machine) and Codex sessions
are the user's to delete or archive. A store that recomputed from the logs would answer "all
time" with "the last few days" and would answer differently every week. A store that adds
keeps what the logs no longer hold, and a pruned transcript costs nothing that was already
counted.

Three consequences worth naming:

- **A second scan over unchanged logs adds nothing at all** — not because the numbers agree
  but because there are no new bytes to read. Every offset is already past the end, no month
  document differs from what is on disk, and nothing is rewritten: the directory is left byte
  for byte as it was.
- **And reading the same bytes *again* also adds nothing.** A log that was truncated, rotated
  or rewritten is read from the top, which is the one moment the offset stops being a
  guarantee. What stands in for it is the cursor's record of everything that log has already
  been credited for: a message it holds credits only what it has grown by, and an event it
  holds credits nothing. So the totals are the same after a prune as before one — which is
  what the word idempotent above is worth, and what the first version got wrong past the last
  sixteen messages of a file.
- **Largest-wins is a rule about copies of one message, and it lives before the buckets.**
  Claude Code writes a message once per content block and every copy carries the whole
  `usage` object; the scan keeps the copy with the largest total, credits that one, and a
  bucket never sees the others. What is remembered for the key is the **largest** reading
  ever seen, counter by counter, and never the latest: three passes seeing 100, then 90, then
  100 credit 100 once, where remembering 90 would have let the third pass add another 10.
  Codex writes each event once and gives it no id, so there is nothing to deduplicate there —
  and nothing to deduplicate *with*; see the Codex section below for what stands in its place.
- **Numbers are never revised, including a wrong one.** There is no pass that could revise
  them: the scan that would have to notice is the one that already moved its cursor past
  those bytes. If a scan ever over-counts, the fix is a **rebuild** — the cursors and the
  months deleted together, and whatever the logs still hold counted again from the top.
  Whatever they no longer hold is gone, which is why a rebuild is a decision somebody makes
  rather than something a reader does to recover.

## The cursor documents, and the one way to double count

Beside the month documents, in the same directory: `cursors-claude.json` and
`cursors-codex.json` — **one per reader**. They are not part of this contract. They are the
writer's own bookkeeping, nothing reads them but the scan that wrote them, and their shape
may change in any release without a `version` bump here.

What is worth writing down is what they are *for*, because deleting one has a consequence
nobody would guess:

- **They hold where each log was read up to**, filed under a hash of its path rather than the
  path — `~/.claude/projects/` is named after every working directory somebody has opened a
  session in, and none of that belongs in a file this product writes. A credited message is
  filed under a hash of its identifiers, and a rollout's events under a hash of their
  timestamps and counters. Nothing in these documents names anything on the machine.
- **"Where it was read up to" is three things, not two.** The file's identity says *this is
  the same file*; the byte offset says *this is how far*; and a hash of the 64 bytes
  immediately before that offset says *and this is still the same place*. The third exists
  because the first two can both be satisfied by a file that was rewritten in place — same
  birth time, same first 512 bytes, same length — inside which the old offset now points at
  different content, and everything written before it would never be read. A mismatch is
  treated exactly like a truncation: read the file again from the top.
- **They hold what has already been credited, per log, in full.** For a transcript that is
  every `(message.id, requestId)` key it has produced and the largest reading of each, as a
  bare array of five numbers per key; for a rollout, the fingerprint of every event. That is
  what makes reading a log from the top again *safe* rather than a doubling: a re-read record
  credits the difference between the largest copy now and the most already credited, which
  for the same bytes is nothing. It is also the size of these documents — a few hundred
  kilobytes on a machine with 9 500 messages of history — and the trade is deliberate: a
  bounded window of recent keys is smaller and only protects the last few messages of each
  file, which is not where a truncation starts.
- **They are also the write-ahead half of the counting invariant.** A pass writes its new
  offsets *and* the totals read from them in one atomic write, as a `pending` journal, before
  those totals reach any month; then each provider's block in a month document stamps the
  scan `generation` it last absorbed in `applied_through`, so replaying a pending entry is a
  no-operation for a month that already carries it. A crash before that write loses a pass
  that is simply repeated; a crash after it leaves entries the next pass files. There is no
  state in which an offset moved past bytes whose totals were never recorded, and none in
  which totals were recorded twice.
- **The journal is per month, and an entry that cannot be filed stays in it.** A month whose
  document no longer parses is skipped, and its entry waits — through any number of scans —
  until the month can be written. See the damaged-month section below for what that is worth.
- **A cursor document that no longer parses is an error, not a fresh start.** Treating it as
  absent would reset every offset to zero and count every surviving log into months that
  already hold it. The scan stops and says so instead.
- **Deleting one by hand double counts, and nothing can detect it.** An absent cursor is
  indistinguishable from a machine that has never scanned — which is what makes the only
  supported reset `store::rebuild()`: the cursors **and** the month documents, removed
  together. Removing the months alone leaves a store that will never read those logs again;
  removing the cursors alone adds every surviving log to months that already hold it. Either
  half on its own is the bug; both together is the escape hatch.

## The two providers, and what Codex spells differently

`claude` and `codex` file into the same month documents, under their own key, with their own
`applied_through` stamp and their own cursor document. Neither reader can disturb the other's
offsets, and a month may hold one, both, or neither.

The Claude side is `message.usage` as the server reported it, deduplicated by
`(message.id, requestId)` — minus two kinds of line that carry a full set of counters and are
not billed usage: a message whose model is `<synthetic>`, and a line marked
**`isApiErrorMessage`**, which is what an API failure looks like in a transcript. (On the
maintainer's machine all 11 of those were also `<synthetic>`, so reading the flag changed no
total there; it is read because the flag, not the model name, is what promises the line is an
error, and a billed model name marked as an error would otherwise be counted.)

The Codex side reads `payload.info.last_token_usage` on every `token_count` event and differs
in four ways that are visible in the file:

- **`input` has the cache taken out of it.** Codex's `input_tokens` **includes** the cached
  part, so the store writes `input_tokens − cached_input_tokens` as `input` and
  `cached_input_tokens` as `cache_read`. Adding the two as reported would count the cache
  twice. (Claude Code reports them as separate numbers already, which is why only this side
  subtracts.)
- **`cache_create` is `0`.** It is `cache_write_input_tokens` when the event carries one —
  the field is read rather than assumed, so the day Codex starts reporting cache writes the
  store carries them — and it was `0` on every event observed on the maintainer's machine, all
  446 the reader counted and the 438 in the archived tree beside them.
  A missing one is read as `0` rather than as unknown: a turn that wrote no cache wrote none.
- **The model comes from another line.** A `token_count` event does not name the model that
  produced it; the session's `turn_context` lines do, and the store attributes an event to the
  last model named before it. An event that arrives before any `turn_context` is filed under
  **`unknown`** — the one id this file writes itself, and the reason it exists.
- **The cumulative counter is never read.** `info.total_token_usage` looks like a session
  total and is not: it falls back down mid-session when the context is compacted, in 3 of the
  22 logs under `sessions/` here, and summing the per-turn counter disagreed with it in 8 of
  30 files, once by a factor of 43. Nor are `reasoning_output_tokens`, which is already inside
  `output_tokens`, and `total_tokens`, which is the sum of two counters that are already here.

**Counting an event once, without an event id.** Claude Code's `(message.id, requestId)` has
no counterpart in a rollout log, so *which bytes have been read* is the whole answer, and it
is exact as long as a log is only appended to. Two shapes get past it, and each has a rule.

The first is a log that is **truncated or rewritten**, after which the reader goes back to
byte zero and reads events it has already counted. So the cursor carries a fingerprint of
every event it has credited — the event's timestamp to the millisecond and its four raw
counters, hashed — and a pass that had to restart matches what it reads against that set
before crediting anything, consuming each match, so a log that genuinely holds two identical
events keeps both. It is the same thing the transcript reader does with its dedupe keys, built
out of the only evidence a rollout offers.

The second is a fork or a resume that copies a run of events into a **new** log, which arrives
as a new path with a cursor that has read nothing. The rule, stated so it can be argued with:

> A log whose **opening run** of events is, event for event — same timestamp to the
> millisecond, same four counters — the opening run of a log already known is a copy of it up
> to the point where the two diverge. That run is skipped; whichever of the two was read first
> keeps the tokens, and it does not matter which, because exactly one copy is counted.

Only a leading run, and only against another log's leading run: the same event in the middle
of two sessions is a coincidence worth nothing, while the same event *first in both files* is
not a coincidence at all. The guard is bounded — only the first 32 events of each log are
compared, because the rule asks every log about every other log — so a copy longer than that
is caught for its first 32 events and counted again for the rest. No fork on the maintainer's
machine copied any events at all; the rule is there because a byte offset alone would have no
answer if one did.

**`archived_sessions/` is not read.** Codex keeps logs of exactly this shape in a second tree,
and this store walks only `sessions/`. The reason is mechanical rather than squeamish: a cursor
is filed under a hash of the path, so a log that Codex *moves* into `archived_sessions/`
arrives as a file nothing has read, and every event in it would be counted a second time.
Reading one tree and not the other is what makes archiving a session leave the totals exactly
as they were. The cost, stated rather than discovered: **a session archived before it was ever
scanned is never counted at all.** Changing that means reading both trees and telling them
apart by something other than a path, and it is not a line this file can add on its own.

## A damaged month is left alone

A file that does not parse is **reported and kept**, never replaced by an empty one and never
"repaired". The months beside it still load; the damaged one displays as absent, with the error
visible rather than swallowed.

That is the opposite of `alerts.json`'s rule, and for the opposite reason: a lost alert record
costs one extra toast, while a lost month costs a month that may no longer exist anywhere else.
Deleting it is a thing the user does, once they know.

**What it costs in the pass that finds it: nothing, and that is the fix.** The totals that
scan had just read for that month stay in the journal inside the cursor document, one entry
per month, and every later scan tries again. The moment the month becomes readable — the user
deleted the broken file, or put back a copy of it — the entry is filed, once, guarded by the
same `applied_through` stamp as everything else. Every month beside it keeps its own totals
and files them immediately.

The first version dropped those totals instead, which was a quiet way of losing them for good:
the bytes they came from are behind a cursor that has already moved, so nothing would ever
read them again, and repairing the month afterwards could not bring them back. A journal that
waits is the whole of the difference.

Two honest limits. A month that stays damaged keeps one merged entry, carrying the newest
generation, so the journal does not grow with every scan — and a user who "repairs" the month
by restoring an **older** copy of it, one whose `applied_through` is behind some of the
generations merged into that entry, gets those generations counted twice. Nothing can tell
that document apart from the one that was damaged. A rebuild is still the supported answer,
and it brings a month back only as far as the logs still reach.

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
