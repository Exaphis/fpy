use chrono::{DateTime, Local, Utc};
use nucleo::{
    Config as NucleoConfig, Matcher as NucleoMatcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};

use super::{
    duration_format::format_duration_ns, syntax::highlighted_python_lines,
    transcript::display_width,
};
use crate::history::{HistoryEntry, HistoryOutcome};

const HISTORY_SEARCH_RESULT_LIMIT: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HistorySearchEntry {
    pub(super) code: String,
    pub(super) search_text: String,
    first_line: String,
    pub(super) line_count: usize,
    pub(super) duration_ns: Option<u64>,
    timestamp_unix_ns: Option<u64>,
    pub(super) outcome: Option<HistoryOutcome>,
}

impl HistorySearchEntry {
    pub(super) fn new(code: String) -> Self {
        let first_line = code.lines().next().unwrap_or_default().to_string();
        let line_count = code.lines().count().max(1);
        Self {
            search_text: normalize_history_search_text(&code),
            code,
            first_line,
            line_count,
            duration_ns: None,
            timestamp_unix_ns: None,
            outcome: None,
        }
    }

    pub(super) fn from_history_entry(entry: HistoryEntry) -> Self {
        let mut search = Self::new(entry.code);
        search.duration_ns = entry.duration_ns;
        search.timestamp_unix_ns = Some(entry.ts_unix_ns);
        search.outcome = entry.outcome;
        search
    }

    pub(super) fn highlighted_summary(&self, width: usize) -> String {
        let plain_left = self.plain_summary_left();
        let highlighted_left = syntax_highlighted_history_preview(&self.first_line)
            .into_iter()
            .next()
            .map(|mut line| {
                if line.contains("\u{1b}[")
                    && !line.ends_with("\u{1b}[39m")
                    && !line.ends_with("\u{1b}[0m")
                {
                    line.push_str("\u{1b}[39m");
                }
                if self.line_count > 1 {
                    format!("{line} …")
                } else {
                    line
                }
            })
            .unwrap_or_else(|| plain_left.clone());
        self.summary_with_left(width, &highlighted_left)
    }

    fn plain_summary_left(&self) -> String {
        if self.line_count > 1 {
            format!("{} …", self.first_line)
        } else {
            self.first_line.clone()
        }
    }

    fn summary_with_left(&self, width: usize, left: &str) -> String {
        let right = self.metadata();
        if width == 0 {
            return String::new();
        }
        if right.is_empty() {
            return if display_width(left) <= width {
                left.to_string()
            } else {
                truncate_chars(&self.plain_summary_left(), width)
            };
        }
        let right_width = right.chars().count();
        if right_width >= width {
            return truncate_chars(&right, width);
        }
        let left_width = width.saturating_sub(right_width + 1);
        let rendered_left = if display_width(left) <= left_width {
            left.to_string()
        } else {
            truncate_chars(&self.plain_summary_left(), left_width)
        };
        let padding = left_width.saturating_sub(display_width(&rendered_left));
        format!("{rendered_left}{} {right}", " ".repeat(padding))
    }

    fn metadata(&self) -> String {
        let mut parts = Vec::new();
        if let Some(duration_ns) = self.duration_ns {
            parts.push(format_duration_ns(duration_ns));
        }
        if let Some(timestamp) = self.timestamp_unix_ns.and_then(format_history_timestamp) {
            parts.push(timestamp);
        }
        if let Some(outcome) = self.outcome {
            match outcome {
                HistoryOutcome::Ok => {}
                HistoryOutcome::Error => parts.push("error".to_string()),
                HistoryOutcome::Interrupted => parts.push("interrupted".to_string()),
            }
        }
        parts.join(" ")
    }
}

pub(super) struct HistorySearchState {
    pub(super) open: bool,
    pub(super) query: String,
    matcher: NucleoMatcher,
    pub(super) results: Vec<usize>,
    pub(super) selected: usize,
    pub(super) scroll: usize,
}

impl HistorySearchState {
    pub(super) fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            matcher: NucleoMatcher::new(NucleoConfig::DEFAULT),
            results: Vec::new(),
            selected: 0,
            scroll: 0,
        }
    }

    pub(super) fn refresh_results(&mut self, entries: &[HistorySearchEntry]) {
        let query = self.query.trim();
        if query.is_empty() {
            self.results = (0..entries.len()).rev().collect();
            self.selected = 0;
            self.scroll = 0;
            return;
        }

        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut matches = entries
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, entry)| {
                pattern
                    .score(
                        Utf32Str::new(&entry.search_text, &mut buf),
                        &mut self.matcher,
                    )
                    .map(|score| (index, score))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0)));
        self.results = matches.into_iter().map(|(index, _)| index).collect();
        self.selected = 0;
        self.scroll = 0;
    }
}

pub(super) fn history_search_layout_for_popup(
    popup_height: u16,
    result_count: usize,
    preview_rows: usize,
) -> (u16, u16) {
    let available = popup_height.saturating_sub(4).max(2);
    let desired_results = result_count.clamp(1, HISTORY_SEARCH_RESULT_LIMIT) as u16;
    let desired_preview = preview_rows.max(1) as u16;

    if desired_results.saturating_add(desired_preview) <= available {
        return (desired_results, desired_preview);
    }

    let results_height = desired_results.min(available.saturating_sub(desired_preview).max(1));
    let preview_height = available.saturating_sub(results_height).max(1);
    (results_height, preview_height)
}

pub(super) fn history_search_scroll_for_selection(
    result_count: usize,
    visible_rows: usize,
    selected: usize,
) -> usize {
    if visible_rows == 0 {
        return 0;
    }

    let max_scroll = result_count.saturating_sub(visible_rows);
    let preferred_row = visible_rows.saturating_sub(1).min(4);
    selected.saturating_sub(preferred_row).min(max_scroll)
}

fn normalize_history_search_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_history_timestamp(timestamp_unix_ns: u64) -> Option<String> {
    let seconds = i64::try_from(timestamp_unix_ns / 1_000_000_000).ok()?;
    let nanos = u32::try_from(timestamp_unix_ns % 1_000_000_000).ok()?;
    let utc = DateTime::<Utc>::from_timestamp(seconds, nanos)?;
    Some(utc.with_timezone(&Local).format("%Y%m%d %H:%M").to_string())
}

pub(super) fn syntax_highlighted_history_preview(code: &str) -> Vec<String> {
    highlighted_python_lines(code, true).unwrap_or_else(|| plain_history_preview(code))
}

fn plain_history_preview(code: &str) -> Vec<String> {
    let mut lines = code
        .split('\n')
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn truncate_chars(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        HistorySearchEntry, HistorySearchState, format_history_timestamp,
        history_search_layout_for_popup, history_search_scroll_for_selection,
        syntax_highlighted_history_preview,
    };
    use crate::{history::HistoryOutcome, ui::transcript::strip_ansi};

    #[test]
    fn formats_history_timestamp_as_local_yyyymmdd_24h_time() {
        assert_eq!(
            format_history_timestamp(1_700_000_000_000_000_000),
            Some(
                chrono::DateTime::<chrono::Utc>::from_timestamp(1_700_000_000, 0)
                    .unwrap()
                    .with_timezone(&chrono::Local)
                    .format("%Y%m%d %H:%M")
                    .to_string()
            )
        );
    }

    #[test]
    fn history_search_metadata_includes_runtime_then_timestamp() {
        let mut entry = HistorySearchEntry::new("x = 1".to_string());
        entry.duration_ns = Some(1_500_000);
        entry.timestamp_unix_ns = Some(1_700_000_000_000_000_000);

        let metadata = entry.metadata();
        assert!(metadata.starts_with("1.50ms "));
        assert!(metadata.contains(&format_history_timestamp(1_700_000_000_000_000_000).unwrap()));
    }

    #[test]
    fn history_search_metadata_includes_non_ok_outcome() {
        let mut entry = HistorySearchEntry::new("x = 1".to_string());
        entry.outcome = Some(HistoryOutcome::Interrupted);

        assert_eq!(entry.metadata(), "interrupted");
    }

    #[test]
    fn history_search_refreshes_empty_query_in_reverse_order() {
        let entries = vec![
            HistorySearchEntry::new("a".to_string()),
            HistorySearchEntry::new("b".to_string()),
        ];
        let mut state = HistorySearchState::new();

        state.refresh_results(&entries);

        assert_eq!(state.results, vec![1, 0]);
        assert_eq!(state.selected, 0);
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn history_search_layout_expands_preview_when_space_allows() {
        assert_eq!(history_search_layout_for_popup(39, 1, 7), (1, 7));
    }

    #[test]
    fn history_search_scroll_recenters_before_selection_hits_bottom() {
        assert_eq!(history_search_scroll_for_selection(20, 10, 5), 1);
    }

    #[test]
    fn history_search_scroll_keeps_selected_result_visible_near_bottom() {
        assert_eq!(history_search_scroll_for_selection(12, 10, 8), 2);
    }

    #[test]
    fn history_search_preview_uses_python_syntax_highlighting() {
        let lines = syntax_highlighted_history_preview("x = 1\nprint(x)");

        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines
                .iter()
                .map(|line| strip_ansi(line))
                .collect::<Vec<_>>()
                .join(""),
            "x = 1print(x)"
        );
        assert!(lines.iter().any(|line| line.contains("\u{1b}[")));
    }

    #[test]
    fn history_search_summary_uses_highlighting_without_wrapping_long_entries() {
        let entry = HistorySearchEntry::new("very_long_variable_name = 123".to_string());
        let summary = entry.highlighted_summary(10);

        assert_eq!(strip_ansi(&summary).chars().count(), 10);
        assert_eq!(strip_ansi(&summary), "very_long_");
    }

    #[test]
    fn history_search_highlighted_summary_resets_before_metadata_padding() {
        let mut entry = HistorySearchEntry::new("import os".to_string());
        entry.duration_ns = Some(5_000_000);
        let summary = entry.highlighted_summary(30);

        assert_eq!(strip_ansi(&summary), "import os               5.00ms");
        let reset_index = summary.find("\u{1b}[39m").expect("foreground reset");
        let metadata_index = summary.find("5.00ms").expect("metadata");
        assert!(
            reset_index < metadata_index,
            "metadata should not inherit syntax color: {summary:?}"
        );
    }

    #[test]
    fn history_search_preview_resets_line_comments() {
        let lines = syntax_highlighted_history_preview("# comment\nx = 1");

        assert_ne!(ansi_prefix(&lines[0]), ansi_prefix(&lines[1]));
    }

    fn ansi_prefix(line: &str) -> Option<&str> {
        let end = line.find('m')?;
        Some(&line[..=end])
    }
}
