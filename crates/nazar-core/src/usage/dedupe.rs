//! Counting a message once, however many lines it was written on.
//!
//! Claude Code writes **one transcript line per content block**, and every one of them
//! carries the whole `usage` object. A message with a `thinking` block, a `text` block and
//! a `tool_use` block is three lines, each claiming the same 651 output tokens. Adding
//! them up inflated a real session by 1.81× on the maintainer's machine and by 2.08× on
//! the sibling repository's — and the factor is **not a constant** that can be divided out
//! afterwards: the same measurement run over three slices of one machine's logs gave
//! 2.49×, 2.38× and 1.04×, the last because a sub-agent that answers in a single block
//! barely repeats itself at all.
//!
//! So the rule, which is ccusage's and is the field's:
//!
//! * The key is `(message.id, requestId)`. A line without a `message.id` is not counted at
//!   all; a line without a `requestId` is keyed on the message id alone and counted in
//!   [`Deduper::fallbacks`], because on a real machine that is eleven lines in fourteen
//!   thousand and hiding them would be worse than naming them.
//! * When a key repeats, **the copy with the largest `input + output + cache_create +
//!   cache_read` wins**, rather than the first one seen. ccusage found the case the hard
//!   way (their issue #888): a streaming view can write an early, partial snapshot of a
//!   message and a complete one later, and keeping the first loses the difference.
//!
//! # How much memory this holds
//!
//! The working set is **one file's new records**, not the whole scan and not the whole
//! corpus. Within a pass the deduper holds one entry per distinct key in the file being
//! read — 7 979 for the 230 MB of transcripts on the maintainer's machine, spread across
//! 118 files, so a few thousand entries at the peak — and it is dropped when the file is.
//! What survives between passes is [`Deduper::recent`]: the last [`RECENT_KEYS`] keys of
//! that file and the usage already credited for them, which is what keeps a message whose
//! blocks landed either side of a cursor from being counted twice. Copies of a key are
//! written as consecutive lines, so a window of sixteen covers a message with far more
//! blocks than any real one while keeping the cursor document small.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::scan::{Record, Usage, fnv1a};

/// Keys carried from one pass to the next, per file.
///
/// Sixteen because copies of a message are consecutive lines and no observed message has
/// more than a handful of content blocks; small because this is written to disk on every
/// scan, once per transcript file.
pub const RECENT_KEYS: usize = 16;

/// A key that has already been credited, and what was credited for it.
///
/// The key is a 64-bit hash rather than the identifiers themselves, for two reasons: the
/// cursor document stays small, and it cannot be read as a list of the message identifiers
/// a machine has produced. Two distinct keys colliding inside a sixteen-entry window would
/// under-count one message by the difference between them; at 2⁻⁶⁴ per pair that is not a
/// risk worth a bigger file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credit {
    /// Hash of `(message.id, requestId)`.
    pub key: u64,
    /// `input_tokens` already credited for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    /// `output_tokens` already credited for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    /// `cache_creation_input_tokens` already credited for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_create: Option<u64>,
    /// `cache_read_input_tokens` already credited for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
}

impl Credit {
    /// The four measurements this credit stands for.
    #[must_use]
    pub fn usage(&self) -> Usage {
        Usage {
            input: self.input,
            output: self.output,
            cache_create: self.cache_create,
            cache_read: self.cache_read,
        }
    }

    fn new(key: u64, usage: Usage) -> Self {
        Credit {
            key,
            input: usage.input,
            output: usage.output,
            cache_create: usage.cache_create,
            cache_read: usage.cache_read,
        }
    }
}

/// The dedupe key of a record, hashed.
///
/// The two halves are separated by a byte that cannot appear in either — the identifiers
/// are checked against a closed character set before they get here — and an absent
/// `requestId` is a different key from an empty one, so two messages cannot merge because
/// one of them was missing a field.
#[must_use]
pub fn key_of(message_id: &str, request_id: Option<&str>) -> u64 {
    let mut bytes = Vec::with_capacity(message_id.len() + 34);
    bytes.extend_from_slice(message_id.as_bytes());
    bytes.push(0);
    match request_id {
        Some(request) => {
            bytes.push(1);
            bytes.extend_from_slice(request.as_bytes());
        }
        None => bytes.push(2),
    }
    fnv1a(&bytes)
}

/// One message, and how much of it is new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credited {
    /// The winning copy of the message.
    pub record: Record,
    /// What it adds on top of anything already credited for the same key.
    pub delta: Usage,
    /// `true` when no earlier pass had credited this key, which is what makes it a
    /// request rather than the rest of one.
    pub fresh: bool,
}

/// One file's dedupe state for one pass.
#[derive(Debug, Default)]
pub struct Deduper {
    credited: HashMap<u64, Usage>,
    order: Vec<u64>,
    /// Lines that were a copy of a key already seen.
    pub duplicates: u64,
    /// Records keyed on `message.id` alone because they carried no `requestId`.
    pub fallbacks: u64,
    /// What the same records would have added up to without any of this.
    ///
    /// Kept so the inflation this module removes can be observed at run time rather than
    /// argued about — the sibling repository's idea, and the reason the 1.81× above is a
    /// measurement and not a guess.
    pub naive_total: u64,
}

impl Deduper {
    /// A deduper that already knows what the previous pass credited for this file.
    #[must_use]
    pub fn with_recent(recent: &[Credit]) -> Self {
        let mut deduper = Deduper::default();
        for credit in recent {
            deduper.credited.insert(credit.key, credit.usage());
            deduper.order.push(credit.key);
        }
        deduper
    }

    /// Reduce one file's new records to what each of them adds.
    ///
    /// Two steps. First the records are collapsed within the pass, largest total winning,
    /// so a message written on five lines becomes one record. Then each survivor is
    /// compared with what a previous pass already credited for the same key, and only the
    /// difference comes out — which is what makes a message whose blocks straddled the
    /// cursor add up to one message rather than two.
    ///
    /// The returned records are in file order and none of them carries an empty delta.
    pub fn reduce(&mut self, records: Vec<Record>) -> Vec<Credited> {
        let mut index: HashMap<u64, usize> = HashMap::new();
        let mut winners: Vec<Record> = Vec::new();

        for record in records {
            self.naive_total = self.naive_total.saturating_add(record.usage.total());
            if record.request_id.is_none() {
                self.fallbacks += 1;
            }
            let key = key_of(&record.message_id, record.request_id.as_deref());
            match index.get(&key) {
                Some(&at) => {
                    self.duplicates += 1;
                    if record.usage.total() > winners[at].usage.total() {
                        winners[at] = record;
                    }
                }
                None => {
                    index.insert(key, winners.len());
                    winners.push(record);
                }
            }
        }

        let mut out = Vec::with_capacity(winners.len());
        for record in winners {
            let key = key_of(&record.message_id, record.request_id.as_deref());
            let (delta, fresh) = match self.credited.get(&key) {
                Some(already) => {
                    self.duplicates += 1;
                    (record.usage.since(already), false)
                }
                None => (record.usage, true),
            };
            self.credited.insert(key, record.usage);
            self.order.push(key);
            if !fresh && (delta.is_empty() || delta.total() == 0) {
                continue;
            }
            out.push(Credited {
                record,
                delta,
                fresh,
            });
        }
        out
    }

    /// The last [`RECENT_KEYS`] keys this file credited, newest last.
    ///
    /// Stored with the cursor. Everything older is forgotten on purpose: the bytes those
    /// keys were read from are behind the cursor and will not be read again.
    #[must_use]
    pub fn recent(&self) -> Vec<Credit> {
        let mut seen = Vec::new();
        for key in self.order.iter().rev() {
            if seen.len() >= RECENT_KEYS {
                break;
            }
            if seen.contains(key) {
                continue;
            }
            seen.push(*key);
        }
        seen.reverse();
        seen.into_iter()
            .filter_map(|key| {
                self.credited
                    .get(&key)
                    .map(|usage| Credit::new(key, *usage))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, request: Option<&str>, output: u64) -> Record {
        Record {
            hour: "2026-09-11T15".to_owned(),
            model: "claude-opus-5".to_owned(),
            message_id: id.to_owned(),
            request_id: request.map(str::to_owned),
            usage: Usage {
                input: Some(1),
                output: Some(output),
                cache_create: None,
                cache_read: None,
            },
        }
    }

    #[test]
    fn three_copies_of_one_message_are_one_message() {
        let mut deduper = Deduper::default();
        let out = deduper.reduce(vec![
            record("msg_one", Some("req_one"), 651),
            record("msg_one", Some("req_one"), 651),
            record("msg_one", Some("req_one"), 651),
        ]);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].delta.output, Some(651));
        assert_eq!(deduper.duplicates, 2);
        assert_eq!(deduper.naive_total, 3 * 652);
    }

    #[test]
    fn the_largest_copy_wins_not_the_first() {
        let mut deduper = Deduper::default();
        let out = deduper.reduce(vec![
            record("msg_two", Some("req_two"), 130_785),
            record("msg_two", Some("req_two"), 648_562),
        ]);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].delta.output, Some(648_562));
    }

    #[test]
    fn two_different_messages_stay_two() {
        let mut deduper = Deduper::default();
        let out = deduper.reduce(vec![
            record("msg_a", Some("req_a"), 10),
            record("msg_b", Some("req_b"), 20),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(deduper.duplicates, 0);
    }

    #[test]
    fn the_same_message_id_on_two_requests_is_two_messages() {
        let mut deduper = Deduper::default();
        let out = deduper.reduce(vec![
            record("msg_same", Some("req_one"), 10),
            record("msg_same", Some("req_two"), 20),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn a_missing_request_id_is_counted_rather_than_hidden() {
        let mut deduper = Deduper::default();
        let out = deduper.reduce(vec![
            record("msg_lonely", None, 10),
            record("msg_lonely", None, 10),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(deduper.fallbacks, 2);
    }

    #[test]
    fn a_copy_that_lands_after_the_cursor_adds_nothing() {
        let mut first = Deduper::default();
        let out = first.reduce(vec![record("msg_split", Some("req_split"), 100)]);
        assert_eq!(out.len(), 1);

        let mut second = Deduper::with_recent(&first.recent());
        let out = second.reduce(vec![record("msg_split", Some("req_split"), 100)]);
        assert!(out.is_empty(), "a second copy must add nothing");
        assert_eq!(second.duplicates, 1);
    }

    #[test]
    fn a_larger_copy_after_the_cursor_adds_only_the_difference() {
        let mut first = Deduper::default();
        first.reduce(vec![record("msg_grow", Some("req_grow"), 100)]);

        let mut second = Deduper::with_recent(&first.recent());
        let out = second.reduce(vec![record("msg_grow", Some("req_grow"), 250)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].delta.output, Some(150));
        assert_eq!(out[0].delta.input, Some(0));
    }

    #[test]
    fn the_carried_window_is_bounded() {
        let mut deduper = Deduper::default();
        let records: Vec<Record> = (0..RECENT_KEYS * 3)
            .map(|at| record(&format!("msg_{at}"), Some("req"), 1))
            .collect();
        deduper.reduce(records);

        let recent = deduper.recent();
        assert_eq!(recent.len(), RECENT_KEYS);
        // The newest keys are the ones kept.
        let newest = key_of(&format!("msg_{}", RECENT_KEYS * 3 - 1), Some("req"));
        assert_eq!(recent.last().unwrap().key, newest);
    }
}
