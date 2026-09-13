# nazar-tray 0.2.0

**Your Claude Code and Codex quota, in the system tray — and now where your tokens
went.** 0.1.0 answered *how much of my window is gone?*. This one also answers *how
many tokens did I spend, and on which model?*, off files that were already on your
disk, with no network call and no estimate anywhere in it.

    winget install xfurqan0.nazar-tray

Windows 10 1809 or newer. Installs per user, into `%LOCALAPPDATA%\nazar-tray`,
and asks for no administrator rights.

**The winget package is reviewed a day or two after each release**, so until that
command resolves to 0.2.0, take `nazar-tray_0.2.0_x64-setup.exe` from the assets
below and run it. It upgrades an existing 0.1.0 in place.

## What's new

- **A Usage view**, on a button in the panel footer, with four tabs over one
  history: **Week** (this week, Monday to today where you are), **Weeks** (one row
  per calendar week), **All** (a calendar heat-map, up to twelve months) and
  **Models** (tokens per day, one line per model).
- **Every day and every week opens.** Click a day in the strip or the heat-map, or a
  row of Weeks, and you get what it went on — the four counters, then one row per
  model, under a provider heading when both worked in it. *Back*, or Esc, closes it.
- **Both providers, from files that were already there**: Claude Code's transcripts,
  sub-agent transcripts included, and the `token_count` events in the Codex session
  logs the quota reader already opens. Nine fields are read from a record and a new
  record is built out of them.
- **A second line on the tray tooltip** — this week's tokens and the model that spent
  most of them — which is the same number the view behind it draws.
- **A history that survives pruning.** Hourly UTC totals in `%APPDATA%\nazar\usage\`,
  written by the one process holding the writer's lock, so a transcript Claude Code
  deletes cannot erase what it once contributed.

## How the counting works

The headline is `input + output + cache_read + cache_create`, which is the same
definition Claude Code's `/usage` prints as *total tokens* — and underneath it, the
same total taken apart in four, because cache reads were 98.5 % of the raw total
over six days of real work.

**By default a message is counted once**, however many lines Claude Code wrote it
on. That is not what `/usage` shows: it counts the lines, which on the machine this
was measured on is **1.667×** the real spend and is not a constant you could divide
back out — [anthropics/claude-code#91775](https://github.com/anthropics/claude-code/issues/91775#issuecomment-5654151098).
If you would rather the two windows agreed, **Count like Claude Code** in settings
shows the per-line numbers instead, labelled *as /usage counts*, with the tooltip
following the same switch. The store keeps both counts, so the setting changes what
is drawn and never what was recorded.

A second switch, **Fill history from Claude Code's stats**, copies in the days older
than your transcripts as Claude Code reported them — totals only, no breakdown, drawn
apart from the days nazar-tray measured itself. Both switches are off by default.

## Privacy

**Only metadata is read: nine fields, and never a word you or the model wrote.** Not
message content, not replies, not reasoning, not tool input or output, and not the
paths, project names, branch names or session ids sitting beside them in the same
file. The reader does not filter and hope — it builds a new record out of the fields
it needs, so everything else is gone with the parse, and **a test is what makes that
a fact rather than a promise**: a transcript whose every text field carries a
sentinel goes through the whole scan, and the build fails if that string appears in
the result, in the totals, or in the file they are written to.

The store goes nowhere. It sits beside your settings rather than in `~/.nazar`,
because a month of hourly token counts is a usage profile, and no other program reads
it — this one included.

## Known limits

- Claude numbers move only while a session refreshes its status line. Between
  sessions the last value is shown with its age.
- The scan runs when you open the Usage view, at most once every five minutes, and
  never on the quota refresh loop. *Refresh now* is the only thing that overrides it.
- Model-scoped weekly windows still need the opt-in detailed-windows mode, which
  reads a token — `docs/detailed-windows.md` before you switch it on.
- Codex's log format is not a documented contract; the reader is defensive and
  reports *unknown* rather than guessing.
- **Unsigned.** SmartScreen warns on a browser download — *More info → Run
  anyway*; `winget install` does not go through it. Why, and what to check
  instead: `docs/CODE_SIGNING.md`.
- Windows 11 keeps new tray icons in the `^` overflow; the first run says so.

Full list in the [README](https://github.com/xfurqan0/nazar-tray#known-limits), and
every change in the [CHANGELOG](https://github.com/xfurqan0/nazar-tray/blob/main/CHANGELOG.md).

## Verify

Both files below were built by the release workflow on a GitHub runner, from this
tag, and `SHA256SUMS` is the hash list it wrote:

    Get-FileHash .\nazar-tray_0.2.0_x64-setup.exe -Algorithm SHA256
    gh attestation verify .\nazar-tray_0.2.0_x64-setup.exe --repo xfurqan0/nazar-tray

The attestation is the stronger of the two: signed by GitHub, it names the
workflow run and the commit that produced that exact file.

MIT licensed.
