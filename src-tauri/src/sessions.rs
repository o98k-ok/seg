use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::PathBuf;

/// Expand a leading `~` to `$HOME` so users can type `~/work/.codex` in the UI.
pub fn expand_tilde(input: &str) -> PathBuf {
    let s = input.trim();
    if let Some(rest) = s.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        return PathBuf::from(home).join(rest);
    }
    if s == "~" {
        return PathBuf::from(std::env::var("HOME").unwrap_or_default());
    }
    PathBuf::from(s)
}

pub const RUNNING_THRESHOLD_SECS: i64 = 60;
pub const FINISHED_THRESHOLD_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Finished,
    Stopped,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayKind {
    Current,
    Recent,
    Longest,
}

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub duration_secs: i64,
    pub mode: Option<String>,
    pub turn_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Goal {
    pub objective: String,
    pub status: Option<String>,
    pub time_used_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub source: Source,
    pub id: String,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cli_version: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub current_mode: Option<String>,
    pub has_plan_mode: bool,
    pub goal: Option<Goal>,
    pub started_at: DateTime<Utc>,
    pub last_event_at: DateTime<Utc>,
    pub total_turns: usize,
    pub total_duration_secs: i64,
    pub segment_count: usize,
    pub segments: Vec<Segment>,
    pub status: Status,
    pub display_segment: Segment,
    pub display_segment_kind: DisplayKind,
    pub file_path: String,
    pub file_size_bytes: u64,
}

pub fn status_from_age_secs(age_secs: i64) -> Status {
    if age_secs <= RUNNING_THRESHOLD_SECS {
        Status::Running
    } else if age_secs <= FINISHED_THRESHOLD_SECS {
        Status::Finished
    } else {
        Status::Stopped
    }
}

/// Build segments anchored to user messages. Each user message starts a new
/// segment; the segment runs from that user-message timestamp until just before
/// the next user message, or until the last event in the session for the final
/// segment.
///
/// `activity_timestamps` must hold only *conversational* events (real user/
/// assistant/tool activity), NOT bookkeeping rows the CLI writes at prompt-submit
/// time (Claude `attachment`/`file-history-snapshot`/`permission-mode`, Codex
/// `event_msg` like `task_started`). Those carry the next turn's timestamp but sit
/// ~1ms before the user message, so if they were included a segment's end would
/// snap to resume time and absorb the entire idle gap (e.g. an overnight pause
/// showing up as a 16h "turn"). Both slices must be sorted ascending.
pub fn build_segments(
    user_timestamps: &[DateTime<Utc>],
    activity_timestamps: &[DateTime<Utc>],
    mode_events: &[(DateTime<Utc>, String)],
) -> Vec<Segment> {
    if user_timestamps.is_empty() {
        // No user message; fall back to a single segment spanning the whole file.
        if activity_timestamps.is_empty() {
            return Vec::new();
        }
        let start = *activity_timestamps.first().unwrap();
        let end = *activity_timestamps.last().unwrap();
        return vec![Segment {
            start,
            end,
            duration_secs: (end - start).num_seconds().max(0),
            mode: mode_at(mode_events, end),
            turn_count: activity_timestamps.len(),
        }];
    }

    let last_event = activity_timestamps
        .last()
        .copied()
        .unwrap_or(*user_timestamps.last().unwrap());

    let mut segments = Vec::with_capacity(user_timestamps.len());
    for i in 0..user_timestamps.len() {
        let seg_start = user_timestamps[i];
        let seg_end = match user_timestamps.get(i + 1) {
            Some(&next_user) => activity_timestamps
                .iter()
                .rev()
                .find(|&&t| t < next_user)
                .copied()
                .unwrap_or(seg_start),
            None => last_event.max(seg_start),
        };
        let turn_count = activity_timestamps
            .iter()
            .filter(|&&t| t >= seg_start && t <= seg_end)
            .count();
        segments.push(Segment {
            start: seg_start,
            end: seg_end,
            duration_secs: (seg_end - seg_start).num_seconds().max(0),
            mode: mode_at(mode_events, seg_end),
            turn_count,
        });
    }
    segments
}

pub fn pick_display_segment(segments: &[Segment], status: Status) -> (Segment, DisplayKind) {
    match status {
        Status::Running => (segments.last().cloned().unwrap(), DisplayKind::Current),
        Status::Finished => (segments.last().cloned().unwrap(), DisplayKind::Recent),
        Status::Stopped => {
            let longest = segments
                .iter()
                .max_by_key(|s| s.duration_secs)
                .cloned()
                .unwrap();
            (longest, DisplayKind::Longest)
        }
    }
}

fn mode_at(events: &[(DateTime<Utc>, String)], when: DateTime<Utc>) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|(t, _)| *t <= when)
        .map(|(_, m)| m.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    // A user works for ~5min in the evening, leaves, then resumes the next morning.
    // The evening turn must end when work stopped, not when the resume happened —
    // the overnight gap belongs to neither turn.
    #[test]
    fn overnight_idle_is_not_billed_to_prior_turn() {
        let evening = t("2026-05-25T18:42:00Z");
        let evening_end = t("2026-05-25T18:47:00Z");
        let morning = t("2026-05-26T11:19:01Z");
        let morning_end = t("2026-05-26T11:20:00Z");

        let users = vec![evening, morning];
        // Boundaries are conversation events only — the resume's bookkeeping row
        // (which lands ~1ms before `morning`) has been filtered out upstream.
        let activity = vec![evening, evening_end, morning, morning_end];

        let segs = build_segments(&users, &activity, &[]);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].duration_secs, 5 * 60, "evening turn = real work only");
        assert_eq!(segs[1].duration_secs, 59);
    }

    // Sanity: a genuinely long agentic run (events spread across the gap) stays long.
    #[test]
    fn continuous_work_stays_long() {
        let start = t("2026-05-25T10:00:00Z");
        let mid = t("2026-05-25T11:30:00Z");
        let end = t("2026-05-25T13:00:00Z");
        let segs = build_segments(&[start], &[start, mid, end], &[]);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].duration_secs, 3 * 60 * 60);
    }
}

#[tauri::command]
pub fn list_sessions(
    codex_home: Option<String>,
    claude_home: Option<String>,
) -> Vec<Session> {
    let mut out = Vec::new();
    out.extend(crate::codex::scan(codex_home.as_deref()));
    out.extend(crate::claude::scan(claude_home.as_deref()));
    out.sort_by(|a, b| {
        b.display_segment
            .duration_secs
            .cmp(&a.display_segment.duration_secs)
    });
    out
}
