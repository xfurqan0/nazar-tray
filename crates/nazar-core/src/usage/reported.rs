//! The days the transcripts no longer reach, as Claude Code itself reported them.
//!
//! Claude Code prunes transcripts — thirty days by default, six on the machine this was
//! measured on — and everything before the store's first scan is therefore gone from this
//! product's point of view. It is not gone from Claude Code's: `~/.claude/stats-cache.json`
//! is an undocumented file it keeps for its own Stats screen, and on this machine it reached
//! back sixteen days further than any transcript did.
//!
//! **It is not a source this store counts from, and this module does not make it one.** The
//! numbers in it are the *per-line* sums — every copy of every streamed message added up —
//! which the market research proved digit for digit against four overlapping days:
//! `stats-cache / per-line = 1.00×`, `stats-cache / deduplicated = 1.68×`. Importing them
//! into `claude` would put another program's arithmetic into a total this one promises to
//! have measured, and `docs/limits-contract.md` rule 2 is older than that temptation.
//!
//! So they are imported as what they are:
//!
//! * under a provider key of their own, [`store::PROVIDER_REPORTED`], never merged into
//!   `claude` and never added to any counter of it;
//! * as **one number per model per day**, [`store::Bucket::reported_total`], with the five
//!   counters left at zero — the file holds one total and no split, and writing a guess at
//!   the split would be inventing four numbers out of one;
//! * only for days **strictly before** the first hour the transcripts gave this store, so a
//!   day can never be both measured and reported;
//! * only when the user has asked for it, and rewritten whole from the file every time, so
//!   the block is a copy of another program's answer rather than a history of its own.
//!
//! # Which day is "before the transcripts"
//!
//! The earliest hour the **`claude`** provider holds, floored to its UTC day — not the
//! store-wide `since`, which on a machine with Codex on it is dragged back by rollout logs
//! that say nothing about Claude Code's transcripts. On the maintainer's machine the two
//! differ by twelve days, every one of them a day of Claude work with no transcript left and
//! no reason to be hidden.
//!
//! A store whose `claude` provider holds nothing has no boundary and every reported day is
//! filled; a store that holds the transcripts already has no reported days at all.
//!
//! **The dates in the file are that program's own day keys.** The research measured them
//! against UTC days and they matched digit for digit, which is what this module assumes, and
//! the assumption costs at most one day either way at the edges of the world: a reported date
//! is written at hour `T00` of itself and carried to the panel **as a date**, never
//! re-bucketed into a local day, so nothing here converts a time zone and nothing here can be
//! an hour wrong. The one boundary case a negative offset can produce — a local day that is
//! partly covered by transcripts and also has a reported number — is dropped by the panel,
//! which is the side that knows the offset.
//!
//! # What it reads
//!
//! Three fields of the document and nothing else: `version`, `lastComputedDate`, and
//! `dailyModelTokens[] { date, tokensByModel }`. `modelUsage`, `dailyActivity`,
//! `hourCounts`, `totalSessions`, `totalMessages`, `longestSession` and `firstSessionDate`
//! have no field in any struct here — `longestSession.sessionId` is an identifier of a
//! session on this machine, and the surest way not to store it is to have nowhere to put it.
//! `docs/pinned-internal-formats.md` is the inventory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::store::{
    self, Bucket, Hours, Models, Month, PROVIDER, PROVIDER_REPORTED, ProviderTotals, Read,
};
use crate::error::{Error, Result};
use crate::timefmt::now_rfc3339;

/// What the `source` of a reported block says, so a reader never has to guess.
pub const SOURCE: &str = "claude-stats-cache";

/// The `version` of `stats-cache.json` this build was written against.
///
/// Observed 5 on the maintainer's machine, with `dailyModelTokensVersion` also 5. It is
/// **recorded, not enforced**: the shape read here is three fields deep and a version bump
/// that kept them is not a reason to stop reading, while one that changed them fails the
/// same way a missing field does — the entry is skipped and nothing is written.
pub const VERSION_OBSERVED: u64 = 5;

/// `<home>/.claude/stats-cache.json`.
///
/// Derived from the home the caller passes, like every other path in this module, so a test
/// points the whole thing at a throwaway tree. A caller honouring `CLAUDE_CONFIG_DIR` names
/// the file itself and calls [`backfill_claude_stats_from`].
#[must_use]
pub fn stats_cache_path(home: &Path) -> PathBuf {
    home.join(".claude").join("stats-cache.json")
}

/// What one backfill did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Backfill {
    /// Days written, which is days the file holds that are before the boundary.
    pub days: u64,
    /// Model rows written across those days.
    pub models: u64,
    /// What those rows add up to. Per-line tokens, as the other program counted them.
    pub total: u64,
    /// The first UTC day the transcripts cover, `YYYY-MM-DD`, or `None` when they cover none.
    pub boundary: Option<String>,
    /// `version` as the file carried it, for a diagnostic. See [`VERSION_OBSERVED`].
    pub version: Option<u64>,
    /// `lastComputedDate` as the file carried it. That program recomputes lazily, so this is
    /// routinely a day or two behind today and is worth being able to see.
    pub last_computed: Option<String>,
    /// The `YYYY-MM` months whose documents were written.
    pub months: Vec<String>,
    /// The `YYYY-MM` months whose documents do not parse. Left exactly as they are.
    pub damaged: Vec<String>,
    /// `true` when there is no `stats-cache.json` to read, which is not a failure.
    pub absent: bool,
}

/// The three fields of `stats-cache.json` this build reads.
///
/// Nothing else has a field here, which is the whole of the promise: what is not named is not
/// built, so it never becomes a string, a value or a borrowed slice. See the module
/// documentation and `docs/pinned-internal-formats.md`.
#[derive(Debug, Default, Deserialize)]
struct StatsCache {
    #[serde(default)]
    version: Option<u64>,
    #[serde(default, rename = "lastComputedDate")]
    last_computed_date: Option<String>,
    #[serde(default, rename = "dailyModelTokens")]
    daily_model_tokens: Vec<DailyModelTokens>,
}

/// One day of it: a date, and one total per model.
#[derive(Debug, Default, Deserialize)]
struct DailyModelTokens {
    #[serde(default)]
    date: Option<String>,
    /// The values are read as [`Value`] and then asked for a non-negative integer, rather
    /// than typed as one: a number that changed shape must skip its row and leave the rest of
    /// the file readable, which is the rule the four counters already keep in
    /// [`super::scan`]. Nothing is kept but the model name and the integer.
    #[serde(default, rename = "tokensByModel")]
    tokens_by_model: BTreeMap<String, Value>,
}

/// Whether `text` is a `YYYY-MM-DD` day key, which is the only shape a date here may have.
fn is_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..].iter().all(u8::is_ascii_digit)
}

/// The first UTC day the `claude` provider holds anything for, `YYYY-MM-DD`.
///
/// The boundary of the backfill, and the module documentation says why it is this provider's
/// earliest hour rather than the store-wide `since`.
pub fn transcript_boundary(state_dir: &Path) -> Result<Option<String>> {
    let mut earliest: Option<String> = None;
    for month in store::months(state_dir)? {
        let Read::Document(document) = store::read_month(state_dir, &month)? else {
            continue;
        };
        let Some(hours) = document.buckets(PROVIDER) else {
            continue;
        };
        if let Some(first) = hours.keys().next()
            && earliest.as_ref().is_none_or(|held| first < held)
        {
            earliest = Some(first.clone());
        }
    }
    // `YYYY-MM-DDTHH` floored to its day is its first ten characters.
    Ok(earliest.map(|hour| hour[..10].to_owned()))
}

/// Fill the days before the transcripts from Claude Code's own statistics cache.
///
/// `home` is the user's home directory; `state_dir` is where this product keeps the usage
/// documents. Safe to call as often as the scan runs: the whole [`PROVIDER_REPORTED`] block
/// is rebuilt from the file on every call, so calling it twice writes the same bytes and
/// [`store::write_month`] does not touch the disk the second time.
pub fn backfill_claude_stats(home: &Path, state_dir: &Path) -> Result<Backfill> {
    backfill_claude_stats_from(&stats_cache_path(home), state_dir)
}

/// [`backfill_claude_stats`] against an explicit `stats-cache.json`.
///
/// **This is the only function in the module that writes**, and what it writes is a
/// *replacement*: every month it touches has its reported block set to what the file says
/// now, and a month that had one and no longer qualifies for it has the key removed. There is
/// no journal, no generation and no `applied_through`, because there is nothing to be
/// idempotent *about* — the block is not an accumulation of passes, it is a copy.
pub fn backfill_claude_stats_from(path: &Path, state_dir: &Path) -> Result<Backfill> {
    let mut outcome = Backfill::default();

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            // A machine whose Claude Code has never computed its statistics. Nothing to fill,
            // and nothing already filled to take away either: an absent file is not a file
            // that says "no days".
            outcome.absent = true;
            return Ok(outcome);
        }
        Err(source) => return Err(Error::io(path, source)),
    };
    let cache: StatsCache =
        serde_json::from_str(&text).map_err(|source| Error::json(path, source))?;
    outcome.version = cache.version;
    outcome.last_computed = cache
        .last_computed_date
        .as_deref()
        .filter(|date| is_day(date))
        .map(str::to_owned);

    outcome.boundary = transcript_boundary(state_dir)?;

    // Month to hour-keyed buckets, built entirely out of new values: a model id and an
    // integer. Nothing that was parsed off the document is carried into it.
    let mut wanted: BTreeMap<String, Hours> = BTreeMap::new();
    for day in &cache.daily_model_tokens {
        let Some(date) = day.date.as_deref().filter(|date| is_day(date)) else {
            continue;
        };
        // Strictly before: a day the transcripts touch at all is a day this store measured,
        // and two numbers for one day is the thing this rule exists to prevent.
        if outcome
            .boundary
            .as_deref()
            .is_some_and(|boundary| date >= boundary)
        {
            continue;
        }

        let mut models = Models::new();
        for (model, value) in &day.tokens_by_model {
            // A total that is not a non-negative integer is not read as one, for the reason
            // the contract gives about the four counters: a source that changed the shape of
            // a field has stopped saying what it used to say.
            let Some(total) = value.as_u64() else {
                continue;
            };
            if total == 0 || model.is_empty() {
                continue;
            }
            models.insert(
                model.clone(),
                Bucket {
                    reported_total: Some(total),
                    ..Bucket::default()
                },
            );
            outcome.models += 1;
            outcome.total = outcome.total.saturating_add(total);
        }
        if models.is_empty() {
            continue;
        }
        outcome.days += 1;
        // One bucket at hour `T00` of the day it names. It is a *day* the other program
        // computed, not an hour anything happened in; the panel is handed the date rather
        // than the hour, and the contract says so where it says what this block is.
        wanted
            .entry(date[..7].to_owned())
            .or_default()
            .insert(format!("{date}T00"), models);
    }

    write_blocks(state_dir, &wanted, &mut outcome)?;
    Ok(outcome)
}

/// Put each month's reported block on disk, and take away the ones that no longer belong.
fn write_blocks(
    state_dir: &Path,
    wanted: &BTreeMap<String, Hours>,
    outcome: &mut Backfill,
) -> Result<()> {
    let known = store::months(state_dir)?;
    let mut documents: BTreeMap<String, Month> = BTreeMap::new();

    for month in known.iter().chain(wanted.keys()) {
        if documents.contains_key(month) || outcome.damaged.contains(month) {
            continue;
        }
        match store::read_month(state_dir, month)? {
            Read::Document(document) => {
                documents.insert(month.clone(), *document);
            }
            // A month that has no document yet is one the transcripts never reached — which
            // is exactly the case this feature exists for, so it is created rather than
            // skipped.
            Read::Absent if wanted.contains_key(month) => {
                documents.insert(month.clone(), Month::new(month));
            }
            Read::Absent => {}
            Read::Damaged => outcome.damaged.push(month.clone()),
        }
    }

    // The store-wide `since` as the measured providers see it, so a month created here
    // carries the same one every month beside it does. `apply` recomputes it the same way on
    // the next scan; this is only for the case where nothing has scanned since.
    let since = documents
        .values()
        .filter_map(Month::earliest)
        .min()
        .or_else(|| {
            documents
                .values()
                .find_map(|document| document.since.clone())
        });
    let scanned_at = now_rfc3339();

    for (month, document) in &mut documents {
        let before = document.providers.get(PROVIDER_REPORTED).cloned();
        match wanted.get(month) {
            Some(hours) => {
                document.providers.insert(
                    PROVIDER_REPORTED.to_owned(),
                    ProviderTotals {
                        applied_through: 0,
                        buckets: hours.clone(),
                        extra: reported_source(),
                    },
                );
            }
            // A month that used to have reported days and no longer does — the transcripts
            // reached further back, or the other program forgot the day. The block goes; a
            // copy that is no longer a copy of anything is not history, it is a leftover.
            None => {
                document.providers.remove(PROVIDER_REPORTED);
            }
        }
        if before == document.providers.get(PROVIDER_REPORTED).cloned() {
            continue;
        }
        if document.providers.is_empty() {
            // Nothing left to say. The document is left exactly as it is rather than written
            // out empty: a month with no providers is a month this store never had.
            continue;
        }
        document.version = store::VERSION;
        document.month = month.clone();
        if document.since.is_none() {
            document.since.clone_from(&since);
        }
        if document.scanned_at.is_none() {
            document.scanned_at = Some(scanned_at.clone());
        }
        if store::write_month(state_dir, document)? {
            outcome.months.push(month.clone());
        }
    }
    Ok(())
}

/// `"source": "claude-stats-cache"` on the provider block, written once rather than per row.
///
/// The provider key already says where these numbers came from; this says it in words, in the
/// one place a person opening the file will look, and it costs one line per month instead of
/// one per model per day.
fn reported_source() -> serde_json::Map<String, Value> {
    let mut extra = serde_json::Map::new();
    extra.insert("source".to_owned(), Value::String(SOURCE.to_owned()));
    extra
}
