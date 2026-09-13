//! The usage reader, run against transcripts shaped like the ones a real installation
//! writes.
//!
//! The fixtures under `crates/nazar-core/fixtures/usage/` are synthetic — no line of any
//! of them came off a machine — but every shape in them was observed on one: the several
//! lines per message, the `iterations` array that repeats the numbers above it, the
//! `<synthetic>` model, the offset-bearing timestamp, the line that was still being
//! written. Each file is one claim, and each test below is that claim.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{
    Bucket, PROVIDER, UNKNOWN_MODEL, UsageSummary, projects_dir, query, scan, scan_claude,
    store::{self, Month, Read},
};
use crate::testutil::TempDir;

/// The fixtures directory.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("usage")
}

/// One fixture's text.
fn fixture(name: &str) -> String {
    let path = fixtures().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// A throwaway machine: a home directory with transcripts, and a state directory.
struct Machine {
    dir: TempDir,
}

impl Machine {
    fn new(label: &str) -> Self {
        Machine {
            dir: TempDir::new(label),
        }
    }

    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    fn state(&self) -> PathBuf {
        self.dir.join("state")
    }

    /// Put text at `relative` under `projects/`, creating the directories on the way.
    fn put(&self, relative: &str, text: &str) -> PathBuf {
        let path = projects_dir(&self.home()).join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn scan(&self) -> UsageSummary {
        scan_claude(&self.home(), &self.state()).unwrap()
    }

    fn month(&self, month: &str) -> Month {
        match store::read_month(&self.state(), month).unwrap() {
            Read::Document(document) => *document,
            other => panic!("no document for {month}: {other:?}"),
        }
    }

    fn bucket(&self, month: &str, hour: &str, model: &str) -> Bucket {
        self.month(month)
            .buckets(PROVIDER)
            .and_then(|hours| hours.get(hour))
            .and_then(|models| models.get(model))
            .cloned()
            .unwrap_or_else(|| panic!("no bucket for {hour} / {model}"))
    }

    /// Every byte this machine has written under `usage/`.
    fn documents(&self) -> Vec<(String, String)> {
        let dir = store::usage_dir(&self.state());
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return found;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(text) = std::fs::read_to_string(entry.path()) {
                found.push((name, text));
            }
        }
        found.sort();
        found
    }
}

fn bucket(input: u64, output: u64, cache_create: u64, cache_read: u64, requests: u64) -> Bucket {
    Bucket {
        input: Some(input),
        output: Some(output),
        cache_create: Some(cache_create),
        cache_read: Some(cache_read),
        requests,
        extra: serde_json::Map::new(),
    }
}

// ---------------------------------------------------------------------------
// (a) known totals
// ---------------------------------------------------------------------------

#[test]
fn a_fixture_with_known_totals_adds_up_to_them() {
    let machine = Machine::new("usage-known");
    machine.put("a-project/session-a.jsonl", &fixture("known-totals.jsonl"));

    let summary = machine.scan();
    assert_eq!(summary.files_seen, 1);
    assert_eq!(summary.credited, 6, "six billable messages");
    assert_eq!(summary.duplicates, 0);
    assert_eq!(summary.malformed, 0);

    assert_eq!(
        machine.bucket("2026-08", "2026-08-31T23", "claude-opus-5"),
        bucket(10, 100, 1000, 10_000, 1)
    );
    assert_eq!(
        machine.bucket("2026-08", "2026-08-31T23", "claude-fable-5-1"),
        bucket(20, 200, 2000, 20_000, 1)
    );
    // Two messages an hour apart from each other, in the same hour.
    assert_eq!(
        machine.bucket("2026-09", "2026-09-01T00", "claude-opus-5"),
        bucket(70, 700, 7000, 70_000, 2)
    );
    assert_eq!(
        machine.bucket("2026-09", "2026-09-01T01", "claude-fable-5-1"),
        bucket(50, 500, 5000, 50_000, 1)
    );
    // Written `07:30+03:00`, which is `04:30` in the only time zone this crate has.
    assert_eq!(
        machine.bucket("2026-09", "2026-09-01T04", "claude-opus-5"),
        bucket(60, 600, 6000, 60_000, 1)
    );
    assert_eq!(summary.credited_total, 233_310);
    assert_eq!(summary.naive_total, summary.credited_total);
}

#[test]
fn the_walk_counts_the_sub_agent_transcripts_too() {
    let machine = Machine::new("usage-subagents");
    machine.put("a-project/session-a.jsonl", &fixture("known-totals.jsonl"));
    machine.put(
        "a-project/session-a/subagents/agent-one.jsonl",
        &fixture("duplicates.jsonl"),
    );

    let summary = machine.scan();
    assert_eq!(
        summary.files_seen, 2,
        "the deeper transcript is 78% of a real machine's bytes"
    );
    assert_eq!(summary.credited, 8);
}

// ---------------------------------------------------------------------------
// (b) duplicates
// ---------------------------------------------------------------------------

#[test]
fn the_copies_one_message_is_written_on_inflate_the_answer_by_nothing() {
    let machine = Machine::new("usage-duplicates");
    machine.put("b-project/session-b.jsonl", &fixture("duplicates.jsonl"));

    let summary = machine.scan();
    assert_eq!(summary.lines, 5, "five lines carrying usage");
    assert_eq!(summary.credited, 2, "two messages");
    assert_eq!(summary.duplicates, 3);

    // `msg_dup` is one message written on three blocks; `msg_grow` is a partial view of a
    // message followed by the whole one, and the larger copy is the one that counts.
    assert_eq!(
        machine.bucket("2026-09", "2026-09-02T12", "claude-opus-5"),
        bucket(6, 649_213, 50, 500, 2)
    );
    assert_eq!(summary.credited_total, 649_769);
    assert_eq!(
        summary.naive_total, 782_967,
        "what the same lines would have claimed with no dedupe"
    );
}

// ---------------------------------------------------------------------------
// (c) what is deliberately not counted
// ---------------------------------------------------------------------------

#[test]
fn synthetic_messages_and_iteration_copies_are_not_counted() {
    let machine = Machine::new("usage-ignored");
    machine.put("c-project/session-c.jsonl", &fixture("ignored.jsonl"));

    let summary = machine.scan();
    assert_eq!(summary.synthetic, 1, "a message the server never billed");
    assert_eq!(
        summary.malformed, 1,
        "a truncated line, counted not guessed at"
    );
    assert_eq!(
        summary.unattributable, 1,
        "the line with no message id cannot be told apart from its own copies"
    );
    assert_eq!(summary.unnamed_model, 1);
    assert_eq!(summary.credited, 2);

    let counted = machine.bucket("2026-09", "2026-09-03T08", "claude-opus-5");
    assert_eq!(counted.input, Some(7));
    assert_eq!(counted.output, Some(70));
    assert_eq!(
        counted.cache_read, None,
        "the 700 lives only inside iterations[], which is never read"
    );
    assert_eq!(counted.requests, 1);

    // The line whose source named no model keeps its tokens under the one id this
    // reader writes itself, rather than losing them to a missing field.
    let unnamed = machine.bucket("2026-09", "2026-09-03T08", UNKNOWN_MODEL);
    assert_eq!(unnamed.input, Some(321));
    assert_eq!(unnamed.output, Some(654));

    // The nine-hundred-thousands of the synthetic line reached nothing.
    assert_eq!(summary.credited_total, 1052);
}

// ---------------------------------------------------------------------------
// (d) a line that was still being written
// ---------------------------------------------------------------------------

#[test]
fn a_transcript_caught_mid_line_is_finished_on_the_next_scan() {
    let machine = Machine::new("usage-partial");
    let text = fixture("known-totals.jsonl");
    let cut = text.len() - 60;
    let path = machine.put("d-project/session-d.jsonl", &text[..cut]);

    let first = machine.scan();
    assert_eq!(first.credited, 5, "the last line is not a line yet");

    std::fs::write(&path, &text).unwrap();
    let second = machine.scan();
    assert_eq!(second.credited, 1, "and now it is");
    assert_eq!(
        machine.bucket("2026-09", "2026-09-01T04", "claude-opus-5"),
        bucket(60, 600, 6000, 60_000, 1),
        "counted once, with the numbers of the whole line"
    );
}

// ---------------------------------------------------------------------------
// (e) the leak test
// ---------------------------------------------------------------------------

/// The one that matters.
///
/// The fixture line holds a sentinel in every string it has: in `message.content`, which
/// is the prompt and the answer; in `cwd`, `gitBranch`, `sessionId` and `uuid`, which are
/// identity; in `toolUseResult`, which is whatever a command printed; and in `requestId`,
/// which *is* allow-listed and therefore has to be refused on its shape instead.
///
/// Nothing this module produces may contain either of them: not the parsed record, not
/// the summary, and not a byte of anything written to disk.
#[test]
fn nothing_but_the_allow_listed_values_leaves_the_reader() {
    const CONTENT: &str = "SENTINEL-content-do-not-leak-7c41";
    const FIELD: &str = "SENTINEL field do not leak 9b2e";

    let text = fixture("sentinel.jsonl");
    assert!(
        text.contains(CONTENT) && text.contains(FIELD),
        "the fixture lost its sentinels"
    );

    let outcome = scan::parse_line(text.trim_end());
    let parsed = format!("{outcome:?}");
    assert!(
        !parsed.contains(CONTENT),
        "the parsed record carries the content sentinel"
    );
    assert!(
        !parsed.contains(FIELD),
        "the parsed record carries the field sentinel"
    );
    match outcome {
        scan::Outcome::Usage(record) => {
            assert_eq!(record.message_id, "msg_sentinel");
            assert_eq!(record.model, "claude-haiku-4-5-20251001");
            assert_eq!(
                record.request_id, None,
                "an allow-listed field holding prose is refused on its shape"
            );
        }
        other => panic!("expected a usage record, got {other:?}"),
    }

    let machine = Machine::new("usage-sentinel");
    machine.put("e-project/session-e.jsonl", &text);
    let summary = machine.scan();

    let printed = format!("{summary:?}");
    assert!(!printed.contains(CONTENT) && !printed.contains(FIELD));
    let serialised = serde_json::to_string(&summary).unwrap();
    assert!(!serialised.contains(CONTENT) && !serialised.contains(FIELD));

    let documents = machine.documents();
    assert!(
        documents.len() >= 2,
        "a month document and a cursor document"
    );
    for (name, body) in &documents {
        assert!(
            !body.contains(CONTENT),
            "{name} carries the content sentinel"
        );
        assert!(!body.contains(FIELD), "{name} carries the field sentinel");
    }

    // And the numbers still arrived, so this is a test of a reader that works.
    assert_eq!(
        machine.bucket("2026-09", "2026-09-04T09", "claude-haiku-4-5-20251001"),
        bucket(3, 33, 333, 3333, 1)
    );

    let view = query(
        &machine.state(),
        "2026-09-01T00:00:00Z",
        "2026-10-01T00:00:00Z",
    )
    .unwrap();
    let drawn = serde_json::to_string(&view).unwrap();
    assert!(!drawn.contains(CONTENT) && !drawn.contains(FIELD));
}

// ---------------------------------------------------------------------------
// (f) idempotence
// ---------------------------------------------------------------------------

#[test]
fn scanning_the_same_transcripts_again_adds_nothing() {
    let machine = Machine::new("usage-idempotent");
    machine.put("f-project/session-f.jsonl", &fixture("known-totals.jsonl"));
    machine.put(
        "f-project/session-f/subagents/agent-one.jsonl",
        &fixture("duplicates.jsonl"),
    );

    let first = machine.scan();
    let after_first = machine.documents();

    let second = machine.scan();
    assert_eq!(second.credited, 0);
    assert_eq!(second.lines, 0, "nothing was read a second time");
    assert_eq!(second.bytes_read, 0);
    assert_eq!(second.months, Vec::<String>::new());

    let after_second = machine.documents();
    assert_eq!(
        after_first, after_second,
        "a rescan must leave the documents byte for byte as they were"
    );
    assert_eq!(first.credited, 8);
}

#[test]
fn a_commit_that_was_never_filed_is_filed_once_when_the_next_scan_finds_it() {
    let machine = Machine::new("usage-replay");
    machine.put("g-project/session-g.jsonl", &fixture("known-totals.jsonl"));
    machine.scan();
    let before = machine.documents();

    // What the disk looks like after a crash between the commit and the filing: the
    // cursor still carries the block, and the month already carries its generation.
    let september = machine.month("2026-09");
    let totals = &september.providers[PROVIDER];
    let mut cursors = store::read_cursors(&machine.state()).unwrap();
    let mut months = super::Months::new();
    months.insert("2026-09".to_owned(), totals.buckets.clone());
    cursors.pending = Some(store::Pending {
        provider: PROVIDER.to_owned(),
        generation: totals.applied_through,
        scanned_at: september.scanned_at.clone().unwrap(),
        months,
    });
    store::write_cursors(&machine.state(), &cursors).unwrap();

    machine.scan();
    let after = machine.documents();
    let numbers = |documents: &Vec<(String, String)>| {
        documents
            .iter()
            .filter(|(name, _)| name.starts_with("2026-"))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        numbers(&before),
        numbers(&after),
        "replaying a filed generation must change nothing"
    );
}

// ---------------------------------------------------------------------------
// (g) the month boundary, and reading the store back
// ---------------------------------------------------------------------------

#[test]
fn a_month_ends_where_utc_says_it_does() {
    let machine = Machine::new("usage-months");
    machine.put("h-project/session-h.jsonl", &fixture("known-totals.jsonl"));
    machine.scan();

    assert_eq!(
        store::months(&machine.state()).unwrap(),
        vec!["2026-08", "2026-09"]
    );
    let august = machine.month("2026-08");
    assert_eq!(
        august.buckets(PROVIDER).unwrap().keys().collect::<Vec<_>>(),
        vec!["2026-08-31T23"]
    );
    assert_eq!(august.since.as_deref(), Some("2026-08-31T23:00:00Z"));

    let september = machine.month("2026-09");
    assert_eq!(
        september
            .buckets(PROVIDER)
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        vec!["2026-09-01T00", "2026-09-01T01", "2026-09-01T04"]
    );
    assert_eq!(
        september.since.as_deref(),
        Some("2026-08-31T23:00:00Z"),
        "every month carries the earliest instant the whole store holds, not its own"
    );
}

#[test]
fn a_query_answers_in_hours_and_leaves_the_calendar_to_the_panel() {
    let machine = Machine::new("usage-query");
    machine.put("i-project/session-i.jsonl", &fixture("known-totals.jsonl"));
    machine.scan();

    let view = query(
        &machine.state(),
        "2026-09-01T00:00:00Z",
        "2026-09-01T02:00:00Z",
    )
    .unwrap();
    let claude = &view.providers[PROVIDER];
    assert_eq!(
        claude.keys().collect::<Vec<_>>(),
        vec!["2026-09-01T00", "2026-09-01T01"],
        "half open: the hour at the end is not in the range"
    );
    assert_eq!(claude["2026-09-01T00"]["claude-opus-5"].output, Some(700));
    assert_eq!(
        view.since.as_deref(),
        Some("2026-08-31T23:00:00Z"),
        "the panel's \"since\" line is the whole store, not the range"
    );
    assert!(view.scanned_at.is_some());

    // A range that crosses the month boundary opens both documents.
    let wide = query(
        &machine.state(),
        "2026-08-01T00:00:00Z",
        "2026-10-01T00:00:00Z",
    )
    .unwrap();
    assert_eq!(wide.providers[PROVIDER].len(), 4);
    assert!(wide.damaged.is_empty());

    // A range nobody can name draws nothing rather than failing.
    let nonsense = query(&machine.state(), "last week", "now").unwrap();
    assert!(nonsense.providers.is_empty());
    assert_eq!(nonsense.since.as_deref(), Some("2026-08-31T23:00:00Z"));
}

#[test]
fn a_machine_with_no_transcripts_is_a_state_not_a_fault() {
    let machine = Machine::new("usage-empty");
    let summary = machine.scan();
    assert_eq!(summary.files_seen, 0);
    assert_eq!(summary.credited, 0);
    assert_eq!(summary.since, None);

    let view = query(
        &machine.state(),
        "2026-09-01T00:00:00Z",
        "2026-09-02T00:00:00Z",
    )
    .unwrap();
    assert!(view.providers.is_empty());
    assert_eq!(view.since, None);
}

#[test]
fn a_replaced_transcript_is_read_from_the_top_without_losing_what_it_had_said() {
    let machine = Machine::new("usage-replaced");
    let path = machine.put("j-project/session-j.jsonl", &fixture("known-totals.jsonl"));
    let first = machine.scan();
    assert_eq!(first.credited, 6);

    // A different file at the same path: shorter, and with different messages in it.
    std::fs::write(&path, fixture("duplicates.jsonl")).unwrap();
    let second = machine.scan();
    assert_eq!(second.files_restarted, 1);
    assert_eq!(second.credited, 2);

    // What the old file had said is still in the store, because that is what the store
    // is for.
    assert_eq!(
        machine.bucket("2026-08", "2026-08-31T23", "claude-opus-5"),
        bucket(10, 100, 1000, 10_000, 1)
    );
    assert_eq!(
        machine.bucket("2026-09", "2026-09-02T12", "claude-opus-5"),
        bucket(6, 649_213, 50, 500, 2)
    );
}

#[test]
fn a_message_whose_blocks_straddle_the_cursor_is_still_one_message() {
    let machine = Machine::new("usage-straddle");
    let text = fixture("duplicates.jsonl");
    let mut lines = text.lines();
    let first_block = lines.next().unwrap().to_owned();
    let rest: Vec<&str> = lines.collect();

    let path = machine.put("k-project/session-k.jsonl", &format!("{first_block}\n"));
    let first = machine.scan();
    assert_eq!(first.credited, 1);

    std::fs::write(&path, &text).unwrap();
    let second = machine.scan();
    assert_eq!(rest.len(), 4);
    assert_eq!(
        second.duplicates, 3,
        "two more copies of the first message, and one of the second"
    );

    assert_eq!(
        machine.bucket("2026-09", "2026-09-02T12", "claude-opus-5"),
        bucket(6, 649_213, 50, 500, 2),
        "the same totals as one pass over the whole file"
    );
}

#[test]
fn rebuilding_and_rescanning_reproduces_the_same_totals() {
    let machine = Machine::new("usage-rebuild");
    machine.put("l-project/session-l.jsonl", &fixture("known-totals.jsonl"));
    machine.put(
        "l-project/session-l/subagents/agent-one.jsonl",
        &fixture("duplicates.jsonl"),
    );
    machine.scan();
    let before: Vec<_> = ["2026-08", "2026-09"]
        .iter()
        .map(|month| machine.month(month).providers[PROVIDER].buckets.clone())
        .collect();

    // The escape hatch the contract points at: the cursor and the months go together.
    assert!(store::rebuild(&machine.state()).unwrap() >= 3);
    assert!(store::months(&machine.state()).unwrap().is_empty());

    machine.scan();
    let after: Vec<_> = ["2026-08", "2026-09"]
        .iter()
        .map(|month| machine.month(month).providers[PROVIDER].buckets.clone())
        .collect();
    assert_eq!(
        before, after,
        "a rebuild recounts the transcripts to exactly the same numbers"
    );
}

// ---------------------------------------------------------------------------
// Cost
// ---------------------------------------------------------------------------

/// A full pass over a corpus the size of a real one.
///
/// Ignored by default because it writes a few hundred megabytes: run it with
/// `cargo test -p nazar-core --release -- --ignored usage_scan_cost --nocapture`.
/// The shape matches what was measured on the maintainer's machine — 118 files, 230 MB,
/// 14 441 lines carrying usage, the rest prompts and tool output — because the thing being
/// measured is the cost of *walking past* the bytes that are not accounting.
#[test]
#[ignore = "writes a few hundred megabytes"]
fn usage_scan_cost_over_a_real_sized_corpus() {
    let machine = Machine::new("usage-cost");
    let padding = "x".repeat(15_000);
    let mut bytes = 0u64;

    for file in 0..100 {
        let mut text = String::with_capacity(2_400_000);
        for line in 0..145 {
            text.push_str(&format!(
                r#"{{"parentUuid":null,"isSidechain":false,"type":"user","timestamp":"2026-09-05T12:00:00.000Z","sessionId":"session-{file}","message":{{"role":"user","content":"{padding}"}}}}
"#
            ));
            text.push_str(&format!(
                r#"{{"parentUuid":null,"isSidechain":false,"type":"assistant","timestamp":"2026-09-05T12:00:00.000Z","requestId":"req_{file}_{line}","apiBlockIndex":0,"sessionId":"session-{file}","message":{{"id":"msg_{file}_{line}","role":"assistant","model":"claude-opus-5","content":[{{"type":"text","text":"an answer"}}],"usage":{{"input_tokens":2,"output_tokens":328,"cache_creation_input_tokens":24843,"cache_read_input_tokens":35613}}}}}}
"#
            ));
        }
        bytes += text.len() as u64;
        machine.put(&format!("cost-project/session-{file}.jsonl"), &text);
    }

    let started = Instant::now();
    let summary = machine.scan();
    let took = started.elapsed();

    assert_eq!(summary.files_seen, 100);
    assert_eq!(summary.credited, 14_500);
    println!(
        "scanned {} files, {:.1} MB, {} usage lines in {:?}",
        summary.files_seen,
        bytes as f64 / 1_048_576.0,
        summary.lines,
        took
    );
    let budget = if cfg!(debug_assertions) { 60 } else { 2 };
    assert!(
        took.as_secs() < budget,
        "a full pass took {took:?}, which is over the {budget}s budget"
    );
}
