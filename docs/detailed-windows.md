# Detailed windows — the opt-in mode

**Off by default. With it off, nazar-tray opens no credential file and makes no network
request, and there is no code path that would.** This page is what it does when you turn it
on, why it exists, and what you are agreeing to.

- Setting: `detailedWindows` in `%APPDATA%\nazar\config.json` (`~/.config/nazar/config.json`
  elsewhere; `$NAZAR_HOME/config.json` when that override is set).
- One run without changing the setting: `nazar-tray --print --detailed`.
- Implemented by `crates/nazar-core/src/claude/detailed/`, behind the `detailed-windows`
  cargo feature as well as the runtime flag.

## Why it exists

Claude Code's status line reports two windows: the five-hour one and **one global** weekly
one. A Max subscriber also has **model-scoped** weekly caps, and those are reported by the
official usage endpoint and nowhere else.

Measured on the maintainer's machine on 2026-09-07 at 03:05, which is the whole reason this
work package exists:

| Window | Status line (passive) | Usage endpoint |
|---|---|---|
| weekly, all models | 18 % | 18 % |
| weekly, Fable only | **not reported** | **23 %** |

A tray built only on the passive path would have said 18 % to somebody whose actual
constraint was 23 % and rising faster. That is the failure mode a quota tray exists to
prevent, so the mode exists — as a switch, not as a default.

On **Pro** the endpoint reports the same two windows the status line already reports, so
there is nothing to gain and a token to read. The app therefore only ever offers the mode
on a Max plan.

## What it reads, and when

Only while the mode is on, and only as part of a refresh:

| File | Fields read | Written? |
|---|---|---|
| `<CLAUDE_CONFIG_DIR or ~/.claude>/.credentials.json` | `claudeAiOauth.accessToken`, `.expiresAt`, `.rateLimitTier`, `.subscriptionType` | **never** — opened read-only, never created, never renamed |
| `GET https://api.anthropic.com/api/oauth/usage` | `limits[]`, or the older top-level `five_hour`/`seven_day` | — |

`refreshToken`, `refreshTokenExpiresAt` and `scopes` are in that file and are **not read**.
Refreshing a token is Claude Code's job. A tool that could do it would be a tool that
intermediates a sign-in, which is the line this product does not cross — see the terms note
below.

The request carries four headers and nothing else:

```
Authorization: Bearer <token>
anthropic-beta: oauth-2025-04-20
Accept: application/json
User-Agent: nazar-tray/<version>
```

Twenty-second budget, no redirects followed, response body read up to 256 KiB.

## What happens to the token

1. The file is read into a string, parsed, and the token is copied into a `Secret`.
2. The file string **and** the copy inside the parsed document are overwritten immediately.
3. The `Secret` has no `Display`, no `Clone`, no `Serialize`, and a `Debug` that prints
   `Secret(<redacted>)`. The characters come out through one method, named
   `expose_for_one_request`, and a test asserts that shipping code calls it in exactly one
   place: the line that builds the `Authorization` header.
4. When the request is over the `Secret` is dropped, and its `Drop` wipes the buffer.

It is never written to a file, never put in an error, never printed. Four tests hold that
down, and they are the acceptance criterion rather than a promise in a README:

- `the_token_reaches_the_header_and_nowhere_else` runs the whole flow with a sentinel where
  the token goes — settings written, sign-in read, request sent, answer mapped, block
  merged, `limits.json` written, then a second request that fails with a body echoing the
  sentinel back — and then fails if the sentinel is in **any** file under the temporary
  `NAZAR_HOME`, in any error's `Display` or `Debug`, in any `Debug` of the reader or the
  document, or in `limits.json`. It also asserts the sentinel *did* reach the mock server's
  `Authorization` header, so it cannot pass by not running.
- `the_token_is_exposed_in_one_place_in_the_shipping_code` is the grep described above.
- `the_opt_in_mode_contains_no_printing_at_all` fails if any file in the module contains
  `println!`, `eprintln!`, `eprint!` or `dbg!`. A token that never reaches a file can still
  reach a terminal.
- `no_source_file_outside_the_opt_in_mode_names_a_credential_file` keeps the exception
  inside one directory. It is an allow-listed **path**, not a dropped needle, so a second
  file that wants sign-in material has to be created in one specific place — next to these
  tests.

Best effort has a limit worth stating: an allocator can move a buffer and leave the old
bytes behind, and this code wipes only the buffers it owns. What it does guarantee is that
no copy is made anywhere it controls.

## What happens when the endpoint says no

The endpoint is undocumented and rate-limited. The retired prototype's own logs are the
design here: a 401 that lasted three hours produced seven identical requests, and two
schedulers racing each other produced a 429.

| Answer | What the user sees | What happens next |
|---|---|---|
| `200` | fresh windows, `state: "ok"` | nothing; the wait is reset to zero |
| `401` / `403` | last known windows, `state: "stale"`, `"token expired; run Claude Code once to refresh"` | wait, and clear the wait the moment the sign-in file changes |
| `429` | last known windows, `state: "stale"` | wait for `Retry-After` if the server sent one, else the doubling delay |
| `5xx`, anything else | last known windows, `state: "stale"`, `"the usage endpoint returned HTTP 503"` | doubling delay |
| no answer at all | last known windows, `state: "stale"`, one of five categories | doubling delay |
| `200` with a body this build cannot read | last known windows, `state: "stale"` | doubling delay |
| nothing known yet | the passive windows, exactly as they were | doubling delay |

The doubling delay is `1 s, 2 s, 4 s, 8 s …` capped at **30 minutes**, held in memory and
nowhere else. Three things shortcut it:

- **`Retry-After`** wins when the server sends one, up to the same 30-minute cap. (The
  prototype threw the response headers away and guessed twenty minutes — audit finding B07.)
- **A changed sign-in file** clears the wait entirely. The commonest reason it changes is
  Claude Code refreshing the token, which is exactly the event that makes a 401 stop being
  true. This is what ends the three-hour 401 storm (B06) instead of retrying through it.
- **A stored expiry that has already passed** means no request is sent at all: an expired
  token is a 401 that has not been posted yet.

Two rules about what a failure is allowed to say, both from the audit:

- **No response body ever reaches an error message** (B10). A failure carries a status code
  or one of five network categories, and nothing from the wire. The endpoint is
  undocumented; nobody knows what a future error body holds, and error text ends up in
  screenshots.
- **No absolute path either** (B11). "Claude Code is not signed in on this machine" says the
  same thing as the same sentence with a home directory in it, minus the user name in the
  screenshot.

And one about whose numbers they are: if the plan changes between two refreshes, the
remembered numbers are dropped rather than shown under the new plan's name (B25). If the
sign-in file disappears, they are dropped too — numbers from an account that is no longer
signed in are worse than none.

## What lands in `limits.json`

| `limits[].kind` | Window key | `windowMinutes` |
|---|---|---|
| `session` | `five_hour` | 300 |
| `weekly_all` | `seven_day` | 10080 |
| `weekly_scoped` | `seven_day_<model>` (`seven_day_fable`) | 10080 |
| `opus` (older name) | `seven_day_opus` | 10080 |
| anything else | *skipped* | — |

Every window this path produces carries `detailed: true`. The precedence rules for when
both paths report the same window are in
[`limits-contract.md`](limits-contract.md#which-source-wins-detailed-windows-mode); the
short version is that a **newer** status-line capture wins on `five_hour` and `seven_day`,
because the status line is rewritten every few seconds and the endpoint is asked on a timer.

`plan` comes from the sign-in file's hints, not from the response, which does not carry one:
`rateLimitTier` first (`default_claude_max_20x` → `max_20x`, `…max_5x` → `max_5x`, `…pro` →
`pro`), then `subscriptionType`. An unrecognised tier travels as written if it looks like an
identifier — a new tier name is more useful in the file than a shrug.

Two fields in the response are deliberately **not** read:

- **`is_active`**, which looks like the answer to "which window is binding" and is not the
  answer this product gives. The binding window is the highest percentage across every
  window, computed. The prototype invented that flag for Codex and drew the wrong window in
  bold (B04), and read it for Claude while its tray used a maximum, so its three displays
  disagreed (B16). One rule, computed, everywhere.
- **`extra_usage`**, whether paid overflow is switched on. Real, and interesting, and
  `limits.json` has no field for it.

## The terms-of-service note

Anthropic's terms forbid tools that "collect, store, or intermediate Claude.ai credentials
or session tokens". nazar-tray's default path is built to be plainly outside that: it reads
no credential file at all, and the numbers it shows are ones Claude Code already handed to
your own status line.

This mode is different, and it is different **because you turned it on**:

- It **reads** a token that is already on your disk, put there by Claude Code, using your
  own account, for a read-only request to the endpoint your own `/usage` command calls.
- It **does not store** it: not in a file, not in a log, not in `limits.json`, not in an
  error message. It is in memory for the length of one request.
- It **does not intermediate** it: nothing is forwarded to any third party, no server but
  `api.anthropic.com` is contacted, the token is not refreshed, exchanged, or shared between
  accounts, and the credential file is never written.

That is the maintainer's reading and it is written down here so that a user can disagree
with it before switching anything on rather than after. If you would rather not make that
call, leave the mode off: the passive path is the product, and it works for everybody.

## Why `ureq`

The mode needs an HTTPS client. Both candidates were measured against this workspace's own
lock file rather than argued about:

| | new packages | what they drag in | licences |
|---|---|---|---|
| `ureq` 3.4 with `rustls` | **4** — `ureq`, `ureq-proto`, `utf8-zero`, `webpki-roots` | nothing else; `rustls` and `ring` are already here | MIT OR Apache-2.0, except `webpki-roots` (CDLA-Permissive-2.0, already on the allow-list) |
| `reqwest` 0.13 with `blocking` + `rustls` | **17** — including `aws-lc-rs`, `aws-lc-sys`, `cmake`, `quinn` | a C toolchain at build time, a QUIC stack, and a tokio runtime | permissive, but `aws-lc-sys` needs `cmake` on every build machine |

`reqwest` is already linked by Tauri, which is the argument for it, but only in the *tray*
crate — and this code lives in `nazar-core`, which has no Tauri in it, builds on three
operating systems in CI, and is linked by `nazar-statusline`, a binary whose whole point is
that it starts in under ten milliseconds. Adding a C-built TLS backend and an async runtime
to that crate to save four packages in another one is the wrong trade.

`zeroize` was already in the lock file as a transitive dependency, so the wiping costs
nothing new.

The feature is why none of it reaches the wrapper: `nazar-statusline` depends on
`nazar-core` with `default-features = false`, and `cargo tree -p nazar-statusline` shows
`serde` and `serde_json` and nothing else. (In a `cargo build --workspace`, cargo unifies
features across the workspace and the shared `nazar-core` rlib is built once with the
feature on; the released wrapper is built on its own, where it is not.)

## Checked against the real endpoint

2026-09-07, `nazar-tray --print --detailed` on the maintainer's machine, one read-only
`GET` with the maintainer's own token:

| `kind` | key | percent | resets at (UTC) |
|---|---|---|---|
| `session` | `five_hour` | 2 | 2026-09-07T13:10:00Z |
| `weekly_all` | `seven_day` | 38 | 2026-09-12T02:00:00Z |
| `weekly_scoped` (Fable) | `seven_day_fable` | 30 | 2026-09-12T02:00:00Z |

`plan: "max_20x"` from `rateLimitTier: "default_claude_max_20x"`, `source: "endpoint"`,
`binding: "seven_day"` because 38 is the highest of the three. The percentages match, to the
point, an independent reading of the same endpoint taken seven minutes earlier by the
retired prototype.

The run wrote nothing: `~/.nazar` and `%APPDATA%\nazar` did not exist before it and did not
exist after it, and `.credentials.json` kept its modification time and its size.

One thing the live check found that the fixtures had not: the endpoint writes
`resets_at` as **`2026-09-07T13:10:00.130195+00:00`** — legal RFC 3339, the right instant,
and neither of the two things the `limits.json` contract promises (a `Z`, whole seconds).
The status-line payload writes Unix seconds for the same field. Both are now rewritten into
the contract's one spelling before they reach the file, and the observed string is pinned as
a test.
