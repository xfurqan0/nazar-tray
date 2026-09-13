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
    Bucket, PROVIDER, PROVIDER_CODEX, UNKNOWN_MODEL, UsageSummary, codex, projects_dir, query,
    scan, scan_claude, scan_codex,
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

    /// Put text at `relative` under `.codex/sessions/`, the tree the rollout reader walks.
    fn put_rollout(&self, relative: &str, text: &str) -> PathBuf {
        let path = codex::sessions_dir(&codex::codex_dir(&self.home())).join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Put text at `relative` under `.codex/archived_sessions/`, the tree it does not.
    fn put_archived(&self, relative: &str, text: &str) -> PathBuf {
        let path = codex::codex_dir(&self.home())
            .join("archived_sessions")
            .join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn scan(&self) -> UsageSummary {
        scan_claude(&self.home(), &self.state()).unwrap()
    }

    fn scan_codex(&self) -> UsageSummary {
        scan_codex(&self.home(), &self.state()).unwrap()
    }

    fn month(&self, month: &str) -> Month {
        match store::read_month(&self.state(), month).unwrap() {
            Read::Document(document) => *document,
            other => panic!("no document for {month}: {other:?}"),
        }
    }

    fn bucket(&self, month: &str, hour: &str, model: &str) -> Bucket {
        self.bucket_of(PROVIDER, month, hour, model)
    }

    fn codex_bucket(&self, month: &str, hour: &str, model: &str) -> Bucket {
        self.bucket_of(PROVIDER_CODEX, month, hour, model)
    }

    fn bucket_of(&self, provider: &str, month: &str, hour: &str, model: &str) -> Bucket {
        self.month(month)
            .buckets(provider)
            .and_then(|hours| hours.get(hour))
            .and_then(|models| models.get(model))
            .cloned()
            .unwrap_or_else(|| panic!("no {provider} bucket for {hour} / {model}"))
    }

    /// Every month document this machine has written, byte for byte.
    ///
    /// The cursor documents are deliberately left out: a pass that read the same records
    /// again after a truncation *does* move an offset, and the claim being checked is about
    /// the totals rather than about the bookkeeping that produced them.
    fn month_documents(&self) -> Vec<(String, String)> {
        self.documents()
            .into_iter()
            .filter(|(name, _)| name.starts_with("2026-"))
            .collect()
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
        input,
        output,
        cache_create,
        cache_read,
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
        summary.skipped_api_errors, 1,
        "an error the server answered with carries counters and is not billed usage"
    );
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
    assert_eq!(counted.input, 7);
    assert_eq!(counted.output, 70);
    assert_eq!(
        counted.cache_read, 0,
        "the 700 lives only inside iterations[], which is never read"
    );
    assert_eq!(counted.requests, 1);

    // The line whose source named no model keeps its tokens under the one id this
    // reader writes itself, rather than losing them to a missing field.
    let unnamed = machine.bucket("2026-09", "2026-09-03T08", UNKNOWN_MODEL);
    assert_eq!(unnamed.input, 321);
    assert_eq!(unnamed.output, 654);

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

    // The three public types whose `Debug` a diagnostic would reach for. `message.id` and
    // `requestId` are a dedupe key, they are never written to disk, and they must not
    // arrive in a log line either — so the `Debug` of each is written by hand.
    let machine = Machine::new("usage-sentinel");
    let path = machine.put("e-project/session-e.jsonl", &text);
    let scanned = scan::scan_file(&path, None).unwrap();
    for printed in [
        format!("{scanned:?}"),
        format!("{:?}", scanned.records),
        format!("{:?}", scan::parse_line(text.trim_end())),
    ] {
        assert!(
            !printed.contains("msg_sentinel"),
            "a message id reached a Debug: {printed}"
        );
        assert!(
            !printed.contains(CONTENT) && !printed.contains(FIELD),
            "{printed}"
        );
    }

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
    let mut cursors = store::read_cursors(&machine.state(), PROVIDER).unwrap();
    cursors.pending = vec![store::Pending {
        month: "2026-09".to_owned(),
        generation: totals.applied_through,
        scanned_at: september.scanned_at.clone().unwrap(),
        hours: totals.buckets.clone(),
    }];
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
    assert_eq!(claude["2026-09-01T00"]["claude-opus-5"].output, 700);
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
fn a_truncated_transcript_is_read_again_and_counted_once() {
    let machine = Machine::new("usage-truncated");
    let text = fixture("known-totals.jsonl");
    let lines: Vec<&str> = text.lines().collect();
    let head: String = lines[..3]
        .iter()
        .map(|line| format!("{line}\n"))
        .collect::<String>();

    let path = machine.put("n-project/session-n.jsonl", &text);
    let first = machine.scan();
    assert_eq!(first.credited, 6);
    let before = machine.month_documents();

    // Claude Code prunes a transcript in place. The offset is now past the end, so the
    // next pass reads the file from the top — and every record it finds there is one the
    // months already hold.
    std::fs::write(&path, &head).unwrap();
    let second = machine.scan();
    assert_eq!(second.files_restarted, 1, "the pass has to notice");
    assert_eq!(
        second.credited, 0,
        "and must credit nothing it has credited before"
    );
    assert_eq!(second.credited_total, 0);

    assert_eq!(
        machine.month_documents(),
        before,
        "a truncation must leave the totals byte for byte as they were"
    );

    // And the records the truncation removed are still there, counted once each.
    assert_eq!(
        machine.bucket("2026-09", "2026-09-01T04", "claude-opus-5"),
        bucket(60, 600, 6000, 60_000, 1)
    );
}

#[test]
fn a_month_that_could_not_be_written_keeps_its_totals_until_it_can() {
    let machine = Machine::new("usage-journal");
    let text = fixture("known-totals.jsonl");
    let lines: Vec<&str> = text.lines().collect();
    let path = machine.put(
        "o-project/session-o.jsonl",
        &lines[..6]
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    );
    machine.scan();
    let september =
        std::fs::read_to_string(store::month_path(&machine.state(), "2026-09")).unwrap();

    // September stops parsing, and a record for September arrives.
    let broken = "{ this was a month once";
    std::fs::write(store::month_path(&machine.state(), "2026-09"), broken).unwrap();
    std::fs::write(&path, &text).unwrap();

    let damaged = machine.scan();
    assert_eq!(damaged.credited, 1, "the last line was read");
    assert_eq!(damaged.damaged, vec!["2026-09"], "and could not be filed");
    assert_eq!(
        std::fs::read_to_string(store::month_path(&machine.state(), "2026-09")).unwrap(),
        broken,
        "never replaced by an empty one and never repaired"
    );
    // The bytes it came from are behind a cursor that has moved, so the totals have to be
    // somewhere: they are in the journal.
    let cursors = store::read_cursors(&machine.state(), PROVIDER).unwrap();
    assert_eq!(cursors.pending.len(), 1);
    assert_eq!(cursors.pending[0].month, "2026-09");

    // The month is repaired — here by putting back what was there, which is the best case
    // and the one that could double count.
    std::fs::write(store::month_path(&machine.state(), "2026-09"), &september).unwrap();

    let repaired = machine.scan();
    assert!(repaired.damaged.is_empty());
    assert!(
        store::read_cursors(&machine.state(), PROVIDER)
            .unwrap()
            .pending
            .is_empty(),
        "the journal is emptied only by filing"
    );
    // The record that waited is there, and the records that were already filed did not
    // arrive a second time.
    assert_eq!(
        machine.bucket("2026-09", "2026-09-01T04", "claude-opus-5"),
        bucket(60, 600, 6000, 60_000, 1),
        "the record the journal held"
    );
    assert_eq!(
        machine.bucket("2026-09", "2026-09-01T00", "claude-opus-5"),
        bucket(70, 700, 7000, 70_000, 2),
        "and the ones that were filed before the damage, counted once"
    );

    let again = machine.scan();
    assert_eq!(again.credited, 0);
    assert_eq!(
        machine.bucket("2026-09", "2026-09-01T04", "claude-opus-5"),
        bucket(60, 600, 6000, 60_000, 1)
    );
}

#[test]
fn a_truncated_rollout_is_read_again_and_counted_once() {
    let machine = Machine::new("usage-codex-truncated");
    let text = fixture("rollout-known-totals.jsonl");
    let lines: Vec<&str> = text.lines().collect();
    let path = machine.put_rollout(&rollout("12", "2026-09-12T09-59-58-session-a"), &text);

    let first = machine.scan_codex();
    assert_eq!(first.credited, 4);
    let before = machine.month_documents();

    // A rollout event has no id, so the cursor is the whole dedupe — until the cursor has
    // to go back to zero, which is what the carried fingerprints are for.
    std::fs::write(
        &path,
        lines[..4]
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let second = machine.scan_codex();
    assert_eq!(second.files_restarted, 1);
    assert_eq!(second.credited, 0, "every event was counted before");
    assert_eq!(
        machine.month_documents(),
        before,
        "and the totals are byte for byte what they were"
    );
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T11", "gpt-6-astra"),
        bucket(1000, 300, 0, 3000, 1),
        "including the events the truncation removed"
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
// (h) the Codex reader
// ---------------------------------------------------------------------------

/// A rollout the way Codex lays one out: `sessions/YYYY/MM/DD/rollout-<ISO>-<id>.jsonl`.
fn rollout(day: &str, name: &str) -> String {
    format!("2026/09/{day}/rollout-{name}.jsonl")
}

#[test]
fn a_rollout_with_known_totals_adds_up_to_them() {
    let machine = Machine::new("usage-codex-known");
    machine.put_rollout(
        &rollout("12", "2026-09-12T09-59-58-session-a"),
        &fixture("rollout-known-totals.jsonl"),
    );

    let summary = machine.scan_codex();
    assert_eq!(summary.files_seen, 1);
    assert_eq!(summary.credited, 4, "four token_count events");
    assert_eq!(summary.malformed, 0);
    assert_eq!(summary.credited_total, 10_650);

    // The event before the first turn_context belongs to a model nobody here knows.
    assert_eq!(summary.unnamed_model, 1);
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T10", UNKNOWN_MODEL),
        // 1000 input of which 600 were cached, so 400 of it was new.
        bucket(400, 50, 0, 600, 1)
    );

    // Two events under the model the session named, in one hour.
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T10", "gpt-5.6-sol"),
        bucket(1000, 300, 0, 4000, 2)
    );

    // And the model it switched to, in the next hour.
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T11", "gpt-6-astra"),
        bucket(1000, 300, 0, 3000, 1)
    );

    // The line that carries a `token_count` key inside somebody else's payload matched
    // the needle, was parsed, and was not an event.
    assert_eq!(summary.lines, 7, "four events, two contexts, one impostor");
}

#[test]
fn the_cumulative_counter_resets_mid_session_and_the_per_turn_sum_does_not() {
    let machine = Machine::new("usage-codex-reset");
    machine.put_rollout(
        &rollout("12", "2026-09-12T14-00-00-session-b"),
        &fixture("rollout-reset.jsonl"),
    );

    let summary = machine.scan_codex();
    assert_eq!(summary.credited, 4);

    // `total_token_usage` in that fixture goes 1100 → 3300 → **550** → 1430: the context
    // was compacted and the cumulative counter started again. A reader that took the last
    // one would report 1430 for a session that spent 4730.
    let counted = machine.codex_bucket("2026-09", "2026-09-12T14", "gpt-5.6-sol");
    assert_eq!(counted, bucket(2900, 430, 0, 1400, 4));
    let total = counted.input + counted.output + counted.cache_create + counted.cache_read;
    assert_eq!(total, 4730, "the sum of every turn");
    assert_ne!(total, 1430, "not the cumulative counter's last word");
}

#[test]
fn the_model_in_force_survives_the_gap_between_two_passes() {
    let machine = Machine::new("usage-codex-carry");
    let text = fixture("rollout-reset.jsonl");
    let lines: Vec<&str> = text.lines().collect();
    let path = machine.put_rollout(
        &rollout("12", "2026-09-12T14-00-00-session-b"),
        &format!("{}\n{}\n{}\n", lines[0], lines[1], lines[2]),
    );

    let first = machine.scan_codex();
    assert_eq!(first.credited, 1);
    assert_eq!(first.unnamed_model, 0);

    // The `turn_context` that named the model is behind the cursor now; the events that
    // arrive next still belong to it.
    std::fs::write(&path, &text).unwrap();
    let second = machine.scan_codex();
    assert_eq!(second.credited, 3);
    assert_eq!(second.unnamed_model, 0, "not one of them is unknown");
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T14", "gpt-5.6-sol"),
        bucket(2900, 430, 0, 1400, 4)
    );
}

#[test]
fn a_fork_that_copied_an_opening_run_does_not_count_it_twice() {
    let machine = Machine::new("usage-codex-fork");
    machine.put_rollout(
        &rollout("12", "2026-09-12T09-59-58-session-a"),
        &fixture("rollout-known-totals.jsonl"),
    );
    machine.put_rollout(
        &rollout("12", "2026-09-12T12-00-00-session-a-fork"),
        &fixture("rollout-fork.jsonl"),
    );

    let summary = machine.scan_codex();
    assert_eq!(summary.files_seen, 2);
    assert_eq!(
        summary.duplicates, 2,
        "the two events the fork copied from its parent"
    );
    assert_eq!(summary.credited, 5, "four of the parent's and one new one");

    // The hour both files claim is exactly the parent's.
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T10", UNKNOWN_MODEL),
        bucket(400, 50, 0, 600, 1)
    );
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T10", "gpt-5.6-sol"),
        bucket(1000, 300, 0, 4000, 2)
    );

    // And what the fork actually spent is counted, under the model it was using.
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T12", "gpt-5.6-sol"),
        bucket(1000, 400, 0, 4000, 1)
    );

    // A second pass over both adds nothing, and does not decide the fork is a fork twice.
    let again = machine.scan_codex();
    assert_eq!(again.credited, 0);
    assert_eq!(again.duplicates, 0);
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T10", "gpt-5.6-sol"),
        bucket(1000, 300, 0, 4000, 2)
    );
}

#[test]
fn an_archived_session_is_not_read() {
    let machine = Machine::new("usage-codex-archived");
    machine.put_archived(
        &rollout("12", "2026-09-12T09-59-58-session-a"),
        &fixture("rollout-known-totals.jsonl"),
    );

    let summary = machine.scan_codex();
    assert_eq!(summary.files_seen, 0, "the walk starts at sessions/");
    assert_eq!(summary.credited, 0);
    assert!(
        store::months(&machine.state()).unwrap().is_empty(),
        "and nothing was written at all"
    );
}

#[test]
fn a_machine_with_no_rollout_logs_is_a_state_not_a_fault() {
    let machine = Machine::new("usage-codex-empty");
    let summary = machine.scan_codex();
    assert_eq!(summary.files_seen, 0);
    assert_eq!(summary.credited, 0);
    assert_eq!(summary.since, None);
    assert!(
        store::months(&machine.state()).unwrap().is_empty(),
        "no month document, on a machine that spent nothing"
    );
}

#[test]
fn scanning_the_same_rollouts_again_adds_nothing() {
    let machine = Machine::new("usage-codex-idempotent");
    machine.put_rollout(
        &rollout("12", "2026-09-12T09-59-58-session-a"),
        &fixture("rollout-known-totals.jsonl"),
    );

    let first = machine.scan_codex();
    assert_eq!(first.credited, 4);
    let after_first = machine.documents();

    let second = machine.scan_codex();
    assert_eq!(second.files_seen, 1);
    assert_eq!(second.bytes_read, 0, "every byte was behind the cursor");
    assert_eq!(second.credited, 0);
    assert_eq!(
        machine.documents(),
        after_first,
        "and the store is byte for byte what it was"
    );
}

#[test]
fn the_two_readers_share_a_month_and_never_a_cursor() {
    let machine = Machine::new("usage-both-providers");
    machine.put("m-project/session-m.jsonl", &fixture("known-totals.jsonl"));
    machine.put_rollout(
        &rollout("12", "2026-09-12T09-59-58-session-a"),
        &fixture("rollout-known-totals.jsonl"),
    );

    let claude = machine.scan();
    assert_eq!(claude.credited, 6);
    let codex = machine.scan_codex();
    assert_eq!(codex.credited, 4);

    // One month document, two provider blocks, each with its own generation stamp.
    let month = machine.month("2026-09");
    assert!(month.buckets(PROVIDER).is_some());
    assert!(month.buckets(PROVIDER_CODEX).is_some());
    assert_eq!(month.providers[PROVIDER].applied_through, 1);
    assert_eq!(month.providers[PROVIDER_CODEX].applied_through, 1);

    // Two cursor documents, and a pass by one reader does not touch the other's.
    let codex_cursors = store::read_cursors(&machine.state(), PROVIDER_CODEX).unwrap();
    assert_eq!(codex_cursors.files.len(), 1);
    let claude_again = machine.scan();
    assert_eq!(claude_again.credited, 0);
    assert_eq!(
        store::read_cursors(&machine.state(), PROVIDER_CODEX).unwrap(),
        codex_cursors,
        "the transcript scan left the rollout offsets exactly as they were"
    );

    // And both readers' totals are still there, side by side.
    assert_eq!(
        machine.bucket("2026-09", "2026-09-01T04", "claude-opus-5"),
        bucket(60, 600, 6000, 60_000, 1)
    );
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-12T11", "gpt-6-astra"),
        bucket(1000, 300, 0, 3000, 1)
    );
}

#[test]
fn nothing_but_the_allow_listed_values_leaves_the_rollout_reader() {
    const CONTENT: &str = "SENTINEL-content-do-not-leak-7c41";
    const FIELD: &str = "SENTINEL field do not leak 9b2e";

    let text = fixture("rollout-sentinel.jsonl");
    assert!(
        text.contains(CONTENT) && text.contains(FIELD),
        "the fixture lost its sentinels"
    );

    // The event line itself: a payload whose every neighbouring string is a sentinel.
    let event_line = text.trim_end().lines().next_back().unwrap();
    let outcome = codex::parse_line(event_line);
    let parsed = format!("{outcome:?}");
    assert!(
        !parsed.contains(CONTENT) && !parsed.contains(FIELD),
        "the parsed event carries a sentinel"
    );
    match outcome {
        codex::Outcome::Usage(event) => {
            assert_eq!(event.hour, "2026-09-04T09");
            assert_eq!(event.usage.output, Some(33));
        }
        other => panic!("expected an event, got {other:?}"),
    }

    // The `turn_context` whose model is prose: refused on its shape, and the model the
    // session actually named stays in force for the event after it.
    let prose_context = text
        .lines()
        .find(|line| line.contains(&format!("\"model\":\"{FIELD}\"")))
        .unwrap();
    assert_eq!(codex::parse_line(prose_context), codex::Outcome::Other);

    let machine = Machine::new("usage-codex-sentinel");
    machine.put_rollout(&rollout("04", "2026-09-04T09-00-00-session-s"), &text);
    let summary = machine.scan_codex();

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

    // And the numbers still arrived, under the model the session named.
    assert_eq!(
        machine.codex_bucket("2026-09", "2026-09-04T09", "gpt-5.6-sol"),
        bucket(3, 33, 0, 3333, 1)
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
// Cost
// ---------------------------------------------------------------------------

/// The numbers this module's documentation quotes, measured again.
///
/// Ignored by default: it reads the transcripts this machine actually has, which is a
/// corpus rather than a fixture, and a machine without `~/.claude` has nothing to measure —
/// that is CI, and it is a pass rather than a failure. The store it writes is a throwaway
/// directory, so the maintainer's own history is neither read nor added to.
///
/// `cargo test -p nazar-core --release -- --ignored usage_scan_this_machine --nocapture`
#[test]
#[ignore = "reads the machine's own transcripts"]
fn usage_scan_this_machine_and_print_what_the_docs_quote() {
    let Some(home) = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
    else {
        println!("no home directory in the environment; nothing to measure");
        return;
    };
    let projects = projects_dir(&home);
    if !projects.is_dir() {
        println!("no transcripts on this machine; nothing to measure");
        return;
    }

    let dir = TempDir::new("usage-measure");
    let started = Instant::now();
    let summary = super::scan_claude_in(&projects, &dir.path).unwrap();
    let took = started.elapsed();

    let inflation = if summary.credited_total == 0 {
        0.0
    } else {
        summary.naive_total as f64 / summary.credited_total as f64
    };
    let cursors = std::fs::metadata(store::cursors_path(&dir.path, PROVIDER))
        .map(|meta| meta.len())
        .unwrap_or(0);
    println!(
        "files {} · lines {} · unique {} · duplicates {} · api errors {} · malformed {} \
         · credited {} · naive {} · inflation {inflation:.3}x · {:?} · cursors {:.1} KB",
        summary.files_seen,
        summary.lines,
        summary.credited,
        summary.duplicates,
        summary.skipped_api_errors,
        summary.malformed,
        summary.credited_total,
        summary.naive_total,
        took,
        cursors as f64 / 1024.0
    );

    // A second pass reads only what arrived between the two — which on this machine is
    // whatever the session running the test has written since, and is usually nothing.
    let before = std::fs::read_to_string(store::cursors_path(&dir.path, PROVIDER)).unwrap();
    let again = super::scan_claude_in(&projects, &dir.path).unwrap();
    println!(
        "second pass: {} bytes, {} new messages, {} duplicates",
        again.bytes_read, again.credited, again.duplicates
    );
    if again.bytes_read == 0 {
        assert_eq!(again.credited, 0, "no new bytes, so nothing new to credit");
        assert_eq!(
            std::fs::read_to_string(store::cursors_path(&dir.path, PROVIDER)).unwrap(),
            before,
            "and the cursor document is byte for byte what it was"
        );
    }
}

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
