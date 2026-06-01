use chrono::{DateTime, Utc};
use std::{
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

use crate::sessions::{
    build_segments, pick_display_segment, status_from_age_secs, Goal, Session, Source,
};

fn sessions_dir(home_override: Option<&str>) -> PathBuf {
    let base = home_override
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(crate::sessions::expand_tilde)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".codex")
        });
    base.join("sessions")
}

pub fn scan(home_override: Option<&str>) -> Vec<Session> {
    let dir = sessions_dir(home_override);
    if !dir.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        if let Some(s) = parse_file(path) {
            out.push(s);
        }
    }
    out
}

fn parse_file(path: &Path) -> Option<Session> {
    let bytes = fs::read(path).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let file_size_bytes = fs::metadata(path).ok()?.len();

    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut git_branch: Option<String> = None;
    let mut cli_version: Option<String> = None;
    let mut latest_model: Option<String> = None;
    let mut latest_effort: Option<String> = None;
    let mut latest_mode: Option<String> = None;
    let mut latest_goal: Option<Goal> = None;
    let mut has_plan_mode = false;

    let mut timestamps: Vec<DateTime<Utc>> = Vec::new();
    let mut activity_timestamps: Vec<DateTime<Utc>> = Vec::new();
    let mut user_timestamps: Vec<DateTime<Utc>> = Vec::new();
    let mut mode_events: Vec<(DateTime<Utc>, String)> = Vec::new();
    let mut turn_count = 0usize;

    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc));
        let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");

        if let Some(t) = ts {
            timestamps.push(t);
        }

        match typ {
            "session_meta" => {
                if let Some(p) = v.get("payload") {
                    session_id = p.get("id").and_then(|x| x.as_str()).map(String::from);
                    cwd = p.get("cwd").and_then(|x| x.as_str()).map(String::from);
                    cli_version = p
                        .get("cli_version")
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    git_branch = p
                        .get("git")
                        .and_then(|g| g.get("branch"))
                        .and_then(|x| x.as_str())
                        .map(String::from);
                }
            }
            "turn_context" => {
                if let Some(p) = v.get("payload") {
                    if let Some(m) = p.get("model").and_then(|x| x.as_str()) {
                        latest_model = Some(m.into());
                    }
                    if let Some(e) = p.get("reasoning_effort").and_then(|x| x.as_str()) {
                        latest_effort = Some(e.into());
                    }
                    let mode = p
                        .get("collaboration_mode")
                        .and_then(|cm| cm.get("mode"))
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    if let Some(m) = mode.clone() {
                        if m.eq_ignore_ascii_case("plan") {
                            has_plan_mode = true;
                        }
                        latest_mode = Some(m);
                    }
                    if let (Some(t), Some(m)) = (ts, mode) {
                        mode_events.push((t, m));
                    }
                }
            }
            "response_item" => {
                // Only real conversation items define segment boundaries; metadata
                // rows (`event_msg` like task_started, `turn_context`) are written at
                // turn start and would otherwise absorb the idle gap before a resume.
                if let Some(t) = ts {
                    activity_timestamps.push(t);
                }
                if let Some(p) = v.get("payload") {
                    let inner = p.get("type").and_then(|x| x.as_str()).unwrap_or("");
                    if inner == "message" {
                        let role = p.get("role").and_then(|x| x.as_str()).unwrap_or("");
                        if role == "user" || role == "assistant" {
                            turn_count += 1;
                        }
                        if role == "user" {
                            if let Some(t) = ts {
                                user_timestamps.push(t);
                            }
                        }
                    }
                }
            }
            "event_msg" => {
                if let Some(p) = v.get("payload") {
                    let inner = p.get("type").and_then(|x| x.as_str()).unwrap_or("");
                    if inner == "thread_goal_updated" {
                        if let Some(goal) = p.get("goal").and_then(parse_goal) {
                            latest_goal = Some(goal);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if timestamps.is_empty() {
        return None;
    }
    timestamps.sort();
    activity_timestamps.sort();
    user_timestamps.sort();
    mode_events.sort_by_key(|(t, _)| *t);
    let started_at = *timestamps.first().unwrap();
    let last_event_at = *timestamps.last().unwrap();
    let total_duration_secs = (last_event_at - started_at).num_seconds().max(0);
    let status = status_from_age_secs((Utc::now() - last_event_at).num_seconds());

    // Span/status use every timestamp, but segment boundaries use conversation
    // items only so an idle gap before a resume isn't billed to the prior turn.
    let boundary_ts: &[DateTime<Utc>] = if activity_timestamps.is_empty() {
        &timestamps
    } else {
        &activity_timestamps
    };
    let segments = build_segments(&user_timestamps, boundary_ts, &mode_events);
    let (display_segment, display_segment_kind) = pick_display_segment(&segments, status);
    let segment_count = segments.len();

    Some(Session {
        source: Source::Codex,
        id: session_id.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string()
        }),
        cwd,
        git_branch,
        cli_version,
        model: latest_model,
        reasoning_effort: latest_effort,
        current_mode: latest_mode,
        has_plan_mode,
        goal: latest_goal,
        started_at,
        last_event_at,
        total_turns: turn_count,
        total_duration_secs,
        segment_count,
        segments,
        status,
        display_segment,
        display_segment_kind,
        file_path: path.to_string_lossy().into_owned(),
        file_size_bytes,
    })
}

fn parse_goal(v: &serde_json::Value) -> Option<Goal> {
    let objective = v.get("objective")?.as_str()?.trim();
    if objective.is_empty() {
        return None;
    }

    Some(Goal {
        objective: objective.to_string(),
        status: v
            .get("status")
            .and_then(|x| x.as_str())
            .map(String::from),
        time_used_seconds: v.get("timeUsedSeconds").and_then(|x| x.as_i64()),
    })
}
