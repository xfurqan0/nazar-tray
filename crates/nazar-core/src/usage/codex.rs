//! Reading a Codex rollout log for what it **spent**, rather than for what is left.
//!
//! [`crate::codex`] already opens these files: it wants the newest `rate_limits` line and
//! finds it in a 256 KiB window at the end of the file. This is the other pass over the
//! same logs — every `token_count` event in them, from the first line — and the two share
//! nothing but the format they read.
//!
//! ```text
//! {"timestamp":"2026-09-12T18:04:11.221Z","type":"turn_context",
//!  "payload":{"model":"gpt-5.6-sol", … }}
//! {"timestamp":"2026-09-12T18:04:39.550Z","type":"event_msg",
//!  "payload":{"type":"token_count","info":{
//!     "total_token_usage":{ … },
//!     "last_token_usage":{"input_tokens":34012,"cached_input_tokens":33664,
//!                         "cache_write_input_tokens":0,"output_tokens":611, … }}}}
//! ```
//!
//! Like [`super::scan`] and [`crate::codex::parse`], this is an **allow-list rather than a
//! filter**: the line is deserialised into structs that have a field for the seven values
//! named above and a field for nothing else, so the prompt, the reasoning and everything a
//! shell command printed are walked past by serde and never become a `String`. The leak
//! test in `usage::tests` is the proof.
//!
//! # The three things this file exists to get right
//!
//! **1. The per-turn counter, never the cumulative one.** `info.total_token_usage` looks
//! like a session total and is not: it **falls back down mid-session** when the context is
//! compacted or cleared — in 3 of the 22 logs under `sessions/` on the maintainer's machine
//! — so a reader that took the last one would lose everything before the reset. Summing
//! `info.last_token_usage` disagreed with the final cumulative figure in 8 of 30 files,
//! once by a factor of 43. Nothing about a wrong total looks wrong, which is why
//! `fixtures/usage/rollout-reset.jsonl` reproduces a reset and asserts the sum.
//!
//! **2. The model comes from a different line.** A `token_count` event does not name the
//! model that produced it; the session's `turn_context` lines do. The model in force is
//! therefore carried along the file — and across passes, in the cursor, because the bytes
//! that named it are behind the offset. An event that arrives before any `turn_context` is
//! filed under [`UNKNOWN_MODEL`]: those tokens were spent, and which model spent them is
//! the one thing nobody here knows.
//!
//! **3. There is no event id.** Claude Code writes `message.id` and `requestId`, and the
//! transcript reader deduplicates on them. A Codex event carries neither, so *which bytes
//! have been read* is the whole answer: the cursor's `(file identity, byte offset,
//! fingerprint)` is what keeps an event from being counted twice, and it is exact as long as
//! a log is only ever appended to. Two shapes defeat it, and each has a rule:
//!
//! * A **fork or resume that copies a run of events into a new log**, which arrives as a new
//!   path with a cursor that has read nothing. See [`copied_prefix`], and
//!   `rollout-fork.jsonl` for the test.
//! * A log that is **truncated or rewritten**, after which the reader goes back to byte zero
//!   and reads events it has already counted. So the cursor carries a fingerprint of every
//!   event it has credited — `(timestamp to the millisecond, four raw counters)`, which is
//!   the same evidence the fork rule stands on — and a restarted pass matches what it reads
//!   against that set before crediting anything. The set is consumed as it matches, so a log
//!   that genuinely holds two identical events keeps both.
//!
//! # What is not read
//!
//! `$CODEX_HOME/archived_sessions/` holds logs of exactly this shape and **is not walked**.
//! The reason is mechanical rather than squeamish: a cursor is filed under a hash of the
//! path, so a log that Codex *moves* from `sessions/` into `archived_sessions/` arrives as
//! a file nothing has read, and every event in it would be counted a second time. Reading
//! one tree and not the other is what makes archiving a session leave the totals alone. The
//! cost is stated rather than hidden: a session archived before it was ever scanned is
//! never counted at all. `docs/usage-contract.md` carries the decision.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::fmt;

use super::scan::{
    Count, Resume, UNKNOWN_MODEL, Usage, fnv1a, hour_key, identifier_field, read_compressed_lines,
    read_new_lines, timestamp_field,
};
use crate::codex::zst;
use crate::error::Result;

/// The byte strings a rollout line must contain before it is worth parsing.
///
/// Two, because two kinds of line matter and they have nothing in common: the event that
/// carries the counters, and the line that names the model. Everything else in a rollout is
/// a prompt, a reasoning block or the output of a command, and none of it is turned into
/// text. A line that carries one of these strings inside prose still has to get past
/// [`parse_line`], which asks what kind of line it is rather than what it contains.
pub const NEEDLES: [&[u8]; 2] = [b"\"token_count\"", b"\"turn_context\""];

/// The line type that names the model in force.
const TURN_CONTEXT: &str = "turn_context";

/// The payload type that carries the counters.
const TOKEN_COUNT: &str = "token_count";

/// How many of a log's opening events the fork rule compares.
///
/// The cursor now holds every event's fingerprint — it has to, for a log that was truncated
/// or rewritten — so this is no longer a bound on what is stored. It is a bound on the
/// *comparison*: the fork rule asks every log about every other log, and a leading run of
/// thirty-two is far more than any observed fork copied. A copy longer than this is caught
/// for its first thirty-two events and counted twice for the rest, which is the honest limit
/// of a bounded guard and is written down rather than discovered.
pub const PREFIX_EVENTS: usize = 32;

/// How deep the walk goes below `sessions/`.
///
/// The real layout is `YYYY/MM/DD/rollout-….jsonl` — three levels. Six is room for a tree
/// that grows one and a hard stop for one that loops.
const MAX_DEPTH: usize = 6;

/// Prefix and suffix of a rollout file name, the same two [`crate::codex::locate`] uses.
const ROLLOUT_PREFIX: &str = "rollout-";
const ROLLOUT_SUFFIX: &str = ".jsonl";

/// `<home>/.codex` — where Codex keeps everything, unless `CODEX_HOME` says otherwise.
///
/// Derived from the home directory the caller passes rather than from the environment, so
/// a test points a whole scan at a throwaway tree. A caller that honours `CODEX_HOME`
/// resolves it itself and calls [`super::scan_codex_home`]; see [`crate::codex::codex_home`].
#[must_use]
pub fn codex_dir(home: &Path) -> PathBuf {
    home.join(".codex")
}

/// `<codex home>/sessions` — the one tree this reader walks.
#[must_use]
pub fn sessions_dir(codex_home: &Path) -> PathBuf {
    codex_home.join("sessions")
}

/// Every rollout under `root`, plain or compressed, sorted.
///
/// Sorted so that two runs over the same tree read the files in the same order and produce
/// the same counters, and because the date directories are zero padded: sorting the paths
/// as text sorts the sessions as dates, so a log is normally read before anything that
/// forked from it. A missing root is not an error — a machine without Codex has no rollout
/// logs, and that is a state rather than a fault.
#[must_use]
pub fn rollouts(root: &Path) -> Vec<PathBuf> {
    find_rollouts(root).paths
}

/// What the walk under `root` found: the logs, and how many of them were archived.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// Every rollout under `root`, sorted. A `.jsonl.zst` is one of these since T-WP26.
    pub paths: Vec<PathBuf>,
    /// How many of [`Found::paths`] are compressed.
    pub compressed: u64,
}

/// [`rollouts`], and how many of them are archived.
#[must_use]
pub fn find_rollouts(root: &Path) -> Found {
    let mut found = Found::default();
    collect(root, 0, &mut found);
    found.paths.sort();
    found
}

fn collect(dir: &Path, depth: usize, found: &mut Found) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    // The directory is taken whole first, because one decision needs the other names: an
    // archive whose plain twin is beside it is the same session mid-sweep — or a session
    // Codex has just reopened — and counting it as a second log would count the session
    // twice. The plain name wins; it is the one that can still grow.
    let mut names: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.filter_map(std::result::Result::ok) {
        // `DirEntry::file_type` does not follow a symbolic link, so a link that points at
        // an ancestor is never walked into.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            collect(&path, depth + 1, found);
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with(ROLLOUT_PREFIX) {
            continue;
        }
        // `.jsonl.zst` does not end in `.jsonl`, so the two tests are exclusive.
        if !(zst::is_compressed(&name) || name.ends_with(ROLLOUT_SUFFIX)) {
            continue;
        }
        names.push((name, path));
    }

    let plain: BTreeSet<&str> = names
        .iter()
        .filter(|(name, _)| !zst::is_compressed(name))
        .map(|(name, _)| name.as_str())
        .collect();

    for (name, path) in &names {
        if zst::is_compressed(name) {
            if plain.contains(zst::plain_name(name)) {
                continue;
            }
            found.compressed = found.compressed.saturating_add(1);
        }
        found.paths.push(path.clone());
    }
}

/// The path a rollout's cursor is filed under: the name it has when it is not compressed.
///
/// The compression sweep and its undo both **rename** a log, and a cursor keyed on the name
/// on disk would see the archive as a file it had never met — and credit a session it had
/// already counted all over again. Keyed on the plain name, the sweep looks like what it
/// actually is: the same log, replaced. The cursor's identity check notices that by itself,
/// the pass restarts, and the events it has already credited are matched off one by one.
#[must_use]
pub fn cursor_path(path: &Path) -> PathBuf {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.to_path_buf();
    };
    if !zst::is_compressed(name) {
        return path.to_path_buf();
    }
    path.with_file_name(zst::plain_name(name))
}

/// One `token_count` event, reduced to what the store keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The UTC hour the event falls in, `YYYY-MM-DDTHH`, cut from the line's own timestamp.
    pub hour: String,
    /// The model in force when the event was written, or [`UNKNOWN_MODEL`].
    ///
    /// [`parse_line`] cannot know it — the event does not name it — so it writes the
    /// unknown id and [`scan_file`] replaces it with the model the log last named.
    pub model: String,
    /// The four counters, mapped onto the store's.
    pub usage: Usage,
    /// A hash of the line's timestamp and its four raw counters. The fork rule's evidence,
    /// and a hash rather than the values so the cursor document still names nothing.
    pub fingerprint: u64,
}

/// Why a `token_count` event produced nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// No `timestamp` this crate can read, so the event belongs to no hour.
    NoTimestamp,
    /// No `last_token_usage`, or one in which no field carried a number.
    NoNumbers,
}

/// What one line turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A `token_count` event with at least one counter.
    Usage(Event),
    /// A `turn_context` naming the model the following events belong to.
    Model(String),
    /// A `token_count` event this crate could not file.
    Skipped(Skipped),
    /// Neither of those.
    Other,
    /// Not JSON, or not the shape a rollout line has.
    Malformed,
}

/// Read one rollout line.
///
/// Never panics, never invents a number, and never copies text it did not name.
#[must_use]
pub fn parse_line(line: &str) -> Outcome {
    let parsed: Line = match serde_json::from_str(line) {
        Ok(parsed) => parsed,
        Err(_) => return Outcome::Malformed,
    };
    let Some(payload) = parsed.payload else {
        return Outcome::Other;
    };

    // The model, from the line that carries it and from no other. A `model` key anywhere
    // else in a rollout is somebody else's field with a familiar name.
    if parsed.kind.as_deref() == Some(TURN_CONTEXT) {
        return payload.model.map_or(Outcome::Other, Outcome::Model);
    }

    if payload.kind.as_deref() != Some(TOKEN_COUNT) {
        return Outcome::Other;
    }
    let Some(counters) = payload.info.and_then(|info| info.last_token_usage) else {
        return Outcome::Skipped(Skipped::NoNumbers);
    };
    if counters.is_empty() {
        return Outcome::Skipped(Skipped::NoNumbers);
    }
    let Some(at) = parsed.timestamp else {
        return Outcome::Skipped(Skipped::NoTimestamp);
    };
    let Some(hour) = hour_key(&at) else {
        return Outcome::Skipped(Skipped::NoTimestamp);
    };

    Outcome::Usage(Event {
        hour,
        model: UNKNOWN_MODEL.to_owned(),
        usage: counters.usage(),
        fingerprint: counters.fingerprint(&at),
    })
}

/// What one pass over one rollout produced.
#[derive(Debug, Default)]
pub struct FileScan {
    /// The file's identity as it stands now.
    pub identity: String,
    /// Absolute offset of the byte after the last complete line read.
    pub offset: u64,
    /// Hash of the bytes immediately before [`FileScan::offset`]; see
    /// [`super::scan::Resume`].
    pub fingerprint: u64,
    /// `true` when the file was replaced or truncated and the pass started from the top.
    pub restarted: bool,
    /// Bytes read in this pass.
    pub bytes: u64,
    /// The events found, in file order.
    pub events: Vec<Event>,
    /// The model in force at [`FileScan::offset`]: what the next pass has to start with.
    pub model: Option<String>,
    /// Lines that matched a needle.
    pub lines: u64,
    /// Lines that matched a needle and were not a rollout line this crate could read.
    pub malformed: u64,
    /// Events this crate could not file, by reason.
    pub skipped: [u64; 2],
}

impl FileScan {
    fn note(&mut self, reason: Skipped) {
        let at = match reason {
            Skipped::NoTimestamp => 0,
            Skipped::NoNumbers => 1,
        };
        self.skipped[at] += 1;
    }

    /// How many events were skipped for `reason`.
    #[must_use]
    pub fn skipped(&self, reason: Skipped) -> u64 {
        match reason {
            Skipped::NoTimestamp => self.skipped[0],
            Skipped::NoNumbers => self.skipped[1],
        }
    }
}

/// Read everything appended to one rollout since the offset `previous` names.
///
/// `model` is the model the last pass left in force, from the cursor. It is used only when
/// the pass continues where the last one stopped: a file that was replaced or truncated is
/// read from the top again, and the model it names there is the only one that can be right
/// about its first events.
///
/// The bytes, the identity and the partial trailing line are [`super::scan::read_new_lines`]'s
/// work — the same reader the transcripts go through, and the same invariants.
pub fn scan_file(
    path: &Path,
    previous: Option<Resume<'_>>,
    model: Option<&str>,
) -> Result<FileScan> {
    /// One thing a line said, in the order the file said it.
    enum Step {
        Model(String),
        Event(Box<Event>),
    }

    let mut scan = FileScan::default();
    let mut steps: Vec<Step> = Vec::new();
    let mut on_line = |text: &str| match parse_line(text) {
        Outcome::Model(model) => steps.push(Step::Model(model)),
        Outcome::Usage(event) => steps.push(Step::Event(Box::new(event))),
        Outcome::Skipped(reason) => scan.note(reason),
        Outcome::Malformed => scan.malformed += 1,
        Outcome::Other => {}
    };
    // One parser, two readers. An archive cannot be entered in the middle, so it goes
    // through [`read_compressed_lines`], which decodes it whole and leaves a cursor that
    // says so; a plain log goes through the same incremental reader the transcripts use.
    // What reaches `parse_line` is the same lines in the same order either way, and the
    // fixture pair in [`crate::codex::zst`] is the proof.
    let compressed = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(zst::is_compressed);
    let pass = if compressed {
        read_compressed_lines(path, previous, &NEEDLES, &mut on_line)?
    } else {
        read_new_lines(path, previous, &NEEDLES, &mut on_line)?
    };

    // The model is resolved after the read rather than during it, because whether the
    // carried one applies at all is something only the finished pass knows: a restart means
    // these bytes have been read before and the model that was in force then belongs to a
    // file that no longer exists.
    let mut current = if pass.restarted {
        None
    } else {
        model.map(str::to_owned)
    };
    for step in steps {
        match step {
            Step::Model(model) => current = Some(model),
            Step::Event(mut event) => {
                if let Some(model) = &current {
                    event.model.clone_from(model);
                }
                scan.events.push(*event);
            }
        }
    }

    scan.identity = pass.identity;
    scan.offset = pass.offset;
    scan.fingerprint = pass.fingerprint;
    scan.restarted = pass.restarted;
    scan.bytes = pass.bytes;
    scan.lines = pass.lines;
    scan.malformed += pass.malformed;
    scan.model = current;
    Ok(scan)
}

/// How many of a log's opening events are a copy of some other log's opening run.
///
/// **The fork rule.** Codex has no event id, so two identical events are told apart by
/// where they are and nothing else — and a fork or a resume that copies part of a session
/// into a new log moves them somewhere else, under a new path, with a cursor that has read
/// nothing. What such a copy cannot change is the events themselves: a copied event carries
/// the parent's timestamp, to the millisecond, and the parent's four counters. So a log
/// whose **opening run** of events is, event for event, the opening run of a log already
/// known is a copy of it up to the point where the two diverge, and that run is skipped.
///
/// Only a leading run, and only against another log's leading run: a session that happens
/// to contain an event identical to one in the middle of another session is not evidence of
/// anything, while the same event *first in both files* cannot be a coincidence — the
/// timestamp is to the millisecond.
///
/// Which of the two logs keeps the tokens is whichever was read first, and it does not
/// matter: exactly one copy is counted either way.
pub fn copied_prefix<'a>(prefix: &[u64], others: impl Iterator<Item = &'a [u64]>) -> usize {
    others
        .map(|other| {
            prefix
                .iter()
                .zip(other)
                .take_while(|(ours, theirs)| ours == theirs)
                .count()
        })
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The allow-list
// ---------------------------------------------------------------------------

/// The fields of a rollout line this crate knows the name of.
///
/// `ordinal`, and every payload a rollout holds that is not one of the two below, have no
/// field here, which is why they are never built into anything.
#[derive(Debug, Deserialize)]
struct Line {
    #[serde(rename = "type", default, deserialize_with = "identifier_field")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "timestamp_field")]
    timestamp: Option<String>,
    #[serde(default, deserialize_with = "object_field")]
    payload: Option<Payload>,
}

/// The three fields of a payload that carry accounting.
///
/// A `turn_context` payload also holds `cwd`, `workspace_roots`, `personality` and a dozen
/// more; an `event_msg` payload holds whatever its event says. None of them is named here.
#[derive(Debug, Default, Deserialize)]
struct Payload {
    #[serde(rename = "type", default, deserialize_with = "identifier_field")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "identifier_field")]
    model: Option<String>,
    #[serde(default, deserialize_with = "object_field")]
    info: Option<Info>,
}

/// The one field of `info` that is read.
///
/// `total_token_usage` is the cumulative counter that falls back down mid-session, and
/// `model_context_window` is a context size rather than usage. Neither has a field here.
#[derive(Debug, Default, Deserialize)]
struct Info {
    #[serde(default, deserialize_with = "object_field")]
    last_token_usage: Option<Counters>,
}

/// The four counters of one turn, as Codex spells them.
///
/// `reasoning_output_tokens` and `total_tokens` are in this object and are **not** read:
/// the first is already inside `output_tokens` and the second is the sum of the two that
/// are, so either of them added would double a number that is already there. Measured on
/// this machine over 884 events: `input_tokens + output_tokens` equals `total_tokens` on
/// 882 of them — and the two that disagree are the better argument, because a counter this
/// reader does not read cannot make its totals wrong.
#[derive(Debug, Default, Deserialize)]
struct Counters {
    #[serde(default, deserialize_with = "count_field")]
    input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "count_field")]
    cached_input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "count_field")]
    cache_write_input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "count_field")]
    output_tokens: Option<u64>,
}

impl Counters {
    /// `true` when no field carried a number.
    fn is_empty(&self) -> bool {
        self.input_tokens.is_none()
            && self.cached_input_tokens.is_none()
            && self.cache_write_input_tokens.is_none()
            && self.output_tokens.is_none()
    }

    /// These counters as the store spells them.
    ///
    /// One subtraction and one default, both of them decisions:
    ///
    /// * **`input_tokens` includes the cached part**, verified on every event observed —
    ///   `cached_input_tokens` was never larger than it. So the store's `input` is
    ///   `input_tokens − cached_input_tokens`, the part that was genuinely new, and
    ///   `cache_read` is the cached part. Adding the two as reported would count the cache
    ///   twice, and the headline number leaves `cache_read` out precisely so that it does
    ///   not carry a number nobody spent.
    /// * **`cache_write_input_tokens` absent means zero**, which is the one place this crate
    ///   does not read a missing field as unknown: a turn that wrote no cache wrote none.
    ///   It was `0` on every event observed here — all 446 this reader counted, and the 438
    ///   in the archived tree beside them — so a Codex bucket's `cache_create` is 0 in
    ///   practice; but the field is read rather than assumed, so the day Codex starts
    ///   reporting a cache write the store carries it without a change.
    fn usage(&self) -> Usage {
        Usage {
            input: self
                .input_tokens
                .map(|input| input.saturating_sub(self.cached_input_tokens.unwrap_or(0))),
            output: self.output_tokens,
            cache_create: Some(self.cache_write_input_tokens.unwrap_or(0)),
            cache_read: self.cached_input_tokens,
        }
    }

    /// This event's fingerprint: its instant and its four raw counters, hashed.
    ///
    /// The raw counters rather than the mapped ones, so the evidence is what the source
    /// wrote; the instant at full precision, because milliseconds are what make two events
    /// of two different sessions impossible to confuse.
    fn fingerprint(&self, at: &str) -> u64 {
        let mut bytes = Vec::with_capacity(at.len() + 40);
        bytes.extend_from_slice(at.as_bytes());
        for counter in [
            self.input_tokens,
            self.cached_input_tokens,
            self.cache_write_input_tokens,
            self.output_tokens,
        ] {
            match counter {
                Some(value) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                None => bytes.push(0),
            }
        }
        fnv1a(&bytes)
    }
}

/// Read a field when it is an object, and nothing when it is anything else.
///
/// The generic form of `scan`'s `message_field`: a line whose shape surprises us must not
/// take the file down with it, so a payload that is a string, a number or an array is
/// nothing rather than an error.
fn object_field<'de, D, T>(source: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Object<T> {
        type Value = Option<T>;
        fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
            out.write_str("an object")
        }
        fn visit_map<A: MapAccess<'de>>(
            self,
            map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            T::deserialize(de::value::MapAccessDeserializer::new(map)).map(Some)
        }
        fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(
            self,
            source: D2,
        ) -> std::result::Result<Self::Value, D2::Error> {
            source.deserialize_any(Object(std::marker::PhantomData))
        }
        fn visit_str<E: de::Error>(self, _: &str) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(None)
        }
    }
    source.deserialize_any(Object(std::marker::PhantomData))
}

/// Read a field that should hold a token count, or nothing.
///
/// `scan`'s count, so that a string, a float and an integer are read the same way on both
/// sides of the crate, and so that a counter that is not a number is absent rather than a
/// zero somebody could mistake for a measurement.
fn count_field<'de, D: Deserializer<'de>>(source: D) -> std::result::Result<Option<u64>, D::Error> {
    Ok(Count::deserialize(source)?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(line: &str) -> Event {
        match parse_line(line) {
            Outcome::Usage(event) => event,
            other => panic!("expected an event, got {other:?}"),
        }
    }

    const EVENT: &str = r#"{"timestamp":"2026-09-12T18:04:39.550Z","ordinal":12,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":900000,"cached_input_tokens":880000,"output_tokens":9000,"reasoning_output_tokens":4000,"total_tokens":909000},"last_token_usage":{"input_tokens":34012,"cached_input_tokens":33664,"cache_write_input_tokens":0,"output_tokens":611,"reasoning_output_tokens":384,"total_tokens":34623},"model_context_window":258400}}}"#;

    #[test]
    fn the_cached_part_of_the_input_is_taken_out_of_it() {
        let event = event(EVENT);
        assert_eq!(event.hour, "2026-09-12T18");
        assert_eq!(event.usage.input, Some(34_012 - 33_664));
        assert_eq!(event.usage.cache_read, Some(33_664));
        assert_eq!(event.usage.cache_create, Some(0), "Codex reports no writes");
        assert_eq!(
            event.usage.output,
            Some(611),
            "reasoning tokens are inside this and are never added on top"
        );
        assert_eq!(event.model, UNKNOWN_MODEL, "the event does not name one");
    }

    #[test]
    fn the_cumulative_counter_has_no_field_to_arrive_in() {
        // The numbers in `total_token_usage` are an order of magnitude larger than the
        // ones read; if any of them were being read, this would say so.
        let event = event(EVENT);
        assert!(event.usage.total() < 100_000);
    }

    #[test]
    fn a_turn_context_is_where_the_model_comes_from() {
        let line = r#"{"timestamp":"2026-09-12T18:00:00.000Z","type":"turn_context","payload":{"cwd":"/w","model":"gpt-5.6-sol","effort":"high","personality":"none"}}"#;
        assert_eq!(
            parse_line(line),
            Outcome::Model("gpt-5.6-sol".to_owned()),
            "and nothing else in that payload comes out"
        );
    }

    #[test]
    fn a_model_field_outside_a_turn_context_is_not_a_model() {
        let line = r#"{"timestamp":"2026-09-12T18:00:00.000Z","type":"response_item","payload":{"type":"message","model":"gpt-5.6-sol","content":[{"type":"text","text":"token_count"}]}}"#;
        assert_eq!(parse_line(line), Outcome::Other);
    }

    #[test]
    fn prose_in_the_model_field_is_refused_on_its_shape() {
        let line = r#"{"timestamp":"2026-09-12T18:00:00.000Z","type":"turn_context","payload":{"model":"the model we used was the big one, obviously"}}"#;
        assert_eq!(parse_line(line), Outcome::Other);
    }

    #[test]
    fn an_event_with_no_counters_is_named_rather_than_counted() {
        let line = r#"{"timestamp":"2026-09-12T18:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":258400}}}"#;
        assert_eq!(parse_line(line), Outcome::Skipped(Skipped::NoNumbers));
    }

    #[test]
    fn an_event_with_no_readable_timestamp_belongs_to_no_hour() {
        let line = r#"{"timestamp":"half past four","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":2}}}}"#;
        assert_eq!(parse_line(line), Outcome::Skipped(Skipped::NoTimestamp));
    }

    #[test]
    fn broken_json_is_counted_not_guessed_at() {
        assert_eq!(parse_line("{\"type\":\"event_msg\","), Outcome::Malformed);
    }

    #[test]
    fn two_events_written_in_the_same_millisecond_with_the_same_counters_are_one_fingerprint() {
        let one = event(EVENT);
        let two = event(EVENT);
        assert_eq!(one.fingerprint, two.fingerprint);

        let later = EVENT.replace("18:04:39.550Z", "18:04:39.551Z");
        assert_ne!(
            one.fingerprint,
            event(&later).fingerprint,
            "a millisecond is a different event"
        );
    }

    #[test]
    fn a_copied_opening_run_is_measured_and_a_shared_middle_is_not() {
        let parent = [1u64, 2, 3, 4, 5];
        let fork = [1u64, 2, 3, 9, 9];
        assert_eq!(copied_prefix(&fork, [parent.as_slice()].into_iter()), 3);

        let unrelated = [7u64, 1, 2, 3, 4];
        assert_eq!(
            copied_prefix(&unrelated, [parent.as_slice()].into_iter()),
            0,
            "the same events in the middle of a file are not a copied opening"
        );
        assert_eq!(copied_prefix(&parent, std::iter::empty()), 0);
        assert_eq!(copied_prefix(&[], [parent.as_slice()].into_iter()), 0);
    }
}
