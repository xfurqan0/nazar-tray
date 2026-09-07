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
own `settings.json`. Nothing on this page is copied from documentation without a matching
observation on a real machine.

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
| Claude Code usage endpoint | model-scoped weekly windows | not read yet | none yet | **WP2b, opt-in and off by default.** The only path in the whole product that reads a token, held in memory for one request. It will live behind its own switch and its own tests; the credential gate below grows an explicit allow-list for it rather than losing a needle. |

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
| `…primary.resets_at` | **integer, Unix seconds, UTC** | 652/652, range 1787866049 – 1788783894 | **Confirmed against a second source:** the weekly window's `resets_at` of 1788783892 is `2026-09-07T12:24:52Z`, the same second the official usage endpoint reported for the same window in an independent fetch. Milliseconds would have put it in 1970. The parser still reads a value above 10¹² as milliseconds, so a future switch degrades instead of breaking. |
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
| `~/.claude/.credentials.json` | Same, for the other provider. WP2b's opt-in mode is the single sanctioned exception, off by default, in memory for one request, never written and never logged. |
| `$CODEX_HOME/*.sqlite`, `*.sqlite-wal`, `*.sqlite-shm` | Conversation history, memories, queues, logs. Not quota, and not ours to open. |
| `$CODEX_HOME/session_index.jsonl` | Thread names and ids. Useful to Nazar's canvas; identity to a quota tray, so it stays unread here. |
| `$CODEX_HOME/{archived_sessions,attachments,dictation-history,transcription-history.jsonl,generated_images,…}` | The user's content. None of it is quota. |
| `~/.claude/projects/**` | The retired prototype read up to 400 transcript files to *estimate* usage when a fetch failed. Dropped: no estimates, only reported numbers (plan section 5). |

**The gate:** `crates/nazar-core/tests/hygiene.rs` greps every `.rs` file in the workspace
for the name of a credential file and for token-shaped identifiers, and fails the build on
a hit. It assembles the needles at run time so it does not match itself, which is also why
the file names above live in this document and not in a comment in the source.

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
