//! Backfill from `~/.claude/projects/<sanitized-cwd>/<session-id>.jsonl`.
//!
//! Every Claude Code host writes its transcript here (verify for the desktop
//! app on day 1). Each line is one JSON object. The fields below are from
//! memory of the format, not from a live file, so the parser reads
//! defensively: it looks for `type`, `sessionId`, `cwd`, `timestamp`,
//! `gitBranch`, and `message.usage`, and ignores anything else.
//!
//! Sanity-check against a real file before trusting the counts:
//!     head -3 ~/.claude/projects/*/*.jsonl | jq .

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct TranscriptSummary {
    pub session_id: String,
    pub path: String,
    pub cwd: String,
    pub git_branch: Option<String>,
    pub title: Option<String>,
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    pub user_messages: u32,
    pub assistant_messages: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub fn projects_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

/// Walk every project directory and summarize every transcript.
/// Unreadable or empty files are skipped silently; this runs on a click.
pub fn scan_all() -> Vec<TranscriptSummary> {
    let Some(root) = projects_dir() else {
        return vec![];
    };
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(&root) else {
        return out;
    };
    for project in projects.flatten() {
        let Ok(files) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Some(s) = summarize(&p) {
                    out.push(s);
                }
            }
        }
    }
    out
}

pub fn summarize(path: &Path) -> Option<TranscriptSummary> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut git_branch: Option<String> = None;
    let mut title: Option<String> = None;
    let mut first_at: Option<DateTime<Utc>> = None;
    let mut last_at: Option<DateTime<Utc>> = None;
    let mut user_messages = 0u32;
    let mut assistant_messages = 0u32;
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;

    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if session_id.is_none() {
            session_id = str_field(&v, "sessionId");
        }
        if cwd.is_none() {
            cwd = str_field(&v, "cwd");
        }
        if git_branch.is_none() {
            git_branch = str_field(&v, "gitBranch");
        }
        if let Some(ts) = str_field(&v, "timestamp").and_then(|t| t.parse::<DateTime<Utc>>().ok()) {
            if first_at.is_none() {
                first_at = Some(ts);
            }
            last_at = Some(ts);
        }

        match v.get("type").and_then(Value::as_str) {
            Some("user") => {
                // Tool results also arrive as type=user; only count real
                // human turns, which have a string (or text-block) content.
                if is_human_turn(&v) {
                    user_messages += 1;
                }
            }
            Some("assistant") => {
                assistant_messages += 1;
                if let Some(u) = v.pointer("/message/usage") {
                    input_tokens += u64_field(u, "input_tokens");
                    input_tokens += u64_field(u, "cache_read_input_tokens");
                    input_tokens += u64_field(u, "cache_creation_input_tokens");
                    output_tokens += u64_field(u, "output_tokens");
                }
            }
            Some("summary") => {
                // Claude Code writes a rolling summary line; use it as a title.
                if let Some(s) = str_field(&v, "summary") {
                    title = Some(s);
                }
            }
            _ => {}
        }
    }

    // Fall back to the filename for the session id; that's how the files
    // are named.
    let session_id = session_id.or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
    })?;
    let last_at = last_at?;
    let first_at = first_at.unwrap_or(last_at);

    Some(TranscriptSummary {
        session_id,
        path: path.to_string_lossy().into_owned(),
        cwd: cwd.unwrap_or_default(),
        git_branch,
        title,
        first_at,
        last_at,
        user_messages,
        assistant_messages,
        input_tokens,
        output_tokens,
    })
}

fn is_human_turn(v: &Value) -> bool {
    match v.pointer("/message/content") {
        Some(Value::String(_)) => true,
        Some(Value::Array(blocks)) => blocks
            .iter()
            .any(|b| b.get("type").and_then(Value::as_str) == Some("text")),
        _ => false,
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn u64_field(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn summarizes_minimal_transcript() {
        let dir = std::env::temp_dir().join("aperture-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("sess-1.jsonl");
        let mut f = File::create(&p).unwrap();
        writeln!(f, r#"{{"type":"user","sessionId":"sess-1","cwd":"/r","gitBranch":"main","timestamp":"2026-09-04T10:00:00Z","message":{{"role":"user","content":"hi"}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"assistant","timestamp":"2026-09-04T10:00:05Z","message":{{"role":"assistant","content":[{{"type":"text","text":"hello"}}],"usage":{{"input_tokens":10,"output_tokens":5}}}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"user","timestamp":"2026-09-04T10:00:06Z","message":{{"role":"user","content":[{{"type":"tool_result","content":"ok"}}]}}}}"#).unwrap();

        let s = summarize(&p).unwrap();
        assert_eq!(s.session_id, "sess-1");
        assert_eq!(s.cwd, "/r");
        assert_eq!(s.user_messages, 1); // tool_result not counted
        assert_eq!(s.assistant_messages, 1);
        assert_eq!(s.input_tokens, 10);
        assert_eq!(s.output_tokens, 5);
    }
}
