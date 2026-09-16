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
          "claude-opus-5":   { "input": 118, "output": 9412, "cache_create": 184203, "cache_read": 41118902, "requests": 61,
                               "raw": { "input": 196, "output": 15702, "cache_create": 307201, "cache_read": 68566141 } },
          "claude-fable-5-1": { "input": 12, "output": 1877, "cache_create": 24843,  "cache_read": 3561302,  "requests": 9,
                               "raw": { "input": 20, "output": 3131, "cache_create": 41438, "cache_read": 5938836 } }
        },
        "2026-09-13T01": {
          "claude-opus-5": { "input": 44, "output": 3110, "cache_create": 61044, "cache_read": 9330112, "requests": 22,
                             "raw": { "input": 73, "output": 5187, "cache_create": 101807, "cache_read": 15560686 } }
        }
      }
    },
    "codex": {
      "buckets": {
        "2026-09-13T00": {
          "gpt-5.6-sol": { "input": 2043, "output": 8801, "cache_create": 0, "cache_read": 1988416, "requests": 14 }
        }
      }
    },
    "claude_reported": {
      "source": "claude-stats-cache",
      "buckets": {
        "2026-09-07T00": {
          "claude-opus-5":    { "input": 0, "output": 0, "cache_create": 0, "cache_read": 0, "requests": 0, "reported_total": 2078342191 },
          "claude-fable-5-1": { "input": 0, "output": 0, "cache_create": 0, "cache_read": 0, "requests": 0, "reported_total": 652369779 }
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
| `providers` | object | Keys are `claude` and `codex` — the same two spellings `limits.json` uses — plus `claude_reported`, which is not a reader and is described in its own section below. A provider that has never been read has no key at all, rather than an empty object. |
| `providers.<p>.buckets` | object | Keys are **UTC hours**, `YYYY-MM-DDTHH` (13 characters, no minutes, no offset, no `Z` — it is an hour, not an instant). An hour in which nothing happened is **absent**, never a row of zeroes. |
| `…<hour>.<model>` | object | The model id **exactly as the source reported it**. Five counters, below, plus the optional `raw` and `reported_total`. |

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

### `raw`: the same four counters with no dedupe at all

Beside the five, an optional object with **four** of them summed **per line** rather than per
message:

```json
"raw": { "input": 196, "output": 15702, "cache_create": 307201, "cache_read": 68566141 }
```

That is the number Claude Code's own `/usage` shows, and the store keeps it because two
programs answering one question with two numbers is a thing a user can see. The dedupe rule
below is why they differ: Claude Code writes a message once per content block and every copy
carries the whole `usage` object, so adding the lines up counts a message once per block. On
the maintainer's machine, over the six days both numbers could be measured, `raw` came to
**1.667×** the deduplicated total — and the per-day figures matched `~/.claude/stats-cache.json`
**digit for digit** on all five overlapping days.

**No `requests` in it.** A request is a message and not a line, so the count beside either
reading is the deduplicated one. A per-line count would be a count of content blocks under a
label that promises replies.

**An absent `raw` is a statement, not a hole: the per-line sum of that bucket is its five
counters.** Three kinds of bucket are absent and each is honest about it:

- **Every Codex bucket.** Codex writes each event once; there are no copies to collapse, so
  the two numbers are one number and writing it twice would only invite them to drift.
- **Every bucket written before this field existed.** Those lines are behind a cursor that has
  already moved and nothing will read them again, so the per-line sum is not recoverable —
  and multiplying by 1.667 would be inventing a number, which rule 2 of
  [`limits-contract.md`](limits-contract.md) has forbidden since before this file existed.
- **Any bucket a future reader writes without one**, for whatever reason it has.

**It is written the moment anything credits a per-line sum to the bucket, seeded from the five
counters**, so the sentence above keeps being true rather than becoming a bucket whose `raw` is
smaller than its dedupe. `raw ≥ the five counters` holds hour by hour and model by model, and
it holds because the per-line sum is accumulated against the **same dedupe key** as the
deduplicated one — never per bucket, which a rewrite that swapped one message for another of
the same size could have pushed below its own dedupe.

**The `version` did not change.** `raw` is an added optional field, unknown keys survive a
rewrite, and every reader that has ever existed keeps working: an older build reading a newer
document walks past it, and a newer build reading an older document reads the absence as the
sentence above.

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

**`input + output + cache_read + cache_create` — all four.** That is the number the usage view
shows large, the number the tray tooltip carries, the number a calendar cell is shaded from,
and the number "this week" means without a qualifier. It is **the same definition Claude
Code's own `/usage` calls *total tokens***, and that is the whole of the argument: a user who
has both windows open is comparing two numbers, and two numbers that answer one question have
to be one number.

**The four parts are shown beside it, always.** Under the headline and under every model row,
in the view itself rather than in a footnote:

```text
In 802K · Out 354K · Cache read 1B · Cache write 253K
```

An absent counter prints an em dash there rather than dropping out of the line, because a
breakdown that does not add up to the number above it is worse than one with a hole named in
it.

### Why the breakdown exists

Because the headline is overwhelmingly cache. Measured on the maintainer's machine over six
days of real work, deduplicated:

| | tokens | share |
|---|---|---|
| `cache_read` | 1 491 769 695 | **98.5 %** |
| `cache_create` | 20 109 628 | 1.3 % |
| `output` | 2 145 844 | 0.14 % |
| `input` | 43 724 | 0.003 % |

A total is 98.5 % cache reads, so a number on its own is a number about cache behaviour with
the work lost inside the rounding: `input + output + cache_create` — the part that was actually
produced or newly processed — is 22.3 M of that 1.5 B, and it is the one that moves when a day
was busy. **That measurement is why the four parts are on screen.** It is no longer why the
headline is smaller than `/usage`'s, which is what it used to argue for and what T-WP16 wrote
down here: the arithmetic was right and the reader was not served by it. Side by side the two
windows said 1.4 M and 1.0 B, and a tray answering `/usage`'s question with 0.1 % of `/usage`'s
answer does not read as careful — it reads as broken. So the total is the total, and the reader
can see for themselves, on the same screen, that the billion is the cache.

Everything is in the file either way. The store has never held a headline: it holds four
counters per model per hour, and which sum is drawn large is a decision the view makes, not a
fact the document records.

**Both surfaces use this definition.** `nazar_tray::usage::headline` — the tray tooltip's *This
week* line, and the model it names as the busiest — is the same four-way sum as the panel's, so
the tooltip and the view cannot disagree about a number the user can check against a third
window. Between T-WP20 and T-WP20b they briefly did, which is written up in `PROJECT.md` §9.

## `claude_reported`: days this store never measured

A third key under `providers`, and **not a reader**. It is a copy of what Claude Code's own
`~/.claude/stats-cache.json` says about the days **older than the transcripts** — days this
store has no way to count, because Claude Code has already pruned the files they were in.

```json
"claude_reported": {
  "source": "claude-stats-cache",
  "buckets": {
    "2026-09-07T00": {
      "claude-opus-5": { "input": 0, "output": 0, "cache_create": 0, "cache_read": 0,
                         "requests": 0, "reported_total": 2078342191 }
    }
  }
}
```

| Field | Meaning |
|---|---|
| `source` | `claude-stats-cache`, on the provider block rather than on every row. The key already says where the numbers came from; this says it in words, in the one place somebody opening the file will look. |
| bucket key | `YYYY-MM-DDT00` — **the date that program computed, at hour zero of itself**. It is a day, not an hour: nothing happened at midnight in particular, and the hour is there because this file's keys are hours. |
| `reported_total` | The one number the source holds for that model on that day. **Per-line**, like everything in that file. |
| the five counters | `0`. `stats-cache.json` holds one total per model per day and no split, and writing a guess at the split would be inventing four numbers out of one. |

Seven rules, and each of them is what keeps this from becoming a second, quieter source of
truth:

1. **It is off by default.** `usage.fillHistoryFromStats` in `config.json`, and a history that
   quietly mixes two kinds of number is worse than a shorter one.
2. **It is never merged into `claude`** and never added to any counter of it. A reader summing
   the providers gets what this machine measured; this block is lifted out before the panel
   sees the providers at all.
3. **Only days strictly before the transcripts.** The boundary is the earliest hour the
   `claude` provider holds, floored to its UTC day — *not* the store-wide `since`, which on a
   machine with Codex on it is dragged back by rollout logs that say nothing about Claude
   Code's transcripts. On the maintainer's machine the two differ by twelve days.
4. **It does not move the boundary it measures itself against.** `since` is computed from the
   measured providers alone; counting a backfilled day in it would move the boundary back
   behind itself and leave the store oscillating between two answers.
5. **It is a copy, not an accumulation.** Every run rewrites the whole block from the file, so
   running it twice writes the same bytes and a day the file no longer holds stops being in
   the store. There is no journal, no generation and no `applied_through`: there is nothing to
   be idempotent *about*.
6. **The dates are that program's, and they are carried as dates.** They were measured against
   UTC days and matched digit for digit. A reported day is handed to the panel as
   `YYYY-MM-DD`, never re-bucketed through a time zone, so nothing between the file and the
   screen converts an offset and nothing can be an hour wrong about a number that never had an
   hour. The one boundary case a negative offset can produce — a local day that is partly
   covered by transcripts and also carries a reported number — is dropped by the panel, which
   is the side that knows the offset.
7. **The panel draws it apart.** Outlined rather than shaded on the calendar, outlined in the
   weeks list, and *reported by Claude Code* in words wherever it appears — because a shade is
   a rank against the days beside it, and these are counted in a different unit.

**Why a copy of another program's arithmetic is in this file at all.** Because the alternative
was a history that begins the day this product was installed, on a machine whose transcripts
reach back six days and whose statistics cache reaches back twenty-two. The numbers are real;
what they are not is *ours*, and every one of the seven rules above exists to keep that
distinction visible rather than to hide it.

## The two counts, and which one the panel draws

`docs/pinned-internal-formats.md` pins the fields; this is the decision on top of them.

**The default is the deduplicated spend.** It is the number that answers *what did this machine
use*, and it is the only one here that is a measurement of that.

**`usage.countLikeClaudeCode` swaps every counter for its per-line twin.** `get_usage` then
answers with `mode: "per_line"` and the same bucket shape, so the panel's arithmetic, its
calendar, its weeks and its details are one set of functions over one shape rather than two
copies with two chances to disagree. The headline carries a small tag saying so, and the tray
tooltip follows the same setting — the two surfaces of this application cannot answer one
question with two numbers, which is what `PROJECT.md` §9 spent a package on.

The reason a user would want it: `/usage` shows the per-line number, and somebody comparing the
two windows deserves to be able to make them agree rather than being told one of them is wrong.
The reason it is not the default:
[anthropics/claude-code#91775](https://github.com/anthropics/claude-code/issues/91775#issuecomment-5654151098)
— the per-line sum counts one message once per content block, which is 1.667× the real spend
here and is not a constant that could be divided back out.

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
- **`raw` is idempotent too, and it needed a second number to be.** The deduplicated side can
  say *the largest reading, minus what is already credited* in either direction, because every
  copy of a message carries the whole `usage` object. The per-line side is a **sum over
  lines**, and the same line read twice is two lines unless something remembers that it is
  not. So each dedupe key carries two per-line figures rather than one: the **high-water
  mark**, which is what the bucket has already been credited, and **what the file holds now**,
  which a restart forgets and an append adds to. What is credited is the difference, saturating
  at nothing, and the invariant is:

  > **A bucket's `raw` counters are the largest per-line sum the transcripts behind them have
  > ever held, and reading any byte a second time adds nothing to them.**

  The sequence a byte offset alone gets wrong: a transcript is pruned to its first line and a
  copy of the original is then put back. The restart credits nothing, and the pass after it is
  an **ordinary append** — valid offset, matching fingerprint, nothing to say the file ever
  shrank — which would credit lines two and three a second time, permanently. With *what the
  file holds now* it climbs back to where the high-water mark already is, and credits nothing.
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
  every `(message.id, requestId)` key it has produced, as a bare array of **nine** numbers per
  key — the hash, the largest reading of the message, and that message's per-line sum — or
  thirteen for a key whose file has been truncated below its high-water mark, where the last
  four say what the file holds now. (A row of **five** is one written before `raw` existed:
  its per-line half is seeded from its deduplicated half, which is a lower bound and is
  exactly what an absent `raw` on a bucket already means, so the two cancel and the first
  restart after an upgrade credits the file's true per-line sum once.) For a rollout, the
  fingerprint of every event. That is
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

`claude_reported` has no cursor and no journal at all, because it is not a reader: its whole
block is rewritten from `stats-cache.json` on every pass that runs it. Deleting it by hand
costs nothing; the next pass puts it back.

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

**A compressed rollout is read.** Codex has shipped a worker since 0.153.4 that rewrites every
rollout whose mtime is more than seven days old as `<name>.jsonl.zst` and deletes the plain
file; it sits behind the `local_thread_store_compression` flag, measured `under development`
and `false` on 2026-09-15 under 0.154.0. T-WP25 gave this pass the name and a count, because a
week of history disappearing with nothing anywhere to say so is the one failure this document
exists to prevent. T-WP26 gave it a decoder, so those events are now in the totals like any
others. `UsageSummary::files_compressed`, summed into the bridge's
`UsageScan.files_compressed`, is how many of the logs a pass read were archives — still `0` on
every machine whose Codex has the flag off. The format is in
[`pinned-internal-formats.md`](pinned-internal-formats.md) under "Compression".

**Two things follow from compression being a rename**, and both are load bearing:

* **A rollout's cursor is filed under its plain name.** `<name>.jsonl` and `<name>.jsonl.zst`
  are one session in the two states Codex keeps it in, so keying the cursor on the name on
  disk would meet the archive as a log nothing had ever read and credit the whole session a
  second time. Keyed on the plain name, the sweep is a file that was replaced: the cursor's
  identity check notices, the pass restarts, and the events already credited are matched off
  one by one. Measured, in the test named after it: a forty-event log swept without this
  credits eight of them twice — the eight past the fork rule's thirty-two-event reach.
* **Only one of the two names is walked.** During the sweep, and after Codex reopens an
  archived thread, both exist for a moment. The plain one is the log; the archive beside it is
  passed over and not counted.

**An archive that will not decode is a file that failed to open**, `files_unreadable`, with its
cursor kept exactly as it was — the same treatment a locked transcript gets, for the same
reason: damage today may be readable tomorrow, and a pass that credited half a log and wrote a
finished cursor could never find out.

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

What is in it is what the table above lists: five integers, four more under `raw`, a model id,
a UTC hour. The reader that produces it builds those values into a new object instead of
filtering a parsed line, and a leak test feeds it records whose every text field is a sentinel
and fails if the sentinel turns up in the store or in anything the store serialises. The
statistics reader is held to the same rule and the same test: it has a field for three keys of
`stats-cache.json` and for nothing else, so `longestSession.sessionId` — an identifier of a
session on this machine — is walked past by the deserialiser rather than filtered out by hand. The full inventory of what is read
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
3. Bump `version` only for a change that removes or repurposes a field. `raw`,
   `reported_total` and the `claude_reported` provider were all **added**, so `version` is
   still `1`: unknown keys survive a rewrite, an older build walks past them, and a newer
   build reads their absence as the sentence each of them defines.
4. The day a second program reads this file, add the step that copies a sample into it — and
   that day, this page stops being a plan and becomes a promise.
