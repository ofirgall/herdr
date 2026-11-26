//! Parent attribution via `toolUseId` (design D4).
//!
//! A nested subagent (`spawnDepth > 1`) was spawned by a `tool_use` block that
//! lives in a sibling `agent-<id>.jsonl`. We find it with a bounded raw byte
//! search; no JSON parsing.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use super::SubagentRecord;

/// Per-file read cap for the byte search.
const MAX_SEARCH_BYTES: u64 = 64 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;

/// Fill `cache` (subagent id -> parent subagent id) for records not yet
/// present. Depth <= 1 records, or records without a tool-use id, are cached
/// as `None` without any file I/O. Nested records whose spawner cannot be
/// found are cached as `None` (attached to the root).
pub fn attribute_parents(
    dir: &Path,
    records: &[SubagentRecord],
    cache: &mut HashMap<String, Option<String>>,
) {
    // Sibling transcripts, listed lazily at most once per call.
    let mut siblings: Option<Vec<(String, std::path::PathBuf)>> = None;
    for rec in records {
        if cache.contains_key(&rec.id) {
            continue;
        }
        let needle = match (&rec.tool_use_id, rec.spawn_depth > 1) {
            (Some(t), true) => t,
            _ => {
                cache.insert(rec.id.clone(), None);
                continue;
            }
        };
        let list = siblings.get_or_insert_with(|| list_sibling_transcripts(dir));
        let mut parent = None;
        for (id, path) in list.iter() {
            if *id == rec.id {
                continue;
            }
            let found = File::open(path)
                .and_then(|f| contains_bytes(f.take(MAX_SEARCH_BYTES), needle.as_bytes()));
            match found {
                Ok(true) => {
                    parent = Some(id.clone());
                    break;
                }
                Ok(false) => {}
                Err(err) => tracing::debug!(?path, %err, "subagent attribution read failed"),
            }
        }
        cache.insert(rec.id.clone(), parent);
    }
}

/// `agent-<id>.jsonl` files in `dir`, sorted by id for determinism.
fn list_sibling_transcripts(dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let id = name
                .to_str()?
                .strip_prefix("agent-")?
                .strip_suffix(".jsonl")?
                .to_string();
            (!id.is_empty()).then(|| (id, e.path()))
        })
        .collect();
    out.sort();
    out
}

/// Streaming substring search; stops at the first match.
fn contains_bytes<R: Read>(mut reader: R, needle: &[u8]) -> io::Result<bool> {
    if needle.is_empty() {
        return Ok(false);
    }
    let keep = needle.len() - 1;
    let mut buf: Vec<u8> = Vec::with_capacity(CHUNK + keep);
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            return Ok(false);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(needle.len()).any(|w| w == needle) {
            return Ok(true);
        }
        // Keep a tail so matches spanning chunk boundaries are found.
        if buf.len() > keep {
            buf.drain(..buf.len() - keep);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagents::test_util::TempDir;
    use std::io::Cursor;

    fn rec(id: &str, tool: Option<&str>, depth: u32) -> SubagentRecord {
        SubagentRecord {
            id: id.into(),
            kind: String::new(),
            description: String::new(),
            tool_use_id: tool.map(Into::into),
            spawn_depth: depth,
            background: false,
            finished: false,
            created_at: None,
            started_at: None,
            ended_at: None,
        }
    }

    #[test]
    fn contains_bytes_basic_and_boundary() {
        assert!(contains_bytes(Cursor::new(b"xxabcxx"), b"abc").unwrap());
        assert!(!contains_bytes(Cursor::new(b"xxabxcxx"), b"abc").unwrap());
        assert!(!contains_bytes(Cursor::new(b""), b"abc").unwrap());
        // Needle spanning the chunk boundary.
        let mut data = vec![b'a'; CHUNK - 2];
        data.extend_from_slice(b"NEEDLE");
        data.extend(vec![b'b'; 100]);
        assert!(contains_bytes(Cursor::new(data), b"NEEDLE").unwrap());
        // Single-byte needle.
        assert!(contains_bytes(Cursor::new(b"abc"), b"c").unwrap());
    }

    #[test]
    fn nested_hit_attributes_parent() {
        let t = TempDir::new();
        std::fs::write(
            t.path().join("agent-p.jsonl"),
            r#"{"type":"tool_use","id":"toolu_NEST"}"#,
        )
        .unwrap();
        std::fs::write(t.path().join("agent-other.jsonl"), "nothing").unwrap();
        std::fs::write(t.path().join("agent-c.jsonl"), "toolu_NEST in own file").unwrap();
        let recs = [rec("c", Some("toolu_NEST"), 2)];
        let mut cache = HashMap::new();
        attribute_parents(t.path(), &recs, &mut cache);
        assert_eq!(cache.get("c"), Some(&Some("p".to_string())));
    }

    #[test]
    fn unattributable_is_none_and_cached() {
        let t = TempDir::new();
        std::fs::write(t.path().join("agent-p.jsonl"), "unrelated").unwrap();
        let recs = [rec("c", Some("toolu_X"), 3)];
        let mut cache = HashMap::new();
        attribute_parents(t.path(), &recs, &mut cache);
        assert_eq!(cache.get("c"), Some(&None));
    }

    #[test]
    fn existing_cache_entries_are_not_recomputed() {
        let t = TempDir::new();
        std::fs::write(t.path().join("agent-p.jsonl"), "toolu_X").unwrap();
        let recs = [rec("c", Some("toolu_X"), 2)];
        let mut cache = HashMap::new();
        cache.insert("c".to_string(), None);
        attribute_parents(t.path(), &recs, &mut cache);
        assert_eq!(cache.get("c"), Some(&None));
    }

    #[test]
    fn depth_one_does_no_io() {
        // The directory does not exist; any listing would be observable only
        // as a wrong result, so also use a nested record with no tool id.
        let missing = std::env::temp_dir().join("herdr-subagents-does-not-exist-xyz");
        let recs = [rec("a", Some("toolu_1"), 1), rec("b", None, 2)];
        let mut cache = HashMap::new();
        attribute_parents(&missing, &recs, &mut cache);
        assert_eq!(cache.get("a"), Some(&None));
        assert_eq!(cache.get("b"), Some(&None));
    }

    #[test]
    fn depth_one_does_not_read_files() {
        // A sibling that would match, but depth 1 must stay root.
        let t = TempDir::new();
        std::fs::write(t.path().join("agent-p.jsonl"), "toolu_1").unwrap();
        let recs = [rec("a", Some("toolu_1"), 1)];
        let mut cache = HashMap::new();
        attribute_parents(t.path(), &recs, &mut cache);
        assert_eq!(cache.get("a"), Some(&None));
    }
}
