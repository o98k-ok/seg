use chrono::{DateTime, Utc};
use std::{
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

use crate::sessions::{
    build_segments, pick_display_segment, status_from_age_secs, Goal, Session, Source,
};

fn projects_dir(home_override: Option<&str>) -> PathBuf {
    let base = home_override
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(crate::sessions::expand_tilde)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".claude")
        });
    base.join("projects")
}

pub fn scan(home_override: Option<&str>) -> Vec<Session> {
    let dir = projects_dir(home_override);
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
    let mut latest_mode: Option<String> = None;
    let mut current_running_mode: Option<String> = None;
    let mut has_plan_mode = false;
    let mut latest_goal: Option<Goal> = None;

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
        let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");

        if let Some(s) = v.get("sessionId").and_then(|x| x.as_str()) {
            if session_id.is_none() {
                session_id = Some(s.into());
            }
        }
        if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
            if cwd.is_none() {
                cwd = Some(c.into());
            }
        }
        if let Some(g) = v.get("gitBranch").and_then(|x| x.as_str()) {
            if git_branch.is_none() {
                git_branch = Some(g.into());
            }
        }
        if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
            if cli_version.is_none() {
                cli_version = Some(ver.into());
            }
        }
        if let Some(pm) = v.get("permissionMode").and_then(|x| x.as_str()) {
            if pm.eq_ignore_ascii_case("plan") {
                has_plan_mode = true;
            }
            current_running_mode = Some(pm.into());
            latest_mode = Some(pm.into());
        }

        let ts = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc));

        if let Some(t) = ts {
            timestamps.push(t);
            if is_segment_activity(typ) {
                activity_timestamps.push(t);
            }
            if let Some(m) = current_running_mode.clone() {
                mode_events.push((t, m));
            }
        }

        match typ {
            "user" | "assistant" => {
                let is_sidechain = v
                    .get("isSidechain")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                let is_meta = v.get("isMeta").and_then(|x| x.as_bool()).unwrap_or(false);
                if !is_sidechain && !is_meta {
                    turn_count += 1;
                }
                if typ == "assistant" {
                    if let Some(m) = v
                        .get("message")
                        .and_then(|m| m.get("model"))
                        .and_then(|x| x.as_str())
                    {
                        latest_model = Some(m.into());
                    }
                }
                if typ == "user" && !is_sidechain && !is_meta && is_real_user_input(&v) {
                    if let Some(t) = ts {
                        user_timestamps.push(t);
                    }
                }
                // Claude's GOAL mode is the `/goal` slash command, recorded as a
                // user message whose string content wraps the objective in
                // <command-args>. Latest /goal wins.
                if typ == "user" {
                    if let Some(content) = v
                        .get("message")
                        .and_then(|m| m.get("content"))
                        .and_then(|c| c.as_str())
                    {
                        if extract_tag(content, "command-name") == Some("/goal") {
                            if let Some(obj) =
                                extract_tag(content, "command-args").filter(|s| !s.is_empty())
                            {
                                latest_goal = Some(Goal {
                                    objective: obj.to_string(),
                                    status: None,
                                    time_used_seconds: None,
                                });
                            }
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
    // events only so an idle gap before a resume isn't billed to the prior turn.
    let boundary_ts: &[DateTime<Utc>] = if activity_timestamps.is_empty() {
        &timestamps
    } else {
        &activity_timestamps
    };
    let segments = build_segments(&user_timestamps, boundary_ts, &mode_events);
    let (display_segment, display_segment_kind) = pick_display_segment(&segments, status);
    let segment_count = segments.len();

    Some(Session {
        source: Source::Claude,
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
        reasoning_effort: None,
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

/// A `type:"user"` line in a Claude session jsonl can be:
///   1. a real user prompt (string content, possibly wrapped in `<command-name>`)
///   2. a CLI-injected note like `<local-command-stdout>` / `<local-command-caveat>` — not user-typed
///   3. a tool_result fed back to the model (array content with `tool_result` items)
/// Only (1) should act as a segment boundary.
/// Extract the trimmed inner text of `<tag>…</tag>` from a string, if present.
fn extract_tag<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let rest = &s[start..];
    let end = rest.find(&close)?;
    Some(rest[..end].trim())
}

/// Whether a row counts as conversational activity for segment boundaries.
/// Claude Code writes these bookkeeping rows at prompt-submit/resume time with the
/// new turn's timestamp; letting them end a segment would absorb the idle gap that
/// preceded the resume into the prior turn (e.g. an overnight pause as a 16h turn).
fn is_segment_activity(typ: &str) -> bool {
    !matches!(
        typ,
        "attachment" | "file-history-snapshot" | "permission-mode" | "summary"
    )
}

fn is_real_user_input(v: &serde_json::Value) -> bool {
    let content = match v.get("message").and_then(|m| m.get("content")) {
        Some(c) => c,
        None => return false,
    };
    if let Some(s) = content.as_str() {
        return !s.starts_with("<local-command-");
    }
    if let Some(arr) = content.as_array() {
        return arr.iter().any(|item| {
            item.get("type").and_then(|x| x.as_str()) != Some("tool_result")
        });
    }
    false
}
