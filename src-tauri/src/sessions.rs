use chrono::{DateTime, Utc};
use serde::Serialize;

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
/// segment. `all_timestamps` and `user_timestamps` must both be sorted ascending.
pub fn build_segments(
    user_timestamps: &[DateTime<Utc>],
    all_timestamps: &[DateTime<Utc>],
    mode_events: &[(DateTime<Utc>, String)],
) -> Vec<Segment> {
    if user_timestamps.is_empty() {
        // No user message; fall back to a single segment spanning the whole file.
        if all_timestamps.is_empty() {
            return Vec::new();
        }
        let start = *all_timestamps.first().unwrap();
        let end = *all_timestamps.last().unwrap();
        return vec![Segment {
            start,
            end,
            duration_secs: (end - start).num_seconds().max(0),
            mode: mode_at(mode_events, end),
            turn_count: all_timestamps.len(),
        }];
    }

    let last_event = all_timestamps
        .last()
        .copied()
        .unwrap_or(*user_timestamps.last().unwrap());

    let mut segments = Vec::with_capacity(user_timestamps.len());
    for i in 0..user_timestamps.len() {
        let seg_start = user_timestamps[i];
        let seg_end = match user_timestamps.get(i + 1) {
            Some(&next_user) => all_timestamps
                .iter()
                .rev()
                .find(|&&t| t < next_user)
                .copied()
                .unwrap_or(seg_start),
            None => last_event.max(seg_start),
        };
        let turn_count = all_timestamps
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

#[tauri::command]
pub fn list_sessions() -> Vec<Session> {
    let mut out = Vec::new();
    out.extend(crate::codex::scan());
    out.extend(crate::claude::scan());
    out.sort_by(|a, b| {
        b.display_segment
            .duration_secs
            .cmp(&a.display_segment.duration_secs)
    });
    out
}
