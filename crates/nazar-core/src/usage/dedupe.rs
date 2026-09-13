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
//! nine numbers per key written as a bare array rather than an object.
//!
//! # The other number: what the lines came to before any of this
//!
//! T-WP22 asks this module for a second total beside the deduplicated one — the **per-line**
//! sum, which is what Claude Code's own `/usage` shows and what `~/.claude/stats-cache.json`
//! stores, digit for digit. It is computed in the same pass and against the same key: while
//! the copies of a message are being collapsed into the largest of them, their `usage`
//! objects are also simply added up, and [`Credited::raw`] is what that adds to the bucket.
//!
//! **Keyed per message, not per bucket, and that is the whole of why it is safe.** The two
//! numbers then move together: a bucket's `raw` is the sum over exactly the messages whose
//! deduplicated readings are in the same bucket, so `raw ≥ deduped` holds hour by hour and
//! model by model rather than only on average. Keying the per-line sum by the hour instead
//! would have let a rewrite that swapped one message for another of the same size credit the
//! new message deduplicated and nothing at all raw, which is a raw total *below* a
//! deduplicated one — an impossible number drawn as a fact.
//!
//! # The invariant, and why a byte offset is not enough for a sum
//!
//! **A sum is not a snapshot.** Every copy of a message carries the *whole* `usage` object,
//! which is why the deduplicated side can say `max(new) − already` in both directions and be
//! right however the pass arrived. A sum over lines cannot: the same line read twice is two
//! lines unless something remembers that it is not.
//!
//! So each key carries **two** per-line numbers rather than one:
//!
//! * `raw` — the high-water mark, the most this message's lines have ever come to, and what
//!   a bucket has already been credited.
//! * `in_file` — what those lines come to **in the file as it now stands**. Reset to nothing
//!   by [`Deduper::rereading`] when a pass goes back to byte zero, and added to as the file
//!   grows.
//!
//! What is credited is then `in_file − raw`, saturating at nothing, and the invariant falls
//! out of it:
//!
//! > **A bucket's per-line counters are the largest per-line sum the transcripts behind them
//! > have ever held, and reading any byte a second time adds nothing to them.**
//!
//! The sequence a byte offset alone gets wrong, and this does not: a transcript is pruned to
//! its first line, and then a copy of the original is put back. The restart pass counts one
//! line (`in_file` = 1, high-water 3, credit nothing). The pass after it is an ordinary
//! *append* — the offset is valid, the fingerprint matches, nothing says the file ever shrank
//! — and it reads lines two and three. Adding them would credit the same lines twice,
//! permanently, with nothing able to detect it afterwards. `in_file` climbs back to 3, the
//! high-water is 3, and the credit is nothing.
//!
//! # What the cursor row costs
//!
//! **Nine numbers, or thirteen for a key whose file has been truncated below its high-water
//! mark** — `in_file` is written only when it differs from `raw`, which no ordinary row does.
//!
//! **A row written before T-WP22 carries five**, and both per-line numbers are seeded from
//! its deduplicated half — the largest copy, which is a *lower bound* on the per-line sum and
//! the only honest one available, since the bytes those lines were in are behind a cursor
//! that has already moved. It is the same number [`super::store::Bucket`] reads an absent
//! `raw` as, so the two cancel: a restart after an upgrade credits the file's true per-line
//! sum exactly once, and neither side has to invent a factor of 1.7.

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
/// **Serialised as a nine-element array**, `[key, input, output, cache_create, cache_read,
/// raw_input, raw_output, raw_cache_create, raw_cache_read]`, because there is one of these
/// per message a transcript holds and the field names would be most of the bytes. A counter
/// the source never reported is written as `0`: the only thing a credit is ever used for is
/// [`Usage::since`], which subtracts it, and subtracting nothing and subtracting zero are the
/// same subtraction.
///
/// **A five-element row is read as one written before T-WP22** and its four raw counters are
/// seeded from the four beside them — the largest copy of the message, which is a lower bound
/// on its per-line sum and the only honest one left once the bytes have been walked past. The
/// module documentation says why that seed is exactly right rather than merely safe.
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
    /// `input_tokens` summed over **every line** of this message that has been credited —
    /// the high-water mark, the most this message's lines have ever come to.
    pub raw_input: u64,
    /// `output_tokens`, the same way.
    pub raw_output: u64,
    /// `cache_creation_input_tokens`, the same way.
    pub raw_cache_create: u64,
    /// `cache_read_input_tokens`, the same way.
    pub raw_cache_read: u64,
    /// `input_tokens` summed over this message's lines **as the file now holds them**.
    pub file_input: u64,
    /// `output_tokens`, the same way.
    pub file_output: u64,
    /// `cache_creation_input_tokens`, the same way.
    pub file_cache_create: u64,
    /// `cache_read_input_tokens`, the same way.
    pub file_cache_read: u64,
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

    /// The most this message's lines have ever come to.
    #[must_use]
    pub fn raw(&self) -> Usage {
        Usage {
            input: Some(self.raw_input),
            output: Some(self.raw_output),
            cache_create: Some(self.raw_cache_create),
            cache_read: Some(self.raw_cache_read),
        }
    }

    /// What this message's lines come to in the file as it now stands.
    #[must_use]
    pub fn in_file(&self) -> Usage {
        Usage {
            input: Some(self.file_input),
            output: Some(self.file_output),
            cache_create: Some(self.file_cache_create),
            cache_read: Some(self.file_cache_read),
        }
    }

    fn new(key: u64, usage: &Usage, raw: &Usage, in_file: &Usage) -> Self {
        Credit {
            key,
            input: usage.input.unwrap_or(0),
            output: usage.output.unwrap_or(0),
            cache_create: usage.cache_create.unwrap_or(0),
            cache_read: usage.cache_read.unwrap_or(0),
            raw_input: raw.input.unwrap_or(0),
            raw_output: raw.output.unwrap_or(0),
            raw_cache_create: raw.cache_create.unwrap_or(0),
            raw_cache_read: raw.cache_read.unwrap_or(0),
            file_input: in_file.input.unwrap_or(0),
            file_output: in_file.output.unwrap_or(0),
            file_cache_create: in_file.cache_create.unwrap_or(0),
            file_cache_read: in_file.cache_read.unwrap_or(0),
        }
    }
}

impl Serialize for Credit {
    fn serialize<S: Serializer>(&self, out: S) -> std::result::Result<S::Ok, S::Error> {
        // Thirteen numbers only when the last four are not the four before them, which is
        // the case a file has to have been truncated to reach. Every ordinary row is nine.
        let short = self.raw() == self.in_file();
        let mut seq = out.serialize_seq(Some(if short { 9 } else { 13 }))?;
        for value in [
            self.key,
            self.input,
            self.output,
            self.cache_create,
            self.cache_read,
            self.raw_input,
            self.raw_output,
            self.raw_cache_create,
            self.raw_cache_read,
        ] {
            seq.serialize_element(&value)?;
        }
        if !short {
            for value in [
                self.file_input,
                self.file_output,
                self.file_cache_create,
                self.file_cache_read,
            ] {
                seq.serialize_element(&value)?;
            }
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
                out.write_str("a credit row of five, nine or thirteen numbers")
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
                // Four more, or none. A row of five was written before T-WP22 and its
                // per-line sums are seeded from the largest copy beside them — a lower bound
                // rather than a guess, and the one an absent `raw` on a bucket already means.
                let mut raw = [values[1], values[2], values[3], values[4]];
                let mut in_file = raw;
                if let Some(first) = seq.next_element::<u64>()? {
                    raw[0] = first;
                    for slot in raw.iter_mut().skip(1) {
                        *slot = seq.next_element()?.ok_or_else(|| {
                            <A::Error as de::Error>::invalid_length(5, &"nine numbers")
                        })?;
                    }
                    // And four more again, or none: a row of nine is one whose file holds
                    // exactly what its high-water mark says, which is every row that has
                    // never been truncated.
                    in_file = raw;
                    if let Some(first) = seq.next_element::<u64>()? {
                        in_file[0] = first;
                        for slot in in_file.iter_mut().skip(1) {
                            *slot = seq.next_element()?.ok_or_else(|| {
                                <A::Error as de::Error>::invalid_length(9, &"thirteen numbers")
                            })?;
                        }
                    }
                }
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(Credit {
                    key: values[0],
                    input: values[1],
                    output: values[2],
                    cache_create: values[3],
                    cache_read: values[4],
                    raw_input: raw[0],
                    raw_output: raw[1],
                    raw_cache_create: raw[2],
                    raw_cache_read: raw[3],
                    file_input: in_file[0],
                    file_output: in_file[1],
                    file_cache_create: in_file[2],
                    file_cache_read: in_file[3],
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
    /// What **every line** of this message adds on top of the same, before any copy was
    /// thrown away.
    ///
    /// The per-line number, which is never smaller than [`Credited::delta`] and was 1.68×
    /// it over the days both could be measured on the maintainer's machine. It goes into the
    /// same bucket as the delta, under `raw`; see [`super::store::Bucket`].
    pub raw: Usage,
    /// `true` when no earlier pass had credited this key, which is what makes it a
    /// request rather than the rest of one.
    pub fresh: bool,
}

/// What one dedupe key has been credited for, in the three numbers that decide the next pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Credits {
    /// The largest reading of this message ever seen, counter by counter.
    usage: Usage,
    /// The most this message's lines have ever come to: the per-line high-water mark, and
    /// what a bucket's `raw` has already been credited.
    raw: Usage,
    /// What this message's lines come to **in the file as it now stands** — reset to nothing
    /// when a pass goes back to byte zero, and added to as the file grows.
    ///
    /// The guard, and the thing a byte offset cannot stand in for. Without it a transcript
    /// that was truncated and then grew back past the cut would hand the same lines to an
    /// *append* pass, which has no way to tell them from new ones; with it, what is credited
    /// is `in_file − high-water`, which for lines that were already counted is nothing. The
    /// module documentation works the sequence through.
    in_file: Usage,
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
    credited: BTreeMap<u64, Credits>,
    /// Whether this pass is reading bytes it may have read before.
    ///
    /// `true` when the file was truncated, rewritten or replaced and [`super::scan`] went
    /// back to byte zero. It changes nothing on the deduplicated side — every copy of a
    /// message carries the whole `usage` object, so `max(new) − already` is right either way
    /// — and it is the whole of the arithmetic on the per-line side, which is a **sum** and
    /// would otherwise be added to itself. See the module documentation.
    rereading: bool,
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
            deduper.credited.insert(
                credit.key,
                Credits {
                    usage: credit.usage(),
                    raw: credit.raw(),
                    in_file: credit.in_file(),
                },
            );
        }
        deduper
    }

    /// Say that this pass went back to byte zero, which the per-line sum has to know.
    ///
    /// Taken straight from [`super::scan::Pass::restarted`] and passed on every pass, not
    /// only the ones that restarted: a builder that is only called sometimes is a builder
    /// somebody forgets. The deduplicated numbers do not change either way.
    ///
    /// What it does is forget, for every key, **what the file used to hold** — because the
    /// pass is about to read the file from the top and say so again. A key the new file no
    /// longer has is then a key with nothing in the file, which is exactly true, and one it
    /// still has is counted up from zero as the pass walks it.
    #[must_use]
    pub fn rereading(mut self, restarted: bool) -> Self {
        self.rereading = restarted;
        if restarted {
            for credits in self.credited.values_mut() {
                credits.in_file = Usage::default();
            }
        }
        self
    }

    /// Reduce one file's new records to what each of them adds.
    ///
    /// Two steps. First the records are collapsed within the pass, largest total winning,
    /// so a message written on five lines becomes one record — and added up as they go, which
    /// is the per-line total the same walk produces for nothing. Then each survivor is
    /// compared with what a previous pass already credited for the same key, and only the
    /// difference comes out — which is what makes a message whose blocks straddled the
    /// cursor add up to one message rather than two.
    ///
    /// The returned records are in file order and none of them carries an empty delta on
    /// both sides.
    pub fn reduce(&mut self, records: Vec<Record>) -> Vec<Credited> {
        let mut index: HashMap<u64, usize> = HashMap::new();
        let mut winners: Vec<Record> = Vec::new();
        // Parallel to `winners`: every line of that key in this pass, added up.
        let mut lines: Vec<Usage> = Vec::new();

        for record in records {
            self.naive_total = self.naive_total.saturating_add(record.usage.total());
            if record.request_id.is_none() {
                self.fallbacks += 1;
            }
            let key = key_of(&record.message_id, record.request_id.as_deref());
            match index.get(&key) {
                Some(&at) => {
                    self.duplicates += 1;
                    lines[at] = lines[at].plus(&record.usage);
                    if record.usage.total() > winners[at].usage.total() {
                        winners[at] = record;
                    }
                }
                None => {
                    index.insert(key, winners.len());
                    lines.push(record.usage);
                    winners.push(record);
                }
            }
        }

        let mut out = Vec::with_capacity(winners.len());
        for (at, record) in winners.into_iter().enumerate() {
            let key = key_of(&record.message_id, record.request_id.as_deref());
            let read = lines[at];
            let already = self.credited.get(&key).copied();
            let (delta, fresh) = match already {
                Some(already) => {
                    self.duplicates += 1;
                    (record.usage.since(&already.usage), false)
                }
                None => (record.usage, true),
            };
            // The per-line side is a sum over lines, and what it credits is the difference
            // between **what the file holds now** and the most it has ever held. On an
            // ordinary pass that is exactly the new lines; after a truncation it is nothing,
            // because `in_file` was forgotten and is being counted up again from zero.
            let in_file = already.map_or(read, |already| already.in_file.plus(&read));
            let raw = already.map_or(read, |already| in_file.since(&already.raw));
            // The credit is the **largest** reading of this message, never the latest one.
            // A smaller copy arriving after a larger one credits nothing, and must not lower
            // the mark the next copy is measured against; see [`Usage::largest`].
            let credit = Credits {
                usage: match already {
                    Some(already) => already.usage.largest(&record.usage),
                    None => record.usage,
                },
                // The high-water mark never goes down, for the reason the largest copy never
                // does: a file that lost lines must not lower the mark the next pass is
                // measured against and then credit those lines all over again.
                raw: already.map_or(read, |already| already.raw.largest(&in_file)),
                in_file,
            };
            self.credited.insert(key, credit);
            if !fresh && delta.total() == 0 && raw.total() == 0 {
                continue;
            }
            out.push(Credited {
                record,
                delta,
                raw,
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
            .map(|(key, credits)| Credit::new(*key, &credits.usage, &credits.raw, &credits.in_file))
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
    fn a_copy_that_lands_after_the_cursor_adds_nothing_deduplicated() {
        let mut first = Deduper::default();
        let out = first.reduce(vec![record("msg_split", Some("req_split"), 100)]);
        assert_eq!(out.len(), 1);

        // New bytes, not the same bytes again: this is the second content block of a
        // message whose first one was already read.
        let mut second = Deduper::with_credited(&first.credited());
        let out = second.reduce(vec![record("msg_split", Some("req_split"), 100)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].delta.total(), 0, "the message cost what it cost");
        assert_eq!(
            out[0].raw.total(),
            101,
            "and the line is a line, which is what the per-line number counts"
        );
        assert_eq!(second.duplicates, 1);
    }

    #[test]
    fn the_same_bytes_read_again_add_nothing_on_either_side() {
        // The restart path: a truncated or rewritten transcript, read from byte zero. Both
        // numbers have to stand still, and the per-line one is the new half of that.
        let lines = || {
            vec![
                record("msg_one", Some("req_one"), 651),
                record("msg_one", Some("req_one"), 651),
                record("msg_two", Some("req_two"), 40),
            ]
        };
        let mut first = Deduper::default();
        let one: (u64, u64) = first.reduce(lines()).iter().fold((0, 0), |(d, r), item| {
            (d + item.delta.total(), r + item.raw.total())
        });
        assert_eq!(
            one,
            (693, 1345),
            "two messages deduplicated, three lines raw"
        );

        let mut again = Deduper::with_credited(&first.credited()).rereading(true);
        let two: (u64, u64) = again.reduce(lines()).iter().fold((0, 0), |(d, r), item| {
            (d + item.delta.total(), r + item.raw.total())
        });
        assert_eq!(two, (0, 0), "the same bytes are worth nothing twice");
    }

    #[test]
    fn a_file_that_grew_after_a_restart_credits_only_the_growth() {
        let mut first = Deduper::default();
        first.reduce(vec![
            record("msg_one", Some("req_one"), 100),
            record("msg_one", Some("req_one"), 100),
        ]);

        // Read from the top again, and the file now holds a third copy of that message.
        let mut again = Deduper::with_credited(&first.credited()).rereading(true);
        let out = again.reduce(vec![
            record("msg_one", Some("req_one"), 100),
            record("msg_one", Some("req_one"), 100),
            record("msg_one", Some("req_one"), 100),
        ]);
        let raw: u64 = out.iter().map(|item| item.raw.total()).sum();
        let delta: u64 = out.iter().map(|item| item.delta.total()).sum();
        assert_eq!(raw, 101, "one new line");
        assert_eq!(delta, 0, "and not one new message");
    }

    #[test]
    fn a_file_truncated_and_then_put_back_credits_the_lines_it_lost_only_once() {
        // The sequence a byte offset has no answer to, and the reason a key carries what the
        // file holds beside what it has ever held. Three lines, then one, then three again —
        // and the third pass is an ordinary append, with nothing to say the file ever shrank.
        let three = || {
            vec![
                record("msg_one", Some("req_one"), 100),
                record("msg_one", Some("req_one"), 100),
                record("msg_one", Some("req_one"), 100),
            ]
        };
        let mut first = Deduper::default();
        let start: u64 = first.reduce(three()).iter().map(|it| it.raw.total()).sum();
        assert_eq!(start, 303);

        let mut pruned = Deduper::with_credited(&first.credited()).rereading(true);
        let cut: u64 = pruned
            .reduce(vec![record("msg_one", Some("req_one"), 100)])
            .iter()
            .map(|item| item.raw.total())
            .sum();
        assert_eq!(cut, 0, "a line already counted is not a line");

        let mut back = Deduper::with_credited(&pruned.credited());
        let regrown: u64 = back
            .reduce(vec![
                record("msg_one", Some("req_one"), 100),
                record("msg_one", Some("req_one"), 100),
            ])
            .iter()
            .map(|item| item.raw.total())
            .sum();
        assert_eq!(regrown, 0, "nor are the two lines that came back");

        // And the file growing past what it ever held still credits the growth.
        let mut grown = Deduper::with_credited(&back.credited());
        let more: u64 = grown
            .reduce(vec![record("msg_one", Some("req_one"), 100)])
            .iter()
            .map(|item| item.raw.total())
            .sum();
        assert_eq!(more, 101, "a fourth line is a fourth line");
    }

    #[test]
    fn a_restart_forgets_what_the_file_held_for_a_message_it_no_longer_has() {
        // The other half of the same rule: a key the new file does not mention has nothing
        // in the file, so a later append that brings it back is measured from zero rather
        // than from what the old file held.
        let mut first = Deduper::default();
        first.reduce(vec![
            record("msg_gone", Some("req_gone"), 100),
            record("msg_gone", Some("req_gone"), 100),
        ]);

        let mut pruned = Deduper::with_credited(&first.credited()).rereading(true);
        pruned.reduce(vec![record("msg_other", Some("req_other"), 5)]);

        let mut back = Deduper::with_credited(&pruned.credited());
        let raw: u64 = back
            .reduce(vec![
                record("msg_gone", Some("req_gone"), 100),
                record("msg_gone", Some("req_gone"), 100),
            ])
            .iter()
            .map(|item| item.raw.total())
            .sum();
        assert_eq!(
            raw, 0,
            "both lines are lines this key has already been credited"
        );
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
    fn a_credit_row_is_nine_numbers_and_survives_a_round_trip() {
        let credit = Credit {
            key: 0x0123_4567_89ab_cdef,
            input: 1,
            output: 2,
            cache_create: 3,
            cache_read: 4,
            raw_input: 5,
            raw_output: 6,
            raw_cache_create: 7,
            raw_cache_read: 8,
            file_input: 5,
            file_output: 6,
            file_cache_create: 7,
            file_cache_read: 8,
        };
        let text = serde_json::to_string(&credit).unwrap();
        assert_eq!(
            text, "[81985529216486895,1,2,3,4,5,6,7,8]",
            "a file that holds its own high-water mark writes nine"
        );
        assert_eq!(
            serde_json::from_str::<Credit>(&text).unwrap(),
            credit,
            "the cursor document has one of these per message; it is an array on purpose"
        );
    }

    #[test]
    fn a_row_whose_file_was_truncated_writes_the_other_four_as_well() {
        let credit = Credit {
            key: 9,
            input: 1,
            output: 2,
            cache_create: 0,
            cache_read: 0,
            raw_input: 3,
            raw_output: 6,
            raw_cache_create: 0,
            raw_cache_read: 0,
            file_input: 1,
            file_output: 2,
            file_cache_create: 0,
            file_cache_read: 0,
        };
        let text = serde_json::to_string(&credit).unwrap();
        assert_eq!(text, "[9,1,2,0,0,3,6,0,0,1,2,0,0]");
        assert_eq!(serde_json::from_str::<Credit>(&text).unwrap(), credit);
    }

    #[test]
    fn a_row_of_five_is_read_with_its_per_line_sums_seeded_from_the_largest_copy() {
        // Every cursor on every machine that ran T-WP13 to T-WP21 holds rows of five. The
        // per-line half of them is not recoverable — the bytes are behind a moved cursor —
        // so it is seeded with the largest copy, which is a lower bound and is the same
        // number an absent `raw` on a bucket already means. The two cancel, so a restart
        // after an upgrade credits a file's true per-line sum exactly once.
        let old = serde_json::from_str::<Credit>("[7,1,2,3,4]").unwrap();
        assert_eq!(old.raw(), old.usage());
        assert_eq!(old.in_file(), old.usage());
    }
}
