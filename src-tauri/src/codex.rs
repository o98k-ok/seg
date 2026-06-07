use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::{
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

use crate::sessions::{
    build_segments, pick_display_segment, status_from_age_secs, Goal, Session, Source,
};

fn sessions_dirs(home_override: Option<&str>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(input) = home_override.map(str::trim).filter(|s| !s.is_empty()) {
        for home in input.split([',', ';']) {
            let home = home.trim();
            if !home.is_empty() {
                push_codex_home_dirs(&mut dirs, crate::sessions::expand_tilde(home));
            }
        }
    }

    if dirs.is_empty() {
        let home = std::env::var("HOME").unwrap_or_default();
        push_codex_home_dirs(&mut dirs, PathBuf::from(home).join(".codex"));
    }

    dirs
}

fn push_codex_home_dirs(dirs: &mut Vec<PathBuf>, home: PathBuf) {
    push_unique_path(dirs, home.join("sessions"));

    if let Ok(entries) = fs::read_dir(&home) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() && path.join("sessions").is_dir() {
                push_unique_path(dirs, path.join("sessions"));
            }
        }
    }
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|p| p == &path) {
        paths.push(path);
    }
}

pub fn scan(home_override: Option<&str>) -> Vec<Session> {
    let mut out = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();

    for dir in sessions_dirs(home_override) {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(s) = parse_file(path) {
                if seen_ids.contains(&s.id) || seen_paths.contains(&s.file_path) {
                    continue;
                }
                seen_ids.insert(s.id.clone());
                seen_paths.insert(s.file_path.clone());
                out.push(s);
            }
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
    user_timestamps.sort();
    mode_events.sort_by_key(|(t, _)| *t);
    let started_at = *timestamps.first().unwrap();
    let last_event_at = *timestamps.last().unwrap();
    let total_duration_secs = (last_event_at - started_at).num_seconds().max(0);
    let status = status_from_age_secs((Utc::now() - last_event_at).num_seconds());

    let segments = build_segments(&user_timestamps, &timestamps, &mode_events);
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
        status: v.get("status").and_then(|x| x.as_str()).map(String::from),
        time_used_seconds: v.get("timeUsedSeconds").and_then(|x| x.as_i64()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_codex_home(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("seg-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_rollout(path: &Path, id: &str, cwd: &str, ts: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\",\"cli_version\":\"test\"}}}}\n\
             {{\"timestamp\":\"{ts}\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\"}}}}\n"
        );
        fs::write(path, text).unwrap();
    }

    #[test]
    fn scans_top_level_and_profile_sessions() {
        let home = temp_codex_home("profiles");
        write_rollout(
            &home.join("sessions/2026/06/07/rollout-top.jsonl"),
            "top",
            "/work/top",
            "2026-06-07T01:00:00Z",
        );
        write_rollout(
            &home.join("profile/sessions/2026/06/07/rollout-profile.jsonl"),
            "profile",
            "/work/profile",
            "2026-06-07T02:00:00Z",
        );

        let sessions = scan(Some(home.to_str().unwrap()));
        let ids: HashSet<_> = sessions.iter().map(|s| s.id.as_str()).collect();

        assert!(ids.contains("top"));
        assert!(ids.contains("profile"));

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn scans_multiple_explicit_homes_without_splitting_spaces() {
        let home_a = temp_codex_home("multi-a");
        let home_b = temp_codex_home("multi b");
        let home_c = temp_codex_home("multi-c");

        write_rollout(
            &home_a.join("sessions/2026/06/07/rollout-a.jsonl"),
            "multi-a",
            "/work/a",
            "2026-06-07T01:00:00Z",
        );
        write_rollout(
            &home_b.join("sessions/2026/06/07/rollout-b.jsonl"),
            "multi-b",
            "/work/b",
            "2026-06-07T02:00:00Z",
        );
        write_rollout(
            &home_c.join("sessions/2026/06/07/rollout-c.jsonl"),
            "multi-c",
            "/work/c",
            "2026-06-07T03:00:00Z",
        );

        let input = format!(
            "{}, {}; {}",
            home_a.to_string_lossy(),
            home_b.to_string_lossy(),
            home_c.to_string_lossy()
        );
        let sessions = scan(Some(&input));
        let ids: HashSet<_> = sessions.iter().map(|s| s.id.as_str()).collect();

        assert!(ids.contains("multi-a"));
        assert!(ids.contains("multi-b"));
        assert!(ids.contains("multi-c"));

        fs::remove_dir_all(home_a).unwrap();
        fs::remove_dir_all(home_b).unwrap();
        fs::remove_dir_all(home_c).unwrap();
    }
}
