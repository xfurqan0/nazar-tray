# nazar-tray — what comes after v1

> The single list of work that is wanted but not scheduled. Decisions and the work-package
> log live in [PROJECT.md](PROJECT.md); the public one-paragraph version is the README's
> Roadmap. An entry leaves this file when it becomes a work package there.

## Usage history: tokens per day, tokens per week

*Requested by the maintainer, 2026-09-09.*

Today the panel answers one question: *how much of my window is left?* The second question
it does not answer is *how much did I actually use?* — and the data for it already passes
through the wrapper on every refresh and is thrown away.

**What it should show**

- **Week view:** the last seven days, one bar per day, tokens used that day. Per provider,
  and a total.
- **All view:** every week since the first capture, one bar per week, tokens used that week.
  Same split.

**Where the numbers come from**

- **Claude Code:** the status-line document carries `context_window.total_input_tokens`,
  `total_output_tokens` and `cost.total_cost_usd` per session, cumulative
  (`crates/nazar-core/fixtures/captured/statusline-both-windows.json`). The wrapper keeps
  only `rate_limits` today (`crates/nazar-statusline/src/capture.rs`). Keeping the token
  counters keyed by `session_id` and differencing consecutive captures gives a per-refresh
  delta; a counter that goes backwards means a new session and is a fresh baseline, not a
  negative day.
- **Codex:** the `rollout-*.jsonl` files the Codex reader already tails carry per-turn
  `token_count` events; `input_tokens` includes the cached part, so `input − cached` is the
  honest number (Nazar's N-WP18 reader established that).

**Where they go**

- A new file next to `limits.json`, append-only, one row per day per provider:
  `date, provider, input, output, cached, cost`. Weeks are derived from days at read time,
  never stored. Daily rows are small enough to keep forever.
- `limits.json` itself does not change: its contract is frozen at v1 and Nazar reads it.
  If Nazar wants the history it reads the new file; the contract for that file is written
  into `limits-contract.md` before Nazar depends on it.

**What "week" means**

Calendar weeks starting Monday, in local time, so a bar lines up with the maintainer's own
sense of a week — not the provider's rolling seven-day window, which is what the quota view
already shows and which would confuse the two.

**Open before it is a work package**

- Whether the counters survive a status-line refresh that happens while Claude Code is
  between sessions (the `no-rate-limits` capture has a `context_window` of nulls — a null
  delta is skipped, not zero).
- How the panel's fixed height accommodates a chart: a third view next to quota and
  settings, opened from the provider card, is the leaning.
- Cost in dollars is captured for Claude only; the Codex rows leave it empty rather than
  estimate it.
