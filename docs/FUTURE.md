# nazar-tray — what comes after v1

> The single list of work that is wanted but not scheduled. Decisions and the work-package
> log live in [PROJECT.md](PROJECT.md); the public one-paragraph version is the README's
> Roadmap. An entry leaves this file when it becomes a work package there.

*Usage history — tokens per day, per week, per model — **left this file on 2026-09-13**: it is
work packages T-WP12 – T-WP19 in [PROJECT.md](PROJECT.md) §7, the decision behind them is §8,
and the store it writes is [usage-contract.md](usage-contract.md). The plan that lived here,
differencing the status line's token counters, was measured and is recorded there as
disproven.*

**The same blind spot in the sibling repository.** Nazar picks a Codex rollout with
`CODEX_ROLLOUT_SUFFIX = '.jsonl'` — the line this repository fixed as T-WP25 on 2026-09-15 —
and a `.jsonl.zst` does not match that either. The consequence there is much smaller: Nazar's
canvas is about *live* threads, and a live thread's rollout is far newer than the seven-day
threshold Codex compresses at, so it stays plain; what would empty out is old day directories
the canvas has no use for. Recognising the name, and marking such a thread unreadable rather
than absent, is an hour of work **in that repository**. It is written down here because this
is where it was measured, not because it is work in this one.

*Reading a compressed rollout **is** scheduled here: it is T-WP26 in
[PROJECT.md](PROJECT.md) §7, and it is not on this list for that reason.*

Nothing else is on this list yet.
