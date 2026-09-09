# Pinned internal formats

nazar-tray reads files that belong to other people's programs. None of them is a public
API, so every one of them can change without notice. This page is the inventory: what is
read, which fields the code depends on, the version the shape was observed under, and the
fixture that pins it. It is the sibling of
[Nazar's page of the same name](https://github.com/xfurqan0/nazar/blob/main/docs/pinned-internal-formats.md);
the two repositories read different files and keep separate inventories.

**Codex row observed under Codex `cli_version` 0.153.4, Windows 11, 2026-09-07**, across
**18 rollout logs** written between 2026-08-27 and 2026-09-07 and containing **329
`rate_limits` lines** (652 window objects). **Claude rows observed under Claude Code
2.1.263, Windows 11, 2026-09-07**: one captured status-line payload and the maintainer's
own `settings.json`. The **usage endpoint and sign-in rows were observed live on
2026-09-07** in a single read-only request on the maintainer's own machine, and neither has
a fixture on purpose — a real response is a real account's usage, and a real sign-in file is
a sign-in. Nothing on this page is copied from documentation without a matching observation
on a real machine.

Rules that follow from this table, and that the tests enforce:

1. A parser reads **only** the fields in its "fields used" column. It builds a new value
   out of them rather than filtering a parsed line, so a field that is not on the list is
   gone with the parse result.
2. No prompt text, no reasoning, no command output, no identifier is ever carried out of a
   log. Two strings leave the Codex parser — a plan name and a timestamp — and both have
   to pass a shape check first.
3. A missing field means **unknown**, and unknown is rendered as unknown. `percent` is
   omitted entirely rather than written as `0`; see rule 2 of
   [`limits-contract.md`](limits-contract.md).
4. Each row's fixture is its regression test. A format that moves under us changes the
   fixture and the "version observed" column in the same commit.

## Inventory

| Path | Fields used | Version observed | Fixture | Notes |
|---|---|---|---|---|
| `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ISO>-<uuid>[_<uuid>].jsonl` | `timestamp`; `payload.rate_limits.{plan_type, primary, secondary}`; and inside each window `{used_percent, window_minutes, resets_at}` — **seven values, and nothing else** | Codex 0.153.4 | `fixtures/codex/rollout-sample.jsonl`, `rollout-premium-null.jsonl`, `rollout-no-rate-limits.jsonl`, `rollout-malformed.jsonl` | See the section below. |
| Claude Code status-line payload (stdin JSON handed to `statusLine.command`) | `session_id` (as a file name); `rate_limits.{five_hour, seven_day}.{used_percentage, resets_at}` — **four numbers reach `limits.json`, and nothing else** | Claude Code 2.1.263 | `fixtures/claude/statusline-payload.json`, `statusline-payload-both-windows.json`, `statusline-payload-no-rate-limits.json` | See "The status-line payload" below. |
| `<CLAUDE_CONFIG_DIR or ~/.claude>/settings.json` | **`statusLine` only**, read and written. Every other key is parsed as an opaque value and written back unchanged. | Claude Code 2.1.263 | `fixtures/claude/settings-no-statusline.json`, `settings-ccstatusline.json`, `settings-custom-node.json` | The one file this product writes on someone else's behalf. See "Claude Code's settings file" below. |
| `GET https://api.anthropic.com/api/oauth/usage` | `limits[]`, and inside each entry `{kind, percent, resets_at, scope.model.display_name}`; or the older top-level `five_hour`/`seven_day` `{utilization, resets_at}` | observed live 2026-09-07 | none — no fixture may hold a real response | **Opt-in, off by default.** See "The usage endpoint" below. |
| `<CLAUDE_CONFIG_DIR or ~/.claude>/.credentials.json` | `claudeAiOauth.{accessToken, expiresAt, rateLimitTier, subscriptionType}` — **four values, and the token is never stored** | Claude Code 2.1.263 | none — and there never will be one | **Opt-in, off by default.** Read-only, never written. See "The sign-in file" below. |
| `~/.nazar/limits.lock` | `{schemaVersion, pid, startedAt, heartbeatAt}` — **ours**, read and written | this build | none needed; `lock.rs`'s tests are the fixture | The advisory file that makes "one writer" true. See "The advisory lock" below. |
| `~/.nazar/tray.request` | **existence and modification time only.** The contents are one timestamp, for a human reading the directory | this build | none | How a second launch reaches the running tray. See "The request marker" below. |
| `%APPDATA%\nazar\config.json` | **ours**, read and written; every key the settings page owns, and unknown keys preserved verbatim | this build | none needed; `config.rs`'s tests are the fixture | The user's settings. See "The settings file" below. |
| `%APPDATA%\nazar\alerts.json` | **ours**, read and written: `{schemaVersion, windows{<provider>/<window>{resetsAt, fired[]}}}` | this build | none needed; `alerts.rs`'s tests are the fixture | Which threshold notifications have already been shown. See "The notification log" below. |

## The Codex rollout log

`CODEX_HOME` moves Codex's home directory; when it is unset the default is `~/.codex`
(`%USERPROFILE%\.codex` on Windows). Session logs sit under `sessions/` in a
zero-padded `YYYY/MM/DD` tree, so sorting the directory names as text sorts them as dates.

### The line

Every line of a rollout log is one JSON object with four keys: `timestamp` (string),
`ordinal` (integer), `type` (string) and `payload` (object). Quota lines are
`type: "event_msg"` with `payload.type: "token_count"`, and their payload has three keys:
`type`, `info` (token counters and the context-window size) and `rate_limits`.

```json
{"timestamp":"2026-09-06T22:55:05.479Z","ordinal":409,"type":"event_msg",
 "payload":{"type":"token_count","info":{ … },"rate_limits":{
    "limit_id":"codex","limit_name":null,"plan_type":"plus",
    "primary":  {"used_percent":54.0,"window_minutes":300,  "resets_at":1788751044},
    "secondary":{"used_percent":70.0,"window_minutes":10080,"resets_at":1788783892},
    "credits":{"has_credits":false,"unlimited":false,"balance":"0"},
    "individual_limit":null,"spend_control_reached":null,"rate_limit_reached_type":null}}}
```

### Verified field types

| Field | Type | Observed | Notes |
|---|---|---|---|
| `timestamp` (line level) | string | 329/329, all 24 characters, `…Z` | ISO 8601 in **UTC** with milliseconds. This is `sourceAt`. |
| `rate_limits` | object | 329/329, always the same nine keys | Key set identical across all 18 logs and 12 days. |
| `rate_limits.plan_type` | string | `plus` 329/329 | Copied through as reported, never normalised. `limits.json` shows `"plan": "plus"`. |
| `rate_limits.limit_id` | string | `codex` 326, **`premium` 3** | Not read. See "The `premium` variant" below. |
| `rate_limits.limit_name` | null | 329/329 | Not read. |
| `rate_limits.primary` / `.secondary` | object **or `null`** | object 652, `null` 6 | The five-hour and weekly windows. Both are `null` on the three `premium` lines. |
| `…primary.used_percent` | **float** | 652/652 float, range 0.0 – 98.0 | Always a float here; the parser also accepts an integer. |
| `…primary.window_minutes` | integer | `300` 326, `10080` 326 | 300 on `primary`, 10080 on `secondary`, without exception. |
| `…primary.resets_at` | **integer, Unix seconds, UTC** | 652/652, range 1787866049 – 1788783894 | **Confirmed against a second source:** the weekly window's `resets_at` of 1788783892 is `2026-09-07T12:24:52Z`, the same second the official usage endpoint reported for the same window in an independent fetch. Milliseconds would have put it in 1970. The parser still reads a value above 10¹² as milliseconds, so a future switch degrades instead of breaking. **Written into `limits.json` rounded down to the whole minute** (T-WP10), the same gate the Claude reader has been through since T-WP9, so the two providers spell one instant one way: this window becomes `2026-09-07T12:24:00Z`. Down, never to the nearest — a reset is a deadline. |
| `rate_limits.credits` | object with `has_credits`, `unlimited`, `balance` (string) | 329/329 | Not read. |
| `rate_limits.{individual_limit, spend_control_reached, rate_limit_reached_type}` | null | 329/329 | Not read. **`rate_limit_reached_type` is worth watching**: it is the field a future "you are out of quota" flag would appear in, and it has never been non-null here. |

Line endings are `LF` and every file ends with a newline.

### How often, and where

A quota line appears **dozens of times per session**: 13 of 118 lines, 42 of 307, 5 of 61
across three sampled logs. That is what makes the passive design work — the tray never
has to ask anyone for a number, it just reads the last one Codex was handed.

Two facts shape the reader, and both would have produced a wrong answer if assumed away:

- **The newest quota line sits close to the end**: 1 399 – 1 780 bytes from EOF in the
  four newest logs, in files of 0.3 – 13 MB. So the tail opens a 256 KiB window at the end
  of the file, not the file. A widened pass (64 MiB, bounded) runs only when that window
  turned up nothing.
- **3 of 18 logs carry no `rate_limits` line at all** — three- and nine-line stubs from
  sessions that ended before the first response — and **4 of 10 date directories hold no
  rollout file at all**. So the reader takes a short list of candidates (five, newest
  first by modification time) instead of one path, and an empty date directory does not
  end the walk.

### The `premium` variant

Three of the 329 quota lines carry `limit_id: "premium"` with **both windows `null`**:

```json
{"limit_id":"premium","plan_type":"plus","primary":null,"secondary":null, …}
```

On this machine such a line was the **last** quota line in two of the logs. A reader that
takes "the newest line carrying `rate_limits`" gets a payload with no windows in it, and
would blank a display that had two perfectly good numbers a few lines earlier.

The reader therefore takes the newest line with **at least one non-null window**, and does
not filter on `limit_id` — the id is a name we do not control, and a rule written around
it would break the day it is renamed. This is pinned by `rollout-premium-null.jsonl`.

### Never read from a rollout line

The parser names seven values and constructs a new object out of them. Everything below
appears in real logs on this machine and **none of it ever leaves the parser**:

- `payload.info` in full: `last_token_usage`, `total_token_usage`, `model_context_window`.
- `payload.thread_token_usage`, `turn_token_usage`, `usage`, and the ids beside them:
  `response_id`, `turn_id`, `root_turn_id`, `session_id`, `thread_id`.
- `session_meta` in full: `session_id`, `id`, `cwd`, `cli_version`, `originator`,
  `model_provider`, `source`, `thread_source`, `context_window.window_id`,
  `history_base.*`.
- `turn_context` and `thread_settings_applied`: `model`, `effort`, `cwd`,
  `approval_policy`, `sandbox_policy`, `personality`, `workspace_roots`,
  `collaboration_mode`, `service_tier`.
- Every content and tool field: `response_item/message`, `response_item/reasoning`,
  `custom_tool_call.input`, `custom_tool_call_output`, and `item_completed.item.*`
  (`command`, `cwd`, `stdout`, `stderr`, `aggregated_output`, `parsed_cmd`, `changes`).
- `rate_limits.{limit_id, limit_name, credits, individual_limit, spend_control_reached,
  rate_limit_reached_type}`.

Two mechanical guards back the list. The two strings that *are* copied go through an
allow-list first — a plan name must be a short ASCII identifier, a timestamp must have the
shape of RFC 3339 — so a field repurposed to hold prose is dropped rather than forwarded.
And a leak test builds a line whose every text field carries a sentinel and fails if the
sentinel appears anywhere in the parser's output or in the serialised provider block.

### Not read, on purpose

| Path | Why not |
|---|---|
| `$CODEX_HOME/auth.json` | Sign-in material. **Never opened.** The passive design has no use for it: the numbers are in the logs. The retired prototype read this file to call an endpoint, and got an HTTP 404 out of it on 2026-09-03 when an account id was missing — a documented failure of exactly the approach this one replaces. |
| `<CLAUDE_CONFIG_DIR or ~/.claude>/.credentials.json` **on the default path** | Sign-in material. The opt-in detailed-windows mode is the single sanctioned exception and it is a switch the user turns; with it off this file is not opened, and a test poisons it to prove that. `refreshToken`, `refreshTokenExpiresAt` and `scopes` are never read even with the mode on. |
| `$CODEX_HOME/*.sqlite`, `*.sqlite-wal`, `*.sqlite-shm` | Conversation history, memories, queues, logs. Not quota, and not ours to open. |
| `$CODEX_HOME/session_index.jsonl` | Thread names and ids. Useful to Nazar's canvas; identity to a quota tray, so it stays unread here. |
| `$CODEX_HOME/{archived_sessions,attachments,dictation-history,transcription-history.jsonl,generated_images,…}` | The user's content. None of it is quota. |
| `~/.claude/projects/**` | The retired prototype read up to 400 transcript files to *estimate* usage when a fetch failed. Dropped: no estimates, only reported numbers (plan section 5). |

**The gate:** `crates/nazar-core/tests/hygiene.rs` greps every `.rs` file in the workspace
for the name of a credential file and for token-shaped identifiers, and fails the build on
a hit. It assembles the needles at run time so it does not match itself, which is also why
the file names above live in this document and not in a comment in the source.

**The gate's one exception**, added in WP2b: files under
`crates/nazar-core/src/claude/detailed/` are skipped. It is an allow-listed **directory**
rather than a dropped needle, so the exception cannot spread — a new file that wants
sign-in material has to be created in one specific place, next to the tests that keep it
honest. Three more tests hold that place down: the directory must exist and must still trip
a needle (otherwise the allow-list is guarding nothing and should be deleted), nothing in
it may contain a printing macro, and `Secret::expose_for_one_request` must have exactly one
call site in shipping code.

## The advisory lock

`~/.nazar/limits.lock` is the only file on this page that belongs to nazar-tray itself. It
is listed here anyway, because Nazar reads `~/.nazar` too and a file that appears next to
`limits.json` should be documented rather than discovered.

```json
{
  "schemaVersion": 1,
  "pid": 24184,
  "startedAt": "2026-09-07T09:12:04Z",
  "heartbeatAt": "2026-09-07T09:18:04Z"
}
```

| Field | Meaning |
|---|---|
| `schemaVersion` | `1`. Unknown keys are preserved through a heartbeat, like everywhere else. |
| `pid` | The holder's process id. **A diagnostic, never a liveness proof** — see below. |
| `startedAt` | When the holder took the lock. Together with `pid` it is how a holder recognises its own record. |
| `heartbeatAt` | When the holder last said it was still there. Rewritten on **every** refresh, roughly once a minute, whether or not the numbers changed. |

**Acquisition is one operation, not two.** `OpenOptions::create_new` either creates the file
or fails because it exists; of two processes racing for it exactly one wins. Finding B01 of
the code audit was a lock whose check and whose write were separate, and the logs caught two
refreshers passing that check four seconds apart — which is what produced the observed
HTTP 429.

**Liveness is the heartbeat, not the process id.** There is no portable way to ask the
operating system whether a given process is running, and buying one would cost a platform
crate in the crate that is meant to be boring. A heartbeat costs nothing extra — the holder
is awake every sixty seconds anyway — and it covers a case a process-id check cannot: a
holder that is still running but wedged stops beating and is replaced, where a liveness
probe would wait for ever on a process that will never write again. A record whose heartbeat
is more than **five minutes** old (five missed ticks) may be deleted and retaken.

**An unreadable lock file is respected until it ages out.** Between `create_new` and the
record being written there is a moment when the file is empty; a competitor that read it then
must wait rather than evict a holder that is one instruction from being alive. So a file
whose contents are not a record is judged by its own modification time, against the same five
minutes.

**A holder verifies before it writes.** The heartbeat re-reads the file first and reports
whether it still names this process. A tray whose lock was reclaimed while its machine slept
finds out at the top of its next refresh and stops writing — before the write, not after it.

**Consequence worth knowing:** a tray that is *killed* rather than quit does not release its
lock, so a relaunch within five minutes defers to a process that is gone. WP4's `Quit` is the
graceful exit that avoids it; when a tray really has been killed, deleting
`~/.nazar/limits.lock` by hand is the manual escape, and doing so is safe when no tray is
running.

## The request marker

`~/.nazar/tray.request` is how a second launch of the application reaches the first one.
Two processes cannot share a window, and every portable channel between them — a socket, a
named pipe, D-Bus, a platform single-instance plugin — is heavier than the problem. So the
second launch writes a marker into the directory the running tray is already watching, and
exits; the tray notices it within five seconds, deletes it, and opens its panel.

```json
{
  "requestedAt": "2026-09-07T09:18:31Z"
}
```

Only two things about the file are ever read: **that it exists**, and **when it was last
written**. The timestamp inside is for a human looking at the directory. A marker older than
sixty seconds is swept up without being obeyed — one left behind by a crash is litter, not an
instruction, and a panel opening days later because of it would be a small haunting.

## The settings file

`%APPDATA%\nazar\config.json` on Windows, `$XDG_CONFIG_HOME/nazar/config.json` or
`~/.config/nazar/config.json` elsewhere, and `$NAZAR_HOME/config.json` when that override is
set. It is the user's file: they may edit it by hand, and this application must not lose what
they wrote.

```json
{
  "schemaVersion": 1,
  "detailedWindows": false,
  "detailedSuggested": false,
  "thresholds": { "warn": 60.0, "critical": 85.0, "exhausted": 100.0 },
  "freshness": { "freshMinutes": 5, "agingMinutes": 45 },
  "theme": "nazar",
  "themeMode": "system",
  "firstRunHintDismissed": false,
  "notifications": true,
  "quietHours": { "from": "22:00", "to": "07:00" },
  "providers": { "claude": true, "codex": true }
}
```

| Field | Meaning |
|---|---|
| `schemaVersion` | `1`. Separate from `limits.json`'s: this file is the user's, that one is a contract with another program. |
| `detailedWindows` | Whether the opt-in endpoint mode is on. **`false` ships, and it is the only value a fresh machine has.** |
| `detailedSuggested` | Whether the Max-plan offer has already been made. Set when it is shown, so a user who said no is not asked twice. |
| `thresholds` | Where a window turns amber, red and spent — and where a notification fires. One set of numbers, read by the icon, the panel and the notifications (finding B14). |
| `freshness` | When a reading stops being fresh (`5` min) and starts being stale (`45` min). |
| `theme`, `themeMode` | `nazar` \| `graphite`, and `system` \| `light` \| `dark`. |
| `firstRunHintDismissed` | Whether the Windows 11 overflow tip has been read. |
| `locale` | **Absent** means "follow the system". Present, it is one of `en tr zh ko ru es`. |
| `notifications` | The master switch above the thresholds and the quiet hours. |
| `quietHours` | `HH:MM` **local** wall-clock times; the range wraps midnight when `to` is earlier than `from`, and equal endpoints are an empty range. Absent means there are none. |
| `providers` | Which providers are read at all. A provider switched off has **no reader built for it**: its files are never opened. |

Three rules, and each is a way a settings file gets lost:

* **Unknown keys survive.** A key a newer build wrote is read back and written out unchanged,
  so running an older tray once does not throw the user's settings away. The settings page
  writes the keys it owns into the file *as it is on disk*, rather than replacing it with what
  the application happens to hold.
* **A damaged file is reported, never replaced.** The user wrote it, and overwriting it with
  the defaults would lose their settings to a stray comma.
* **Reading is forgiving; writing is strict.** Whatever is on disk is used as best it can be —
  thresholds that do not ascend still colour the icon, a theme nobody has heard of falls back
  to `nazar` — because a user whose file has a bad number still has to be able to open the
  form that fixes it. `Config::validate` is the gate that form goes through, and it refuses
  thresholds that do not strictly ascend inside `0 < value <= 100`, a language this build
  cannot paint, a mode that is not one of the three, a quiet-hours endpoint that is not an
  `HH:MM` time, and freshness rules that do not ascend.

**The startup entry is deliberately not in here.** "Start with Windows" lives in
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, written by `tauri-plugin-autostart`,
and the settings switch reads its state back from there. A copy in this file would drift the
first time somebody used Task Manager's Startup tab.

## The notification log

`%APPDATA%\nazar\alerts.json`, beside the settings rather than in `~/.nazar`: no consumer
reads it, it is not part of the contract with Nazar, and a machine that has never crossed a
threshold does not have one.

```json
{
  "schemaVersion": 1,
  "windows": {
    "codex/secondary": {
      "resetsAt": "2026-09-07T12:24:00Z",
      "fired": [60.0]
    }
  }
}
```

| Field | Meaning |
|---|---|
| `schemaVersion` | `1`. Unknown keys are preserved through a rewrite, like everywhere else. |
| key | `"<provider>/<window>"`. Neither half contains a slash in the `limits.json` contract, so a human reading the file can always tell them apart. |
| `resetsAt` | The reset this record belongs to, copied from the window. A **different** one is a new period: the fired set is cleared and the remembered percentage forgotten. |
| `fired` | The thresholds already consumed in this period, ascending. |

**This file is the difference between a warning and a nuisance.** Without it, restarting the
tray would re-fire every threshold the user is already above. With it, the key
`(provider, window, threshold, resetsAt)` is written before the toast is shown, so a process
that dies a second later still does not repeat itself.

**A record is only written when something fires.** A window sitting quietly at 10 % has no
entry at all, and a record whose period has turned over with nothing fired in the new one is
removed rather than left as an empty shell. Records for windows nobody reports any more are
swept up **48 hours after their reset** — long enough that a provider which could not be read
for one refresh keeps its bookkeeping, short enough that the file does not grow for ever.

**A damaged file is treated as an empty one**, and this is the one place in the product where
that is the right answer rather than the wrong one: the file is ours, nothing in it can be
recovered by hand, and the cost of the wrong guess here is one extra toast where the cost of
the other wrong guess is silence.

## The startup entry

Written by `tauri-plugin-autostart` 2.5.1 through `auto-launch` 0.5.0, and listed here
because it is a change this application makes outside its own directories.

| Key | Value |
|---|---|
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` | `nazar-tray` = `<path to nazar-tray.exe> --hidden` (`REG_SZ`) |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run` | `nazar-tray` = `02 00 …` (`REG_BINARY`), which is what Task Manager's Startup tab reads as *Enabled* |

Observed live on 2026-09-07 by turning the switch on and off again on the maintainer's own
machine. **Two things the plugin does not do**, both of which belong to WP7's uninstaller:

* Disabling removes the `Run` value but **leaves the `StartupApproved` value behind**. It is
  inert — Task Manager only lists entries that exist in `Run` — but it is residue, and an
  uninstall should take it with it.
* The `Run` value is written **unquoted**: `C:\Program Files\nazar-tray\nazar-tray.exe
  --hidden`. Windows resolves that by trying each space-delimited prefix, so it works, but a
  quoted path would not depend on that.

## The usage endpoint

Read only while the opt-in detailed-windows mode is on;
[`detailed-windows.md`](detailed-windows.md) is the whole story, including the
terms-of-service note. This section is the format.

### The observed shape

`GET https://api.anthropic.com/api/oauth/usage`, observed live 2026-09-07 on a Max 20x
account. The endpoint is **undocumented**: it is the one Claude Code's own `/usage` panel
calls, and it can change without notice, which is why the mapper skips what it does not
recognise instead of guessing.

```json
{"limits":[
   {"kind":"session",       "percent":2,  "resets_at":"2026-09-07T13:10:00.130195+00:00","is_active":true},
   {"kind":"weekly_all",    "percent":38, "resets_at":"2026-09-12T02:00:00.130216+00:00"},
   {"kind":"weekly_scoped", "percent":30, "resets_at":"2026-09-12T02:00:00.130399+00:00",
    "scope":{"model":{"display_name":"Fable"}}}],
 "extra_usage":{"is_enabled":false,"utilization":0}}
```

| Field | Type | Notes |
|---|---|---|
| `limits` | array | Three entries on this account. An entry with an unknown `kind` is **skipped**: a window whose meaning is unknown cannot be given a length, and a window without a length is worse than absent. |
| `…kind` | string | `session`, `weekly_all`, `weekly_scoped`, and the older `opus`. The mapping to window keys is in [`detailed-windows.md`](detailed-windows.md). |
| `…percent` | number | Integer in every observation. Accepted only in `0..=100`; outside that the window is dropped rather than clamped, because clamping would invent a number. |
| `…resets_at` | **string, RFC 3339 with microseconds and `+00:00`** | **Not** the encoding the status line uses for the same idea, which is Unix seconds. Both are rewritten to the contract's `…Z` with whole seconds before they reach `limits.json`; the observed string above is pinned as a test in `timefmt.rs`. |
| `…scope.model.display_name` | string | `Fable`. Becomes the window's `model` and, slugged, the second half of its key (`seven_day_fable`). Dropped if it is not a name — bounded, no control characters, no path separators, at least one alphanumeric. |
| `…is_active` | boolean | **Not read.** The binding window is computed as the highest percentage across all windows; see finding B04/B16 and the note in `detailed-windows.md`. |
| `extra_usage` | object | **Not read.** `limits.json` has no field for paid overflow usage. |
| older top-level `five_hour`/`seven_day` | object with `utilization`, `resets_at` | Read **only** when `limits[]` produced nothing, never merged with it. Not present in the 2026-09-07 observation; kept because the retired prototype carried the fallback and dropping it would be a regression the day the array changes name. |

No fixture pins this row, and that is deliberate: a captured response is a real account's
usage, and there is no version of it that is safe to commit. The mapper is tested against
hand-written bodies covering every `kind`, both shapes, the duplicate-model case, the
unnamed-scope case and six bodies it must refuse.

## The sign-in file

`CLAUDE_CONFIG_DIR` moves the directory; the default is `~/.claude`. Read only while the
opt-in mode is on, **read-only always** — nothing in this product creates, writes, renames
or deletes it.

Observed 2026-09-07, Claude Code 2.1.263, 509 bytes:

```json
{"claudeAiOauth":{
  "accessToken":"…","refreshToken":"…",
  "expiresAt":1788785223254,"refreshTokenExpiresAt":1791150214254,
  "scopes":["…"],"subscriptionType":"max","rateLimitTier":"default_claude_max_20x"}}
```

| Field | Read? | Why |
|---|---|---|
| `claudeAiOauth.accessToken` | yes | One `Authorization` header, in a wiping wrapper, then gone. **Never stored.** |
| `claudeAiOauth.expiresAt` | yes | **Milliseconds** since the epoch — unlike every other timestamp either source writes, which is why the reader decides by magnitude. An expiry that has passed means no request is sent at all. |
| `claudeAiOauth.rateLimitTier` | yes | `default_claude_max_20x` → `plan: "max_20x"`. |
| `claudeAiOauth.subscriptionType` | yes | `max`. The fallback when there is no tier. |
| `claudeAiOauth.refreshToken`, `refreshTokenExpiresAt` | **no** | Refreshing a token is Claude Code's job. A tool that could do it would be one that intermediates a sign-in. |
| `claudeAiOauth.scopes` | **no** | Not quota. |

The file's **size and modification time** are also read, and nothing else about it: they
answer "did Claude Code rewrite this", which clears the backoff (so a refreshed token is
tried at once rather than waited out) and drops remembered numbers when the plan changed
underneath them. Neither is a hash of the contents and neither is derived from the token.

## The status-line payload

Claude Code runs `statusLine.command` on every redraw and writes one JSON object to its
standard input. `nazar-statusline` is installed as that command; what it does with the
payload, and where it puts it, is [`statusline-wrapper.md`](statusline-wrapper.md). This
section is the format itself.

### The observed shape

Captured on the maintainer's machine, Claude Code 2.1.263, 2026-09-07. Twenty top-level
keys:

```json
{"session_id":"…","session_name":"…","prompt_id":"…","transcript_path":"…",
 "scratchpad_dir":"…","cwd":"…",
 "workspace":{"current_dir":"…","project_dir":"…","added_dirs":[],
              "repo":{"host":"…","owner":"…","name":"…"}},
 "model":{"id":"claude-fable-5-1","display_name":"Fable 5.1"},
 "effort":{"level":"high"},"version":"2.1.263","output_style":{"name":"default"},
 "cost":{"total_cost_usd":…,"total_duration_ms":…,"total_api_duration_ms":…,
         "total_lines_added":…,"total_lines_removed":…},
 "context_window":{"total_input_tokens":…,"total_output_tokens":…,
                   "context_window_size":1000000,"current_usage":{…},
                   "used_percentage":16,"remaining_percentage":84},
 "exceeds_200k_tokens":false,"prompt_cache":{…},"fast_mode":false,
 "thinking":{"enabled":true},
 "rate_limits":{"five_hour":{"used_percentage":12,"resets_at":1788768000},
                "seven_day":{"used_percentage":31,"resets_at":1789178400}}}
```

| Field | Type | Notes |
|---|---|---|
| `session_id` | string (UUID) | Used as the capture's **file name**, after a character check. Never copied into `limits.json`. |
| `rate_limits` | object, **often absent** | Present for Pro and Max subscribers, and only after the session's first API response. Absent is a documented state, not a fault. |
| `rate_limits.five_hour` / `.seven_day` | object, **either can be absent** | Claude Code drops a window once its `resets_at` has passed. The fixture captured live has only `seven_day` for exactly that reason. |
| `…used_percentage` | number | Integer in every observation; the parser accepts a float too. Copied through unrounded. |
| `…resets_at` | **integer, Unix seconds, UTC** | Same encoding as Codex's. A value above 10¹² is read as milliseconds, so a future switch degrades instead of breaking. |
| `model.id`, `model.display_name`, `version` | string | **Not a plan name.** No plan is derived from them; see below. |
| everything else | — | Read by nobody. It is captured, because the capture is the whole payload, and it never reaches `limits.json`. |

Documented but not present in this payload, and therefore not depended on: `vim.mode`,
`agent.name`, `pr.*`, `worktree.*`, `workspace.git_worktree`, `rate_limits.spend_limit`.

### Never taken out of a capture

`limits.json` gets **two percentages, two reset times** and — if a payload ever carries an
explicit `plan`, `plan_type` or `subscription_type` — a plan name that has passed the same
identifier check the Codex reader uses. Nothing else. In particular:

| Not read | Why |
|---|---|
| `cwd`, `transcript_path`, `scratchpad_dir`, `workspace.*` | Paths. They identify a machine and a project, and `limits.json` is meant to be safe to paste into a bug report. |
| `session_id`, `prompt_id`, `session_name` | Identity. The session id names a file inside the user's own home directory and goes no further. |
| `cost.*`, `context_window.*`, `prompt_cache.*` | Real data, and Nazar's to use — from the capture file, not from `limits.json`. The contract has no field for them and is not growing one. |
| `model.*`, `version`, `effort`, `output_style` | Interesting, not quota. `model.display_name` and `effort.level` are printed by the wrapper's own minimal status line and are never written to a file. |

A plan name is **not** derived from `model.id` or `version`. "This user is on Fable 5.1"
does not mean "this user is on Max", and inventing the second from the first is exactly
what rule 2 of [`limits-contract.md`](limits-contract.md) forbids. `providers.claude.plan`
is therefore absent on the passive path.

The gate: `nothing_but_the_allow_listed_values_leaves_the_reader` builds a capture whose
every string is a sentinel and fails if the sentinel appears in the reading or in the
serialised provider block.

## Claude Code's settings file

`CLAUDE_CONFIG_DIR` moves the directory; the default is `~/.claude`. `settings.json` is a
JSON object with a flat top level. The installer reads it and writes it back, and the only
key it understands is `statusLine`:

```json
"statusLine": { "type": "command", "command": "…", "padding": 0, "refreshInterval": 30 }
```

| Field | Behaviour on install |
|---|---|
| `type` | Set to `"command"`. |
| `command` | Set to this program's absolute path, quoted if it contains a space. |
| `padding`, `refreshInterval` | **Preserved** if present, absent if not. |
| any other key inside `statusLine` | **Preserved.** A key a newer Claude Code adds is the user's, not ours to drop. |
| every other key in the document | **Preserved, in order.** `serde_json`'s `preserve_order` is on for the whole workspace, and removal uses a shifting remove rather than the swapping one, which would silently reorder the file. |

The three fixtures are the three shapes the acceptance criteria name: no `statusLine` at
all, ccstatusline's `{"type":"command","command":"npx ccstatusline@latest"}`, and the
maintainer's own shape (an interpreter, a quoted absolute path, `padding`,
`refreshInterval`) with the path replaced. Each is installed, diffed and uninstalled in
`crates/nazar-statusline/tests/installer.rs`, and the result is compared to the original
**byte for byte** — all three round-trip exactly.

Two things this file is never allowed to become: a file that was parsed loosely (invalid
JSON is reported and left alone, never "repaired" and never replaced by `{}`), and a file
that was written without a copy of the original beside it. Backups are named
`settings.json.nazar-bak-<stamp>` and are never written over.

## Fixture sanitising

`fixtures/codex/*.jsonl` are real lines from the maintainer's own logs. What was kept and
what was replaced:

| Kept verbatim | Replaced with a placeholder |
|---|---|
| Every number: `used_percent`, `window_minutes`, `resets_at`, `ordinal`, token counters. They are usage figures, not identity. | Every other string: `cwd`, `session_id`, `thread_id`, ids, model names, prompts, command lines, output → `"[redacted]"`. |
| `timestamp`, `type`, `payload.type` — the entry discriminators the parser dispatches on. | |
| `plan_type` (`plus`) and `limit_id` (`codex`, `premium`) — tier names, published in the plan already, and the point of the `premium` fixture. | |

`fixtures/claude/*.json` were sanitised the same way. The payload fixture is a real
status-line payload with every number kept — the cost, the token counters, the two quota
figures — and every path, id and repository name replaced. The settings fixtures keep the
**shape** of the maintainer's file and nothing else: the same keys in the same order, with
`node "C:\projects\example\.claude\scripts\statusline.js"` standing in for the real path.

Two gates in `tests/hygiene.rs`, one per family:
`the_codex_fixtures_carry_nothing_that_identifies_a_machine` and
`the_claude_fixtures_carry_nothing_that_identifies_a_machine`. Both fail on a `~`, a
home-directory path, an e-mail address, a run of 32 hexadecimal characters, or the name of
whoever is running the tests — read from the environment so that the name itself never
enters the repository. The Claude gate adds two checks of its own: every payload fixture
has to still be a payload (and every settings fixture a settings file), and no payload
fixture may contain a sign-in or account field, which is the claim the whole passive
design rests on.

## Times are written in UTC

`resetsAt`, `sourceAt` and `updatedAt` are RFC 3339 with a `Z`. The standard library has
no time-zone database and no way to ask the operating system for the current offset, so a
local offset would cost either a runtime dependency or hand-written daylight-saving code
in the one crate that is supposed to be boring. A UTC timestamp names the same instant, a
countdown — which is what the tray shows — is offset-independent, and every consumer of
`limits.json` can render local time for free (the panel is JavaScript, where it is one
call). If a future package needs a local offset in the file itself, that is a decision to
take with a dependency in front of it, not a default to drift into.
