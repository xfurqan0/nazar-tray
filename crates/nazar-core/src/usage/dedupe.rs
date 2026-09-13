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
//! # What is carried between passes, and why it is all of it
//!
//! The working set inside a pass is one file's new records. What survives *between* passes
//! is [`Deduper::credited`]: **every key this file has ever credited, and the most that was
//! ever credited for it.** The first version kept the last sixteen, on the argument that
//! copies of a message are consecutive lines and a window of sixteen covers any real one.
//! That argument is right about the case it was written for — a message whose blocks
//! straddle the cursor — and wrong about the one the review found:
//!
//! > A transcript is truncated, or rewritten in place, or replaced. The reader notices, goes
//! > back to byte zero, and reads records it has already counted. With sixteen keys in hand
//! > it recognises sixteen of them; everything else is credited a second time, into months
//! > that already hold it, permanently and with nothing able to detect it afterwards.
//!
//! So the whole map is carried, and **a restart seeds the deduper with it** rather than
//! clearing it. A re-read record then credits `max(new) − already credited` per counter and
//! never less than zero: reading the same bytes again adds nothing, which is what the
//! contract's idempotence rule says and what the sixteen-key window could only promise for
//! the last sixteen messages of a file.
//!
//! The cost is the cursor document. On the maintainer's machine that is ~8 500 distinct
//! messages across 129 transcripts — a few hundred kilobytes of `cursors-claude.json`, at
//! five numbers per key written as a bare array rather than an object.

use std::collections::{BTreeMap, HashMap};

use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use super::scan::{Record, Usage, fnv1a};

/// A key that has already been credited, and the most that was ever credited for it.
///
/// The key is a 64-bit hash rather than the identifiers themselves, for two reasons: the
/// cursor document stays small, and it cannot be read as a list of the message identifiers a
/// machine has produced. Two distinct keys colliding would under-count one message by the
/// difference between them; at 2⁻⁶⁴ per pair that is not a risk worth a bigger file.
///
/// **Serialised as a five-element array**, `[key, input, output, cache_create, cache_read]`,
/// because there is one of these per message a transcript holds and the field names would be
/// most of the bytes. A counter the source never reported is written as `0`: the only thing a
/// credit is ever used for is [`Usage::since`], which subtracts it, and subtracting nothing
/// and subtracting zero are the same subtraction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Credit {
    /// Hash of `(message.id, requestId)`.
    pub key: u64,
    /// `input_tokens` already credited for it.
    pub input: u64,
    /// `output_tokens` already credited for it.
    pub output: u64,
    /// `cache_creation_input_tokens` already credited for it.
    pub cache_create: u64,
    /// `cache_read_input_tokens` already credited for it.
    pub cache_read: u64,
}

impl Credit {
    /// The four measurements this credit stands for.
    #[must_use]
    pub fn usage(&self) -> Usage {
        Usage {
            input: Some(self.input),
            output: Some(self.output),
            cache_create: Some(self.cache_create),
            cache_read: Some(self.cache_read),
        }
    }

    fn new(key: u64, usage: &Usage) -> Self {
        Credit {
            key,
            input: usage.input.unwrap_or(0),
            output: usage.output.unwrap_or(0),
            cache_create: usage.cache_create.unwrap_or(0),
            cache_read: usage.cache_read.unwrap_or(0),
        }
    }
}

impl Serialize for Credit {
    fn serialize<S: Serializer>(&self, out: S) -> std::result::Result<S::Ok, S::Error> {
        let mut seq = out.serialize_seq(Some(5))?;
        for value in [
            self.key,
            self.input,
            self.output,
            self.cache_create,
            self.cache_read,
        ] {
            seq.serialize_element(&value)?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Credit {
    fn deserialize<D: Deserializer<'de>>(source: D) -> std::result::Result<Self, D::Error> {
        struct Row;
        impl<'de> Visitor<'de> for Row {
            type Value = Credit;
            fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
                out.write_str("a credit row of five numbers")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Credit, A::Error> {
                let mut values = [0u64; 5];
                for (at, slot) in values.iter_mut().enumerate() {
                    *slot = seq.next_element()?.ok_or_else(|| {
                        <A::Error as de::Error>::invalid_length(at, &"five numbers")
                    })?;
                }
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(Credit {
                    key: values[0],
                    input: values[1],
                    output: values[2],
                    cache_create: values[3],
                    cache_read: values[4],
                })
            }
        }
        source.deserialize_seq(Row)
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
///
/// Ordered rather than hashed, for a reason that is about the disk and not about speed: what
/// comes out of [`Deduper::credited`] is written to the cursor document on every scan, and a
/// document whose rows moved about would be rewritten on a pass that changed nothing. The
/// contract's "a second scan leaves the directory byte for byte as it was" is the claim, and
/// a `HashMap`'s iteration order is not the same twice.
#[derive(Debug, Default)]
pub struct Deduper {
    credited: BTreeMap<u64, Usage>,
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
    /// A deduper that already knows everything this file has been credited for.
    ///
    /// Handed the cursor's whole map, on every pass and **including a pass that restarted at
    /// byte zero** — that is the case it exists for. A record read a second time is then a
    /// key that is already credited, and what it adds is the difference, which for the same
    /// bytes is nothing.
    #[must_use]
    pub fn with_credited(credited: &[Credit]) -> Self {
        let mut deduper = Deduper::default();
        for credit in credited {
            deduper.credited.insert(credit.key, credit.usage());
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
            // The credit is the **largest** reading of this message, never the latest one.
            // A smaller copy arriving after a larger one credits nothing, and must not lower
            // the mark the next copy is measured against; see [`Usage::largest`].
            let credit = match self.credited.get(&key) {
                Some(already) => already.largest(&record.usage),
                None => record.usage,
            };
            self.credited.insert(key, credit);
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

    /// Every key this file has credited and the most that was credited for it, by key.
    ///
    /// Stored with the cursor, and handed back to [`Deduper::with_credited`] on the next
    /// pass. Nothing is forgotten: a cursor can go backwards — a truncation, a rewrite, a
    /// replacement — and the bytes behind it can be read again, at which point a key this
    /// map no longer held would be counted a second time.
    #[must_use]
    pub fn credited(&self) -> Vec<Credit> {
        self.credited
            .iter()
            .map(|(key, usage)| Credit::new(*key, usage))
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

        let mut second = Deduper::with_credited(&first.credited());
        let out = second.reduce(vec![record("msg_split", Some("req_split"), 100)]);
        assert!(out.is_empty(), "a second copy must add nothing");
        assert_eq!(second.duplicates, 1);
    }

    #[test]
    fn a_larger_copy_after_the_cursor_adds_only_the_difference() {
        let mut first = Deduper::default();
        first.reduce(vec![record("msg_grow", Some("req_grow"), 100)]);

        let mut second = Deduper::with_credited(&first.credited());
        let out = second.reduce(vec![record("msg_grow", Some("req_grow"), 250)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].delta.output, Some(150));
        assert_eq!(out[0].delta.input, Some(0));
    }

    #[test]
    fn a_smaller_copy_never_lowers_what_was_credited() {
        // The sequence the review found: 100, then 90, then 100. The second credits
        // nothing; the third must credit nothing either, because 100 is what the
        // largest-copy rule says this message cost.
        let mut credited = Vec::new();
        let mut totals = 0u64;
        for output in [100u64, 90, 100] {
            let mut deduper = Deduper::with_credited(&credited);
            for item in deduper.reduce(vec![record("msg_wobble", Some("req_wobble"), output)]) {
                totals += item.delta.output.unwrap_or(0);
            }
            credited = deduper.credited();
        }
        assert_eq!(totals, 100, "100, 90, 100 is one message that cost 100");
    }

    #[test]
    fn every_key_the_file_credited_is_carried_not_the_last_few() {
        let mut deduper = Deduper::default();
        let records: Vec<Record> = (0..200)
            .map(|at| record(&format!("msg_{at}"), Some("req"), 1))
            .collect();
        deduper.reduce(records);

        let credited = deduper.credited();
        assert_eq!(credited.len(), 200, "a window would have kept sixteen");
        // Including the first, which is the one a restart re-reads first of all.
        let oldest = key_of("msg_0", Some("req"));
        assert!(credited.iter().any(|credit| credit.key == oldest));

        // And the rows are in key order, so an unchanged pass writes an unchanged document.
        let keys: Vec<u64> = credited.iter().map(|credit| credit.key).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn reading_a_whole_file_again_credits_nothing_again() {
        let records = || {
            vec![
                record("msg_a", Some("req_a"), 10),
                record("msg_b", Some("req_b"), 20),
                record("msg_c", None, 30),
            ]
        };
        let mut first = Deduper::default();
        let one: u64 = first
            .reduce(records())
            .iter()
            .map(|item| item.delta.total())
            .sum();
        assert_eq!(one, 63);

        // The restart path: the same records, the carried map as the seed.
        let mut second = Deduper::with_credited(&first.credited());
        let two: u64 = second
            .reduce(records())
            .iter()
            .map(|item| item.delta.total())
            .sum();
        assert_eq!(two, 0, "the same bytes must add nothing the second time");
    }

    #[test]
    fn a_credit_row_is_five_numbers_and_survives_a_round_trip() {
        let credit = Credit {
            key: 0x0123_4567_89ab_cdef,
            input: 1,
            output: 2,
            cache_create: 3,
            cache_read: 4,
        };
        let text = serde_json::to_string(&credit).unwrap();
        assert_eq!(text, "[81985529216486895,1,2,3,4]");
        assert_eq!(
            serde_json::from_str::<Credit>(&text).unwrap(),
            credit,
            "the cursor document has one of these per message; it is an array on purpose"
        );
    }
}
