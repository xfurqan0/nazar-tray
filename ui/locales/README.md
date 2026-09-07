# Translations

Six languages, one flat JSON file each, and **one copy of each file** — the panel bundles
them with esbuild and the Rust side compiles the very same paths in with `include_str!`, so
the tray tooltip, the context menu and the Windows notifications cannot drift from the panel.
There is no translation table anywhere in the Rust sources.

**Corrections are welcome as pull requests.** English and Turkish were written by hand;
Chinese, Korean, Russian and Spanish were machine-translated first, exactly as
`docs/PROJECT.md` §3 says, and none of the four has been reviewed by a native speaker yet. If
something reads badly to you, that is not a nuisance — it is the thing this table is for.

## Status

| File | Language | Written by | Reviewed by a speaker |
|---|---|---|---|
| `en.json` | English | maintainer | — (source language) |
| `tr.json` | Türkçe | maintainer | maintainer (native) |
| `zh.json` | 中文 (简体) | machine-translated, WP6 | not yet |
| `ko.json` | 한국어 | machine-translated, WP6 | not yet |
| `ru.json` | Русский | machine-translated, WP6 | not yet |
| `es.json` | Español | machine-translated, WP6 | not yet |

The status lives in this table rather than in a `_meta` key inside the files, and that is a
finding rather than a preference. `crates/nazar-tray/src/i18n.rs` parses each file as
`BTreeMap<String, String>`; a nested object makes the **whole file** fail to parse, and the
failure is deliberately silent — a damaged translation must not stop the tray from starting —
so the language simply stops being offered in the settings and everybody who chose it gets
English. Tried, watched happen, reverted. Keep every value a string.

## Fixing a translation

1. Edit the string in `ui/locales/<lang>.json`. Nothing else needs to change: the settings
   list, the tray menu and the notifications all read these files, and none of them has a
   copy of a word.
2. `cd ui && npm test` — the checks below run there.
3. `cargo test -p nazar-tray` — the Rust half checks the same files from its own side.
4. Open a pull request. Say which language you speak; that is what moves a row in the table
   above from *not yet* to reviewed.

## The rules a locale file has to keep

Each of these is a test in `ui/test/i18n.test.mjs`, so getting one wrong fails the build
rather than reaching a user.

- **Exactly English's keys.** A missing key fails; so does an invented one. English is the
  fallback, so a missing key would silently show the English string and never be noticed.
- **No empty and no padded values.** A blank string is an invisible label.
- **The same placeholders.** `{model}`, `{percent}`, `{time}`, `{age}`, `{days}`, `{hours}`,
  `{minutes}`, `{seconds}`, `{provider}`, `{window}`. A placeholder with no value is printed
  as written — `{percent}` on somebody's screen — rather than blanked, so a typo is loud.
  They may be **reordered** freely; that is most of what translating these strings is.
- **Product names are left alone**: `nazar-tray`, `Claude Code`, `Codex`, `Claude`, `Nazar`.
  A translated brand is the wrong brand.
- **Language names stay in their own language.** `settings.language.ko` is `한국어` in all six
  files, because a picker is read by somebody looking for their own language in it.
- **Flat strings only.** See above.

## Numbers, units and plurals

**No message in this product needs a plural form, and that is by construction.** Every string
that carries a count renders it next to a unit *abbreviation* — `4 d 2 h`, `2 sa 10 dk`,
`2 小时 10 分`, `2시간 10분`, `2 ч 10 мин`, `2 h 10 min` — and an abbreviation is the same word
after 1 as after 5 in all six languages. That matters most for Russian, which otherwise needs
three forms (1, then 2–4, then 5 and up, with 11–14 in neither of the first two).

If you ever need a counted **word**, `pluralCategory` and `plural` in `ui/src/i18n.ts`
implement exactly that rule. `ui/test/i18n.test.mjs` freezes the list of keys that carry a
count, so adding one fails the suite until somebody decides which of the two routes it takes.

Percent signs and unit spacing follow the language, not English: Turkish writes `%88`,
Chinese and Korean write `88%` with no space, English, Russian and Spanish write `88 %`.

## What is deliberately **not** translated

- **Reader error sentences.** A window that could not be read carries the reader's own
  sentence — which file said what — and the panel prints it verbatim. No translation can know
  in advance what a malformed log will say.
- **Plan names.** `max_20x`, `plus`: shown exactly as the source spelled them.
- **Addresses.** `github.com/xfurqan0/nazar-tray`, `docs/limits-contract.md`.
- **Anything on the command line.** `--print` emits JSON for a script; `--autostart` answers
  in English. Standard output is a maintainer's surface, not a user's, and `docs/PROJECT.md`
  keeps it in English on purpose.
- **Theme identifiers.** The theme *names* are translated (`settings.theme.graphite`); the
  values written into `config.json` are not.

## Writing direction

All six languages are left to right, so the panel sets `<html lang>` and never `dir`. Adding
Arabic, Hebrew, Persian or Urdu is more than a JSON file: it needs a direction table in
`ui/src/i18n.ts` and an audit of `ui/src/styles.css`, which still uses physical
`margin-left` and `text-align: right` in a dozen places. A test in `ui/test/i18n.test.mjs`
fails if an RTL tag is added to `LOCALES` before that work is done.
