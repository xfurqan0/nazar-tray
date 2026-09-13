# Screenshots

Every picture in this directory is produced by one command, from the code in the tree:

```powershell
cargo build -p nazar-tray
powershell -File scripts/screenshot.ps1
```

The script launches the tray with `--demo`, waits for the panel, captures exactly the window
rectangle and stops the process again. **Quit a running nazar-tray from its tray menu first**
— two processes cannot share one WebView2 user-data folder with different browser arguments,
and `--scale` passes one, so a tray that is already up makes every scaled shot time out. The
script refuses to start rather than letting that happen.

**Nothing here is anybody's real data.** `--demo` never takes the advisory lock, never writes,
and since 0.2.0 answers the usage view out of a fixture of its own
(`crates/nazar-tray/src/demo.rs`) rather than out of `%APPDATA%\nazar\usage` — five weeks of
hourly buckets, four model ids, both providers, and two days at the far end that only Claude
Code's statistics cache reaches. A test proves a demo run has no store to open at all.

**150 % and 200 % are rendered at those scales**, not upscaled: WebView2 is given
`--force-device-scale-factor` and the window is multiplied to match, so no display setting has
to be touched to produce them.

## The set

| File | What it is |
|---|---|
| `wp4-100-nazar-dark.png` | the panel, default theme, dark. **The README's first picture.** |
| `wp4-150-nazar-dark.png` | the same at 150 % |
| `wp4-200-nazar-dark.png` | the same at 200 % |
| `wp4-100-nazar-light.png` | the same in light mode |
| `wp4-100-graphite-dark.png` | the `graphite` theme, dark |
| `wp4-100-graphite-light.png` | the `graphite` theme, light |
| `wp4-100-first-run.png` | the Windows 11 overflow hint, a state a machine is in once |
| `wp5-100-settings.png` | the settings page, from the top |
| `wp5-100-offer.png` | the one-time Max-plan offer, the other once-per-machine state |
| `wp7-100-statusline.png` | the settings page's **Status line** section |
| `usage-100-week.png` | the usage view, **Week**: the headline, its four counters, the strip of seven days |
| `usage-100-weeks.png` | **Weeks**: one row per calendar week, newest first |
| `usage-100-all.png` | **All**: the calendar heat-map, with the two reported days drawn as outlines |
| `usage-100-models.png` | **Models**: tokens per day, one line per model, and the share of the span each took |
| `usage-100-day.png` | a day opened from the Week strip, with both providers in it and one *Back* |
| `usage-100-settings.png` | the settings page's **Usage history** section and its two switches |
| `wp6-100-<lang>.png` | the same panel in each of the six languages |

The `wp<n>` names are the work package that first needed the picture; the usage set is named
for what it shows instead, because the four tabs are one feature and spreading them over four
work-package prefixes would name nothing anybody outside this repository could read.

## One state at a time

```powershell
powershell -File scripts/screenshot.ps1 -Scale 2 -Theme graphite -Mode light -Out x.png
powershell -File scripts/screenshot.ps1 -View settings -Scroll 8 -Out x.png
powershell -File scripts/screenshot.ps1 -View usage -UsageTab all -Locale ru -Out x.png
```

`-Scroll` exists because the settings page is taller than the window it lives in, so a section
near its end cannot be photographed without the wheel. `-UsageTab day` is the Week tab with its
newest day already opened, which is the one state of that view a tab name cannot reach.

The bead itself is not photographed — it is drawn by the code that draws the real icon:

```powershell
cargo run -p nazar-tray -- --icons docs/design
```
