//! Walking Claude Code's transcripts, and taking six values out of a line.
//!
//! A transcript line is a whole content block of a conversation: the prompt, the model's
//! reasoning, the text it wrote, the tool calls it made and everything those printed. Six
//! values in it are usage accounting, and they are the only ones this module knows how to
//! name:
//!
//! ```text
//! {"type":"assistant","timestamp":"2026-09-11T15:16:45.816Z","requestId":"req_…",
//!  "message":{"id":"msg_…","model":"claude-opus-5",
//!             "usage":{"input_tokens":2,"output_tokens":328,
//!                      "cache_creation_input_tokens":24843,
//!                      "cache_read_input_tokens":35613}}}
//! ```
//!
//! Like [`crate::codex::parse`], this is an **allow-list rather than a filter**. The line
//! is deserialised into structs that have a field for each of those six values and a field
//! for nothing else, so `message.content` — the prompt and the answer — is walked past by
//! serde and never becomes a `String`, a `Value`, or a borrowed slice that outlives the
//! call. Every string that does come out is shape-checked first. The leak test in
//! `usage::tests` is what proves it: a line whose every text field holds a sentinel
//! produces a record, a summary and a stored document that contain the sentinel nowhere.
//!
//! Three details are ports of what the sibling repository learned the hard way, and each
//! has a test:
//!
//! * **Recursive walk.** `projects/*/*.jsonl` misses the sub-agent transcripts under
//!   `projects/<project>/<session>/subagents/`, which are 78% of the bytes on the
//!   maintainer's machine. The walk is `**/*.jsonl`.
//! * **File identity is birth time, never change time.** On Windows every append moves
//!   `ctime`, so a cursor keyed on it would call every poll of a live transcript a
//!   rotation and read the file from the top again. Birth time plus a hash of the first
//!   512 bytes survives appends and still notices a replacement.
//! * **A partial trailing line is bytes, not text.** A read that lands mid-line may land
//!   mid-UTF-8-sequence. The cursor stops at the last newline; the fragment after it is
//!   read again next time, whole.

use std::fmt;
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::error::{Error, Result};
use crate::timefmt::{rfc3339_from_unix_seconds, unix_seconds_from_rfc3339};

/// The byte string a line must contain before it is worth parsing.
///
/// Two thirds of the lines in a transcript are prompts and tool output and carry no
/// accounting at all; this filter throws them away without building a `String` out of
/// them, which is what keeps a full pass over a few hundred megabytes cheap.
pub const NEEDLE: &[u8] = b"\"input_tokens\"";

/// The model name Claude Code writes for a message the server never billed.
pub const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Longest string accepted out of a transcript field.
///
/// Ported from the sibling repository: a longer value means the field is not the field we
/// think it is, so it is dropped rather than carried.
const MAX_FIELD: usize = 200;

/// Bytes read per `read` call.
const CHUNK: usize = 256 * 1024;

/// Longest line kept while waiting for its newline.
///
/// A transcript line runs to a few hundred kilobytes at the outside. Beyond this the file
/// is not what we think it is, so the fragment is dropped and counted rather than grown.
const MAX_LINE: usize = 8 * 1024 * 1024;

/// Bytes of the head of a file mixed into its identity.
const HEAD_BYTES: usize = 512;

/// The four measurements a usage object carries, each absent until the source reported it.
///
/// Absent is not zero. A bucket whose `input` is `None` was built from records that never
/// named an input count; a bucket whose `input` is `Some(0)` was built from records that
/// said zero. Consumers draw the first as unknown, the way the rest of this crate does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// `message.usage.input_tokens`.
    pub input: Option<u64>,
    /// `message.usage.output_tokens`.
    pub output: Option<u64>,
    /// `message.usage.cache_creation_input_tokens`.
    pub cache_create: Option<u64>,
    /// `message.usage.cache_read_input_tokens`.
    pub cache_read: Option<u64>,
}

impl Usage {
    /// The four measurements added together, treating an absent one as nothing.
    ///
    /// Used only to compare two copies of the same message, never displayed: the headline
    /// number the panel draws leaves `cache_read` out, and this does not.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.input
            .unwrap_or(0)
            .saturating_add(self.output.unwrap_or(0))
            .saturating_add(self.cache_create.unwrap_or(0))
            .saturating_add(self.cache_read.unwrap_or(0))
    }

    /// `true` when no field carried a number.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.input.is_none()
            && self.output.is_none()
            && self.cache_create.is_none()
            && self.cache_read.is_none()
    }

    /// What this reading adds on top of one already credited, field by field.
    ///
    /// A field that went *down* contributes nothing rather than a negative number: the
    /// stored totals are counts of things that happened, and a count cannot be taken back
    /// by a later, smaller reading of the same message. Every copy of a message observed
    /// on this machine carried an identical usage object, and the streaming case the
    /// dedupe rule exists for only ever grows, so this saturation is a guard rather than
    /// a path anything normally takes.
    #[must_use]
    pub fn since(&self, credited: &Usage) -> Usage {
        fn step(now: Option<u64>, before: Option<u64>) -> Option<u64> {
            match (now, before) {
                (None, _) => None,
                (Some(now), None) => Some(now),
                (Some(now), Some(before)) => Some(now.saturating_sub(before)),
            }
        }
        Usage {
            input: step(self.input, credited.input),
            output: step(self.output, credited.output),
            cache_create: step(self.cache_create, credited.cache_create),
            cache_read: step(self.cache_read, credited.cache_read),
        }
    }
}

/// One billable message, reduced to what the store keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The UTC hour the message falls in, `YYYY-MM-DDTHH`. Never a local hour: this crate
    /// stores instants and the panel draws them wherever the reader happens to be.
    pub hour: String,
    /// `message.model`, as the source spelled it. Never normalised, never translated.
    pub model: String,
    /// `message.id`. Half of the dedupe key.
    pub message_id: String,
    /// `requestId`. The other half, absent on a handful of lines per hundred thousand.
    pub request_id: Option<String>,
    /// The four measurements.
    pub usage: Usage,
}

/// Why a line that was shaped like a usage line produced no record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// `model` was `<synthetic>`: a message the server never billed.
    Synthetic,
    /// No `message.model`, so the record could not be attributed to anything. Counted
    /// rather than filed under an invented name.
    NoModel,
    /// No `message.id`, so the record cannot be told apart from its own copies.
    NoMessageId,
    /// No `timestamp` this crate can read, so the record belongs to no hour.
    NoTimestamp,
    /// A `usage` object in which none of the four fields carried a number.
    NoNumbers,
}

/// What one line turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A billable message.
    Usage(Record),
    /// A usage line this crate deliberately does not count.
    Skipped(Skipped),
    /// Not an assistant message with a usage object.
    Other,
    /// Not JSON, or not the shape a transcript line has.
    Malformed,
}

/// Read one transcript line.
///
/// Never panics, never invents a number, and never copies text it did not name.
#[must_use]
pub fn parse_line(line: &str) -> Outcome {
    let parsed: Line = match serde_json::from_str(line) {
        Ok(parsed) => parsed,
        Err(_) => return Outcome::Malformed,
    };
    if !parsed.is_assistant {
        return Outcome::Other;
    }
    let Some(message) = parsed.message else {
        return Outcome::Other;
    };
    let Some(usage) = message.usage else {
        return Outcome::Other;
    };

    let Some(model) = message.model else {
        return Outcome::Skipped(Skipped::NoModel);
    };
    if model == SYNTHETIC_MODEL {
        return Outcome::Skipped(Skipped::Synthetic);
    }
    let Some(message_id) = message.id else {
        return Outcome::Skipped(Skipped::NoMessageId);
    };
    let Some(hour) = parsed.timestamp.as_deref().and_then(hour_key) else {
        return Outcome::Skipped(Skipped::NoTimestamp);
    };
    if usage.is_empty() {
        return Outcome::Skipped(Skipped::NoNumbers);
    }

    Outcome::Usage(Record {
        hour,
        model,
        message_id,
        request_id: parsed.request_id,
        usage,
    })
}

/// The UTC hour an RFC 3339 instant falls in, `YYYY-MM-DDTHH`.
///
/// An instant written with an offset is converted first, so a transcript written on a
/// machine three hours ahead lands in the same hour as one written here. Hours rather
/// than days because a day boundary cannot be re-cut: nothing outside this crate can
/// turn 24 UTC days into 24 days as the reader's calendar has them, and hours can.
#[must_use]
pub fn hour_key(timestamp: &str) -> Option<String> {
    let seconds = unix_seconds_from_rfc3339(timestamp)?;
    let text = rfc3339_from_unix_seconds(seconds.div_euclid(3600) * 3600);
    text.get(..13).map(str::to_owned)
}

/// The UTC month an hour key belongs to, `YYYY-MM`.
#[must_use]
pub fn month_of(hour: &str) -> Option<String> {
    hour.get(..7).map(str::to_owned)
}

// ---------------------------------------------------------------------------
// The allow-list
// ---------------------------------------------------------------------------

/// The fields of a transcript line this crate knows the name of.
///
/// `content`, `cwd`, `gitBranch`, `sessionId`, `uuid`, `toolUseResult` and everything else
/// have no field here, which is why they are never built into anything.
#[derive(Debug, Deserialize)]
struct Line {
    /// `type == "assistant"`, decided while reading so the value is never kept.
    #[serde(rename = "type", default, deserialize_with = "assistant_flag")]
    is_assistant: bool,
    #[serde(default, deserialize_with = "timestamp_field")]
    timestamp: Option<String>,
    #[serde(rename = "requestId", default, deserialize_with = "identifier_field")]
    request_id: Option<String>,
    #[serde(default, deserialize_with = "message_field")]
    message: Option<Message>,
}

/// The three fields of `message` that carry accounting.
#[derive(Debug, Default, Deserialize)]
struct Message {
    #[serde(default, deserialize_with = "identifier_field")]
    id: Option<String>,
    #[serde(default, deserialize_with = "identifier_field")]
    model: Option<String>,
    #[serde(default, deserialize_with = "usage_field")]
    usage: Option<Usage>,
}

/// Read `type` as "is this an assistant message", keeping nothing.
fn assistant_flag<'de, D: Deserializer<'de>>(source: D) -> std::result::Result<bool, D::Error> {
    struct Flag;
    impl<'de> Visitor<'de> for Flag {
        type Value = bool;
        fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
            out.write_str("a line type")
        }
        fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<bool, E> {
            Ok(value == "assistant")
        }
        fn visit_unit<E: de::Error>(self) -> std::result::Result<bool, E> {
            Ok(false)
        }
        fn visit_none<E: de::Error>(self) -> std::result::Result<bool, E> {
            Ok(false)
        }
        fn visit_some<D2: Deserializer<'de>>(
            self,
            source: D2,
        ) -> std::result::Result<bool, D2::Error> {
            source.deserialize_any(Flag)
        }
        fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<bool, E> {
            Ok(false)
        }
        fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<bool, E> {
            Ok(false)
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<bool, E> {
            Ok(false)
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<bool, E> {
            Ok(false)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<bool, A::Error> {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            Ok(false)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<bool, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(false)
        }
    }
    source.deserialize_any(Flag)
}

/// Read a field that should hold an RFC 3339 instant, or nothing.
fn timestamp_field<'de, D: Deserializer<'de>>(
    source: D,
) -> std::result::Result<Option<String>, D::Error> {
    Ok(text_field(source)?.and_then(|text| crate::timefmt::sanitize_timestamp(&text)))
}

/// Read a field that should hold a short identifier, or nothing.
///
/// The character set is closed — letters, digits, and `_ - . : < >` for the two model
/// names that use them — so a field repurposed to hold prose is dropped instead of copied
/// into a document or a tooltip. `<synthetic>` has to survive this check to be recognised
/// and thrown away by name.
fn identifier_field<'de, D: Deserializer<'de>>(
    source: D,
) -> std::result::Result<Option<String>, D::Error> {
    Ok(text_field(source)?.filter(|text| {
        !text.is_empty()
            && text.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'_' | b'-' | b'.' | b':' | b'<' | b'>')
            })
    }))
}

/// Read any field as a short string, or as nothing at all.
///
/// Anything that is not text, and any text longer than [`MAX_FIELD`], yields `None`
/// instead of failing the line: a transcript is somebody else's format and a field that
/// changed shape is not a reason to stop counting the rest of the file.
fn text_field<'de, D: Deserializer<'de>>(
    source: D,
) -> std::result::Result<Option<String>, D::Error> {
    struct Text;
    impl<'de> Visitor<'de> for Text {
        type Value = Option<String>;
        fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
            out.write_str("a short string")
        }
        fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
            if value.len() > MAX_FIELD {
                return Ok(None);
            }
            Ok(Some(value.to_owned()))
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
            source.deserialize_any(Text)
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
        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
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
    source.deserialize_any(Text)
}

/// Read `message` when it is an object, and nothing when it is anything else.
///
/// A user line's `message` is an object too, but a tool result's may not be, and a line
/// whose shape surprises us must not take the file down with it.
fn message_field<'de, D: Deserializer<'de>>(
    source: D,
) -> std::result::Result<Option<Message>, D::Error> {
    struct Object;
    impl<'de> Visitor<'de> for Object {
        type Value = Option<Message>;
        fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
            out.write_str("a message object")
        }
        fn visit_map<A: MapAccess<'de>>(
            self,
            map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            Message::deserialize(de::value::MapAccessDeserializer::new(map)).map(Some)
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
            source.deserialize_any(Object)
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
    source.deserialize_any(Object)
}

/// Read the four top-level measurements out of `usage`, and nothing else out of it.
///
/// `iterations` is the trap this closed list exists for: it is an array of objects that
/// **repeat** the numbers above them, so a reader that added them would double the answer
/// a second time, after dedupe had already fixed the first doubling. It has no arm here,
/// so `next_value::<IgnoredAny>()` walks past it. `cache_creation`, `server_tool_use`,
/// `output_tokens_details` and `service_tier` go the same way.
fn usage_field<'de, D: Deserializer<'de>>(
    source: D,
) -> std::result::Result<Option<Usage>, D::Error> {
    struct Numbers;
    impl<'de> Visitor<'de> for Numbers {
        type Value = Option<Usage>;
        fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
            out.write_str("a usage object")
        }
        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut usage = Usage::default();
            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "input_tokens" => usage.input = map.next_value::<Count>()?.0,
                    "output_tokens" => usage.output = map.next_value::<Count>()?.0,
                    "cache_creation_input_tokens" => {
                        usage.cache_create = map.next_value::<Count>()?.0;
                    }
                    "cache_read_input_tokens" => usage.cache_read = map.next_value::<Count>()?.0,
                    _ => {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
            }
            Ok(Some(usage))
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
            source.deserialize_any(Numbers)
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
    source.deserialize_any(Numbers)
}

/// A count that refuses to become a number when it is not one.
struct Count(Option<u64>);

impl<'de> Deserialize<'de> for Count {
    fn deserialize<D: Deserializer<'de>>(source: D) -> std::result::Result<Self, D::Error> {
        struct Number;
        impl<'de> Visitor<'de> for Number {
            type Value = Option<u64>;
            fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
                out.write_str("a token count")
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(Some(value))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(u64::try_from(value).ok())
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<Self::Value, E> {
                if value.is_finite() && value >= 0.0 {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    return Ok(Some(value as u64));
                }
                Ok(None)
            }
            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
                Ok(value.parse::<u64>().ok())
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<Self::Value, E> {
                Ok(None)
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
                source.deserialize_any(Number)
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
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
        source.deserialize_any(Number).map(Count)
    }
}

// ---------------------------------------------------------------------------
// The walk and the incremental read
// ---------------------------------------------------------------------------

/// How deep the walk goes below `projects/`.
///
/// A sub-agent transcript is three levels down. Ten is room for a nesting nobody has
/// shipped yet and a hard stop for a directory tree that loops.
const MAX_DEPTH: usize = 10;

/// Every `*.jsonl` under `root`, including the sub-agent transcripts, sorted.
///
/// Sorted so that two runs over the same tree produce the same order and therefore the
/// same counters — a scan that is not reproducible cannot be tested against a fixture.
/// A missing root is not an error: a machine without Claude Code has no transcripts, and
/// that is a state, not a fault.
pub fn transcripts(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(root, 0, &mut found);
    found.sort();
    found
}

fn collect(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        // `DirEntry::file_type` does not follow a symbolic link, so a link that points at
        // an ancestor is never walked into.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            collect(&path, depth + 1, found);
        } else if kind.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        {
            found.push(path);
        }
    }
}

/// What one pass over one file produced.
#[derive(Debug, Default)]
pub struct FileScan {
    /// The file's identity as it stands now. Stored with the offset; a change means the
    /// file was replaced and the offset means nothing any more.
    pub identity: String,
    /// Absolute offset of the byte after the last complete line read. A trailing fragment
    /// is deliberately left behind it.
    pub offset: u64,
    /// `true` when the file was replaced or truncated and the pass started from the top.
    pub restarted: bool,
    /// Bytes read in this pass.
    pub bytes: u64,
    /// The billable messages found, in file order.
    pub records: Vec<Record>,
    /// Lines that matched the needle.
    pub lines: u64,
    /// Lines that matched the needle and were not JSON.
    pub malformed: u64,
    /// Lines that were usage lines this crate deliberately does not count, by reason.
    pub skipped: [u64; 5],
}

impl FileScan {
    fn note(&mut self, reason: Skipped) {
        let at = match reason {
            Skipped::Synthetic => 0,
            Skipped::NoModel => 1,
            Skipped::NoMessageId => 2,
            Skipped::NoTimestamp => 3,
            Skipped::NoNumbers => 4,
        };
        self.skipped[at] += 1;
    }

    /// How many lines were skipped for `reason`.
    #[must_use]
    pub fn skipped(&self, reason: Skipped) -> u64 {
        match reason {
            Skipped::Synthetic => self.skipped[0],
            Skipped::NoModel => self.skipped[1],
            Skipped::NoMessageId => self.skipped[2],
            Skipped::NoTimestamp => self.skipped[3],
            Skipped::NoNumbers => self.skipped[4],
        }
    }
}

/// Read everything appended to `path` since the offset `previous` names.
///
/// `previous` is `(identity, offset)` as the last pass left them. When the identity no
/// longer matches — the file was replaced, or rotated, or the offset is past the end
/// because it was truncated — the pass starts at byte zero and says so, and the caller
/// is expected to throw away whatever it had credited for that file's identity.
///
/// The length is pinned at the `metadata` call: bytes that arrive while this is reading
/// are the next pass's work, so a live transcript can be polled without ever reading a
/// line that is still being written.
pub fn scan_file(path: &Path, previous: Option<(&str, u64)>) -> Result<FileScan> {
    let metadata = std::fs::metadata(path).map_err(|source| Error::io(path, source))?;
    let length = metadata.len();

    let mut file = File::open(path).map_err(|source| Error::io(path, source))?;
    let mut head = [0u8; HEAD_BYTES];
    let filled = read_head(&mut file, &mut head).map_err(|source| Error::io(path, source))?;
    let fresh_window = HEAD_BYTES.min(usize::try_from(length).unwrap_or(HEAD_BYTES));

    // The head window is fixed the first time a file is measured and never widened. A
    // transcript that was two hundred bytes long when it was first seen is still hashed
    // over those two hundred bytes when it is two megabytes long, because otherwise every
    // append to a young file would change its identity and be read as a rotation.
    let (identity, start, restarted) = match previous {
        Some((known, offset)) => {
            let window = window_of(known).unwrap_or(fresh_window);
            let candidate = identity_of(&metadata, &head[..window.min(filled)], window);
            if candidate == known && offset <= length {
                (candidate, offset, false)
            } else {
                (
                    identity_of(&metadata, &head[..fresh_window.min(filled)], fresh_window),
                    0,
                    true,
                )
            }
        }
        None => (
            identity_of(&metadata, &head[..fresh_window.min(filled)], fresh_window),
            0,
            false,
        ),
    };

    let mut scan = FileScan {
        identity,
        offset: start,
        restarted,
        ..FileScan::default()
    };
    if start >= length {
        return Ok(scan);
    }

    file.seek(SeekFrom::Start(start))
        .map_err(|source| Error::io(path, source))?;

    let table = skip_table(NEEDLE);
    let mut buffer = vec![0u8; CHUNK];
    let mut carry: Vec<u8> = Vec::new();
    let mut discarding = false;
    let mut remaining = length - start;
    let mut chunk_start = start;

    while remaining > 0 {
        let want = CHUNK.min(usize::try_from(remaining).unwrap_or(CHUNK));
        let read = file
            .read(&mut buffer[..want])
            .map_err(|source| Error::io(path, source))?;
        if read == 0 {
            break;
        }
        remaining -= read as u64;
        scan.bytes += read as u64;

        let mut rest = &buffer[..read];
        while let Some(index) = memchr(rest, b'\n') {
            let (line, tail) = rest.split_at(index);
            rest = &tail[1..];
            scan.offset = chunk_start + (read - rest.len()) as u64;

            if discarding {
                discarding = false;
                continue;
            }
            if carry.is_empty() {
                absorb(line, &table, &mut scan);
            } else {
                carry.extend_from_slice(line);
                let joined = std::mem::take(&mut carry);
                absorb(&joined, &table, &mut scan);
            }
        }

        if !rest.is_empty() {
            if discarding || carry.len() + rest.len() > MAX_LINE {
                // Not a line we could use even if it ever ended. Step the cursor past it
                // so the next pass does not read the same eight megabytes again.
                if !discarding {
                    scan.malformed += 1;
                }
                carry.clear();
                discarding = true;
                scan.offset = chunk_start + read as u64;
            } else {
                carry.extend_from_slice(rest);
            }
        }
        chunk_start += read as u64;
    }

    Ok(scan)
}

/// Parse one complete line, if it is worth parsing.
fn absorb(line: &[u8], table: &[usize; 256], scan: &mut FileScan) {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() || !contains(line, NEEDLE, table) {
        return;
    }
    scan.lines += 1;
    let Ok(text) = std::str::from_utf8(line) else {
        // A transcript with bytes that are not UTF-8 is a transcript we do not understand.
        // Counted, never guessed at: a lossy conversion would hand the parser invented
        // characters and the parser would hand us invented numbers.
        scan.malformed += 1;
        return;
    };
    match parse_line(text) {
        Outcome::Usage(record) => scan.records.push(record),
        Outcome::Skipped(reason) => scan.note(reason),
        Outcome::Malformed => scan.malformed += 1,
        Outcome::Other => {}
    }
}

/// Read up to `head.len()` bytes from the start of a file.
fn read_head(file: &mut File, head: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < head.len() {
        let read = file.read(&mut head[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// A string that identifies this file among the files that have had this path.
///
/// Three parts, and the middle one is the point. **Birth time, never change time**: on
/// Windows every append moves the change time, so a cursor keyed on it would see a
/// rotation every time a session wrote a line and would read the whole transcript again
/// on every poll. Birth time does not move while a file is appended to and does move when
/// a file is replaced, which is exactly the question being asked.
///
/// The inode number joins it where the platform has one. The hash of the head joins both,
/// so that a filesystem that reports neither — and a copy made by a tool that preserved
/// the birth time — is still told apart by its contents. The width of that head window is
/// part of the identity so that later passes hash the same bytes.
fn identity_of(metadata: &Metadata, head: &[u8], window: usize) -> String {
    let born = metadata
        .created()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(|| "-".to_owned(), |since| since.as_millis().to_string());

    #[cfg(unix)]
    let node = {
        use std::os::unix::fs::MetadataExt;
        metadata.ino().to_string()
    };
    #[cfg(not(unix))]
    let node = "-".to_owned();

    format!("{node}:{born}:{window}:{:016x}", fnv1a(head))
}

/// The head window an identity was built with.
fn window_of(identity: &str) -> Option<usize> {
    identity.split(':').nth(2)?.parse().ok()
}

/// FNV-1a, 64 bit. Small, fast, and not a checksum anybody has to agree with.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Index of the first `byte` in `haystack`.
fn memchr(haystack: &[u8], byte: u8) -> Option<usize> {
    haystack.iter().position(|candidate| *candidate == byte)
}

/// Boyer–Moore–Horspool skip table for `needle`.
fn skip_table(needle: &[u8]) -> [usize; 256] {
    let mut table = [needle.len(); 256];
    for (at, byte) in needle.iter().enumerate().take(needle.len() - 1) {
        table[*byte as usize] = needle.len() - 1 - at;
    }
    table
}

/// Whether `haystack` contains `needle`, skipping ahead on a mismatch.
///
/// A plain window-by-window comparison costs a comparison per byte of a few hundred
/// megabytes; skipping on the last byte's table entry costs roughly one per needle length.
fn contains(haystack: &[u8], needle: &[u8], table: &[usize; 256]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    let last = needle.len() - 1;
    let mut at = 0;
    while at + last < haystack.len() {
        let candidate = haystack[at + last];
        if candidate == needle[last] && &haystack[at..at + needle.len()] == needle {
            return true;
        }
        at += table[candidate as usize];
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::io::Write;

    fn line(hour: &str, model: &str, id: &str, request: &str, numbers: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{hour}","requestId":"{request}","message":{{"id":"{id}","model":"{model}","usage":{numbers}}}}}"#
        )
    }

    fn usage(line: &str) -> Record {
        match parse_line(line) {
            Outcome::Usage(record) => record,
            other => panic!("expected a usage record, got {other:?}"),
        }
    }

    #[test]
    fn the_six_values_come_out_and_nothing_else_does() {
        let text = line(
            "2026-09-11T15:16:45.816Z",
            "claude-opus-5",
            "msg_one",
            "req_one",
            r#"{"input_tokens":2,"output_tokens":328,"cache_creation_input_tokens":24843,"cache_read_input_tokens":35613}"#,
        );
        let record = usage(&text);
        assert_eq!(record.hour, "2026-09-11T15");
        assert_eq!(record.model, "claude-opus-5");
        assert_eq!(record.message_id, "msg_one");
        assert_eq!(record.request_id.as_deref(), Some("req_one"));
        assert_eq!(record.usage.input, Some(2));
        assert_eq!(record.usage.output, Some(328));
        assert_eq!(record.usage.cache_create, Some(24843));
        assert_eq!(record.usage.cache_read, Some(35613));
    }

    #[test]
    fn an_hour_is_cut_in_utc_whatever_offset_the_source_wrote() {
        // 01:30 three hours ahead is 22:30 the day before, and that is the hour it lands in.
        assert_eq!(
            hour_key("2026-09-12T01:30:00+03:00").as_deref(),
            Some("2026-09-11T22")
        );
        assert_eq!(
            hour_key("2026-09-11T23:59:59.999Z").as_deref(),
            Some("2026-09-11T23")
        );
        assert_eq!(hour_key("half past four"), None);
    }

    #[test]
    fn iterations_repeat_the_numbers_above_them_and_are_never_read() {
        let text = line(
            "2026-09-11T15:00:00Z",
            "claude-opus-5",
            "msg_two",
            "req_two",
            r#"{"input_tokens":10,"output_tokens":20,"iterations":[{"input_tokens":10,"output_tokens":20},{"input_tokens":10,"output_tokens":20}]}"#,
        );
        let record = usage(&text);
        assert_eq!(record.usage.input, Some(10));
        assert_eq!(record.usage.output, Some(20));
    }

    #[test]
    fn a_synthetic_message_is_named_and_dropped() {
        let text = line(
            "2026-09-11T15:00:00Z",
            "<synthetic>",
            "msg_three",
            "req_three",
            r#"{"input_tokens":0,"output_tokens":0}"#,
        );
        assert_eq!(parse_line(&text), Outcome::Skipped(Skipped::Synthetic));
    }

    #[test]
    fn a_line_that_is_not_an_assistant_message_is_not_a_fault() {
        let text = r#"{"type":"user","timestamp":"2026-09-11T15:00:00Z","message":{"role":"user","content":"input_tokens are on my mind"}}"#;
        assert_eq!(parse_line(text), Outcome::Other);
    }

    #[test]
    fn a_message_that_is_not_an_object_does_not_take_the_line_down() {
        let text =
            r#"{"type":"assistant","timestamp":"2026-09-11T15:00:00Z","message":"input_tokens"}"#;
        assert_eq!(parse_line(text), Outcome::Other);
    }

    #[test]
    fn a_field_that_changed_shape_is_dropped_rather_than_guessed_at() {
        let text = r#"{"type":"assistant","timestamp":404,"requestId":{"was":"a string"},"message":{"id":"msg_four","model":"claude-sonnet-5","usage":{"input_tokens":"7","output_tokens":null}}}"#;
        assert_eq!(parse_line(text), Outcome::Skipped(Skipped::NoTimestamp));
    }

    #[test]
    fn prose_in_an_identifier_field_is_refused() {
        let text = line(
            "2026-09-11T15:00:00Z",
            "a model with spaces",
            "msg_five",
            "req_five",
            r#"{"input_tokens":1}"#,
        );
        assert_eq!(parse_line(&text), Outcome::Skipped(Skipped::NoModel));
    }

    #[test]
    fn broken_json_is_counted_not_guessed_at() {
        assert_eq!(parse_line(r#"{"input_tokens":"#), Outcome::Malformed);
    }

    #[test]
    fn the_walk_reaches_the_sub_agent_transcripts() {
        let dir = TempDir::new("usage-walk");
        let deep = dir.join("project/session-one/subagents");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(dir.join("project/top.jsonl"), b"{}\n").unwrap();
        std::fs::write(deep.join("agent-one.jsonl"), b"{}\n").unwrap();
        std::fs::write(dir.join("project/notes.txt"), b"not a transcript").unwrap();

        let found = transcripts(&dir.path);
        assert_eq!(found.len(), 2, "found {found:?}");
        assert!(found.iter().any(|path| path.ends_with("agent-one.jsonl")));
    }

    #[test]
    fn a_missing_root_is_a_state_not_a_fault() {
        let dir = TempDir::new("usage-walk-missing");
        assert!(transcripts(&dir.join("never-created")).is_empty());
    }

    #[test]
    fn a_second_pass_reads_only_what_arrived_since_the_first() {
        let dir = TempDir::new("usage-incremental");
        let path = dir.join("transcript.jsonl");
        let first = line(
            "2026-09-11T15:00:00Z",
            "claude-opus-5",
            "msg_a",
            "req_a",
            r#"{"input_tokens":1}"#,
        );
        std::fs::write(&path, format!("{first}\n")).unwrap();

        let one = scan_file(&path, None).unwrap();
        assert_eq!(one.records.len(), 1);
        assert!(!one.restarted);

        let second = line(
            "2026-09-11T16:00:00Z",
            "claude-opus-5",
            "msg_b",
            "req_b",
            r#"{"input_tokens":2}"#,
        );
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(format!("{second}\n").as_bytes()).unwrap();
        drop(file);

        let two = scan_file(&path, Some((one.identity.as_str(), one.offset))).unwrap();
        assert_eq!(two.records.len(), 1);
        assert_eq!(two.records[0].message_id, "msg_b");
    }

    #[test]
    fn a_line_still_being_written_is_left_for_the_next_pass() {
        let dir = TempDir::new("usage-partial");
        let path = dir.join("transcript.jsonl");
        let whole = line(
            "2026-09-11T15:00:00Z",
            "claude-opus-5",
            "msg_partial",
            "req_partial",
            r#"{"input_tokens":5}"#,
        );
        let (head, tail) = whole.split_at(whole.len() / 2);
        std::fs::write(&path, head).unwrap();

        let one = scan_file(&path, None).unwrap();
        assert!(one.records.is_empty());
        assert_eq!(one.offset, 0, "the cursor must stop before the fragment");

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(format!("{tail}\n").as_bytes()).unwrap();
        drop(file);

        let two = scan_file(&path, Some((one.identity.as_str(), one.offset))).unwrap();
        assert_eq!(two.records.len(), 1);
        assert_eq!(two.records[0].message_id, "msg_partial");
    }

    #[test]
    fn a_truncated_file_is_read_from_the_top_and_says_so() {
        let dir = TempDir::new("usage-truncated");
        let path = dir.join("transcript.jsonl");
        let text = line(
            "2026-09-11T15:00:00Z",
            "claude-opus-5",
            "msg_t",
            "req_t",
            r#"{"input_tokens":1}"#,
        );
        std::fs::write(&path, format!("{text}\n{text}\n")).unwrap();
        let one = scan_file(&path, None).unwrap();

        std::fs::write(&path, format!("{text}\n")).unwrap();
        let two = scan_file(&path, Some((one.identity.as_str(), one.offset))).unwrap();
        assert!(two.restarted);
        assert_eq!(two.records.len(), 1);
    }

    #[test]
    fn a_line_without_a_trailing_newline_at_the_end_of_a_chunk_still_joins_up() {
        let dir = TempDir::new("usage-chunk");
        let path = dir.join("transcript.jsonl");
        // Longer than one chunk, so the reader has to carry a fragment across a read.
        let padding = "x".repeat(CHUNK);
        let text = format!(
            r#"{{"type":"assistant","timestamp":"2026-09-11T15:00:00Z","requestId":"req_big","note":"{padding}","message":{{"id":"msg_big","model":"claude-opus-5","usage":{{"input_tokens":9}}}}}}"#
        );
        std::fs::write(&path, format!("{text}\n")).unwrap();

        let scan = scan_file(&path, None).unwrap();
        assert_eq!(scan.records.len(), 1);
        assert_eq!(scan.records[0].usage.input, Some(9));
    }

    #[test]
    fn the_skip_search_finds_what_a_plain_one_would() {
        let table = skip_table(NEEDLE);
        assert!(contains(br#"{"input_tokens":2}"#, NEEDLE, &table));
        assert!(!contains(br#"{"output_tokens":2}"#, NEEDLE, &table));
        assert!(!contains(b"", NEEDLE, &table));
        assert!(contains(
            br#"{"a":1,"b":2,"usage":{"input_tokens":0}}"#,
            NEEDLE,
            &table
        ));
    }
}
