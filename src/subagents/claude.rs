//! Claude Code source: `<dir>/<sid>/subagents/agent-<id>.meta.json`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;

use super::{SubagentRecord, SubagentSource};

pub struct ClaudeSubagentSource;

/// Subset of Claude's meta.json that we use; unknown fields are ignored and
/// optional fields may be absent.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    #[serde(default)]
    agent_type: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tool_use_id: Option<String>,
    #[serde(default)]
    spawn_depth: Option<u32>,
    #[serde(default)]
    request_shape: Option<String>,
    /// Absent while running; `false` is treated like absent.
    #[serde(default)]
    finished: Option<bool>,
}

const META_PREFIX: &str = "agent-";
const META_SUFFIX: &str = ".meta.json";

/// Extract the agent id from an `agent-<id>.meta.json` file name.
fn agent_id_from_meta_name(name: &str) -> Option<&str> {
    let id = name.strip_prefix(META_PREFIX)?.strip_suffix(META_SUFFIX)?;
    (!id.is_empty()).then_some(id)
}

fn parse_meta(id: &str, bytes: &[u8], created_at: Option<SystemTime>) -> Option<SubagentRecord> {
    let meta: Meta = serde_json::from_slice(bytes).ok()?;
    Some(SubagentRecord {
        id: id.to_string(),
        kind: meta.agent_type,
        description: meta.description,
        tool_use_id: meta.tool_use_id.filter(|s| !s.is_empty()),
        // Claude omits depth for plain subagents in older formats; treat as 1.
        spawn_depth: meta.spawn_depth.unwrap_or(1),
        background: meta.request_shape.as_deref() == Some("background"),
        finished: meta.finished == Some(true),
        created_at,
    })
}

impl SubagentSource for ClaudeSubagentSource {
    fn subagent_dir(&self, transcript: &Path) -> Option<PathBuf> {
        let stem = transcript.file_stem()?;
        if transcript.extension()? != "jsonl" {
            return None;
        }
        Some(transcript.parent()?.join(stem).join("subagents"))
    }

    fn scan(&self, dir: &Path) -> Vec<SubagentRecord> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut records = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(agent_id_from_meta_name) else {
                continue;
            };
            let path = entry.path();
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(err) => {
                    tracing::debug!(?path, %err, "skipping unreadable subagent meta");
                    continue;
                }
            };
            let created_at = entry
                .metadata()
                .ok()
                .and_then(|m| m.created().or_else(|_| m.modified()).ok());
            match parse_meta(id, &bytes, created_at) {
                Some(r) => records.push(r),
                None => tracing::debug!(?path, "skipping malformed subagent meta"),
            }
        }
        records.sort_by(|a, b| a.id.cmp(&b.id));
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagents::test_util::TempDir;
    use std::fs;

    const RUNNING: &str = r#"{"agentType":"general-purpose","isBuiltIn":true,"description":"Wave 2a: schemas","toolUseId":"toolu_019fx","spawnDepth":1,"requestShape":"background","requestNonInteractive":true,"model":"sonnet"}"#;
    const FINISHED: &str = r#"{"agentType":"Explore","description":"look","toolUseId":"toolu_2","spawnDepth":2,"requestShape":"foreground","finished":true,"pickedEffortModel":"x","futureField":{"a":1}}"#;

    fn write(dir: &Path, name: &str, body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn subagent_dir_layout() {
        let s = ClaudeSubagentSource;
        assert_eq!(
            s.subagent_dir(Path::new("/p/proj/abc-123.jsonl")),
            Some(PathBuf::from("/p/proj/abc-123/subagents"))
        );
        assert_eq!(s.subagent_dir(Path::new("/p/proj/abc-123.txt")), None);
        assert_eq!(s.subagent_dir(Path::new("/")), None);
    }

    #[test]
    fn scans_running_and_finished() {
        let t = TempDir::new();
        write(t.path(), "agent-aaa.meta.json", RUNNING);
        write(t.path(), "agent-bbb.meta.json", FINISHED);
        write(t.path(), "agent-aaa.jsonl", "{}\n");
        write(t.path(), "notes.txt", "x");
        let recs = ClaudeSubagentSource.scan(t.path());
        assert_eq!(recs.len(), 2);
        let a = &recs[0];
        assert_eq!(a.id, "aaa");
        assert_eq!(a.kind, "general-purpose");
        assert_eq!(a.description, "Wave 2a: schemas");
        assert_eq!(a.tool_use_id.as_deref(), Some("toolu_019fx"));
        assert_eq!(a.spawn_depth, 1);
        assert!(a.background);
        assert!(!a.finished);
        assert!(a.created_at.is_some());
        let b = &recs[1];
        assert_eq!(b.id, "bbb");
        assert_eq!(b.spawn_depth, 2);
        assert!(!b.background);
        assert!(b.finished);
    }

    #[test]
    fn finished_false_is_running() {
        let r = parse_meta("x", br#"{"agentType":"t","finished":false}"#, None).unwrap();
        assert!(!r.finished);
    }

    #[test]
    fn minimal_meta_uses_defaults() {
        let r = parse_meta("x", b"{}", None).unwrap();
        assert_eq!(r.kind, "");
        assert_eq!(r.tool_use_id, None);
        assert_eq!(r.spawn_depth, 1);
    }

    #[test]
    fn malformed_and_truncated_are_skipped() {
        let t = TempDir::new();
        write(t.path(), "agent-good.meta.json", RUNNING);
        write(t.path(), "agent-bad.meta.json", "not json");
        write(t.path(), "agent-trunc.meta.json", &RUNNING[..40]);
        write(t.path(), "agent-empty.meta.json", "");
        write(t.path(), "agent-.meta.json", RUNNING);
        let recs = ClaudeSubagentSource.scan(t.path());
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].id, "good");
    }

    #[test]
    fn missing_dir_is_empty() {
        let t = TempDir::new();
        assert!(ClaudeSubagentSource.scan(&t.path().join("nope")).is_empty());
    }
}
