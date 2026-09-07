//! A unified diff, so that a change to somebody's settings file is shown before it is
//! made and not merely described.
//!
//! The installer edits a file it does not own. Four of the four competitors the research
//! phase looked at write that file without showing what they changed, and one of them
//! turns a file it cannot parse into `{}`. Printing the diff — always, whether or not
//! standard output is a terminal, and identically in `--dry-run` — is the cheapest way to
//! make the edit reviewable.
//!
//! The algorithm is a plain longest-common-subsequence table. Settings files are tens of
//! lines, so the quadratic cost is invisible, and a file large enough for it to matter
//! falls back to a whole-file hunk rather than spending a second on a diff nobody asked
//! for.

/// Above this many lines on either side, the diff degrades to a whole-file hunk.
const MAX_LINES: usize = 4_000;

/// Lines of unchanged context printed either side of a change.
pub const CONTEXT: usize = 3;

/// One step of the alignment between the two files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Same(usize, usize),
    Removed(usize),
    Added(usize),
}

/// A unified diff of `before` and `after`, or an empty string when they are identical.
///
/// `before_name` and `after_name` are the labels of the `---` and `+++` header lines.
#[must_use]
pub fn unified(before_name: &str, after_name: &str, before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }

    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();

    if old.len() > MAX_LINES || new.len() > MAX_LINES {
        return whole_file(before_name, after_name, &old, &new);
    }

    let steps = align(&old, &new);
    render(before_name, after_name, &old, &new, &steps)
}

/// Align the two line sequences with a longest-common-subsequence table.
fn align(old: &[&str], new: &[&str]) -> Vec<Step> {
    let (rows, columns) = (old.len() + 1, new.len() + 1);
    let mut table = vec![0u32; rows * columns];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i * columns + j] = if old[i] == new[j] {
                table[(i + 1) * columns + j + 1] + 1
            } else {
                table[(i + 1) * columns + j].max(table[i * columns + j + 1])
            };
        }
    }

    let mut steps = Vec::with_capacity(old.len() + new.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            steps.push(Step::Same(i, j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * columns + j] >= table[i * columns + j + 1] {
            steps.push(Step::Removed(i));
            i += 1;
        } else {
            steps.push(Step::Added(j));
            j += 1;
        }
    }
    while i < old.len() {
        steps.push(Step::Removed(i));
        i += 1;
    }
    while j < new.len() {
        steps.push(Step::Added(j));
        j += 1;
    }
    steps
}

/// Turn an alignment into hunks with [`CONTEXT`] lines either side.
fn render(
    before_name: &str,
    after_name: &str,
    old: &[&str],
    new: &[&str],
    steps: &[Step],
) -> String {
    // Which steps are changes, and which runs of context sit next to one.
    let mut keep = vec![false; steps.len()];
    for (index, step) in steps.iter().enumerate() {
        if matches!(step, Step::Same(..)) {
            continue;
        }
        let from = index.saturating_sub(CONTEXT);
        let to = (index + CONTEXT + 1).min(steps.len());
        for slot in keep.iter_mut().take(to).skip(from) {
            *slot = true;
        }
    }

    let mut out = format!("--- {before_name}\n+++ {after_name}\n");
    let mut index = 0usize;
    while index < steps.len() {
        if !keep[index] {
            index += 1;
            continue;
        }
        let start = index;
        while index < steps.len() && keep[index] {
            index += 1;
        }
        let hunk = &steps[start..index];

        let (mut old_start, mut new_start) = (old.len(), new.len());
        let (mut old_count, mut new_count) = (0usize, 0usize);
        for step in hunk {
            match *step {
                Step::Same(i, j) => {
                    old_start = old_start.min(i);
                    new_start = new_start.min(j);
                    old_count += 1;
                    new_count += 1;
                }
                Step::Removed(i) => {
                    old_start = old_start.min(i);
                    old_count += 1;
                }
                Step::Added(j) => {
                    new_start = new_start.min(j);
                    new_count += 1;
                }
            }
        }
        // Unified-diff line numbers are one-based, and an empty side is written as `0`.
        let old_from = if old_count == 0 { 0 } else { old_start + 1 };
        let new_from = if new_count == 0 { 0 } else { new_start + 1 };
        out.push_str(&format!(
            "@@ -{old_from},{old_count} +{new_from},{new_count} @@\n"
        ));
        for step in hunk {
            match *step {
                Step::Same(i, _) => out.push_str(&format!(" {}\n", old[i])),
                Step::Removed(i) => out.push_str(&format!("-{}\n", old[i])),
                Step::Added(j) => out.push_str(&format!("+{}\n", new[j])),
            }
        }
    }
    out
}

/// The fallback for a file too large to align line by line.
fn whole_file(before_name: &str, after_name: &str, old: &[&str], new: &[&str]) -> String {
    format!(
        "--- {before_name}\n+++ {after_name}\n@@ -1,{} +1,{} @@\n\
         (file too large to diff line by line; {} lines replaced by {})\n",
        old.len(),
        new.len(),
        old.len(),
        new.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_files_produce_no_diff() {
        assert_eq!(unified("a", "b", "one\ntwo\n", "one\ntwo\n"), "");
    }

    #[test]
    fn a_one_line_change_is_shown_with_context() {
        let before = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3\n}\n";
        let after = "{\n  \"a\": 1,\n  \"b\": 9,\n  \"c\": 3\n}\n";
        let diff = unified("settings.json", "settings.json", before, after);

        assert!(diff.starts_with("--- settings.json\n+++ settings.json\n@@ "));
        assert!(diff.contains("-  \"b\": 2,\n"), "{diff}");
        assert!(diff.contains("+  \"b\": 9,\n"), "{diff}");
        assert!(diff.contains(" {\n"), "context is missing from {diff}");
        assert!(
            !diff.contains("-  \"a\""),
            "context must not be marked as removed"
        );
    }

    #[test]
    fn an_addition_to_a_file_that_had_nothing_there() {
        let before = "{\n  \"model\": \"x\"\n}\n";
        let after = "{\n  \"model\": \"x\",\n  \"statusLine\": {}\n}\n";
        let diff = unified("before", "after", before, after);
        assert!(diff.contains("+  \"statusLine\": {}\n"), "{diff}");
    }

    #[test]
    fn every_added_and_removed_line_appears_exactly_once() {
        let before = (0..40).map(|n| format!("line {n}\n")).collect::<String>();
        let after = before.replace("line 7\n", "line seven\n");
        let diff = unified("a", "b", &before, &after);

        assert_eq!(diff.matches("-line 7\n").count(), 1, "{diff}");
        assert_eq!(diff.matches("+line seven\n").count(), 1, "{diff}");
        // Only the neighbourhood of the change is printed, not all forty lines.
        assert!(diff.lines().count() < 14, "{diff}");
    }

    #[test]
    fn two_distant_changes_become_two_hunks() {
        let before = (0..40).map(|n| format!("line {n}\n")).collect::<String>();
        let after = before
            .replace("line 2\n", "two\n")
            .replace("line 35\n", "thirty five\n");
        let diff = unified("a", "b", &before, &after);
        assert_eq!(diff.matches("@@ ").count(), 2, "{diff}");
    }

    #[test]
    fn an_empty_file_on_either_side_is_handled() {
        let diff = unified("a", "b", "", "one\n");
        assert!(diff.contains("+one\n"), "{diff}");
        let diff = unified("a", "b", "one\n", "");
        assert!(diff.contains("-one\n"), "{diff}");
    }

    #[test]
    fn a_huge_file_degrades_instead_of_hanging() {
        let before = (0..MAX_LINES + 10)
            .map(|n| format!("line {n}\n"))
            .collect::<String>();
        let after = format!("{before}extra\n");
        let diff = unified("a", "b", &before, &after);
        assert!(diff.contains("too large to diff"), "{diff}");
    }
}
