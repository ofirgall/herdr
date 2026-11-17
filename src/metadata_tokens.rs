use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
struct MetadataToken {
    value: String,
    link: Option<String>,
    expires_at: Option<Instant>,
}

/// A metadata patch: `Some((value, link))` sets a token, `None` clears it.
pub(crate) type MetadataTokenPatch = HashMap<String, Option<(String, Option<String>)>>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MetadataTokens {
    entries: HashMap<String, MetadataToken>,
}

pub(crate) const MAX_SEQUENCE_SOURCES: usize = 32;

pub(crate) fn sequence_is_fresh(
    sequences: &HashMap<String, u64>,
    source: &str,
    seq: Option<u64>,
) -> bool {
    seq.is_none_or(|seq| sequences.get(source).is_none_or(|last| seq > *last))
}

pub(crate) fn accept_sequence(
    sequences: &mut HashMap<String, u64>,
    source: &str,
    seq: Option<u64>,
) -> Result<bool, ()> {
    let Some(seq) = seq else {
        return Ok(true);
    };
    if !sequence_is_fresh(sequences, source, Some(seq)) {
        return Ok(false);
    }
    if !sequences.contains_key(source) && sequences.len() >= MAX_SEQUENCE_SOURCES {
        return Err(());
    }
    sequences.insert(source.to_string(), seq);
    Ok(true)
}

impl MetadataTokens {
    pub(crate) fn patch(
        &mut self,
        patch: MetadataTokenPatch,
        ttl: Option<Duration>,
        now: Instant,
    ) -> bool {
        let expires_at = ttl.and_then(|ttl| now.checked_add(ttl));
        let mut changed = false;
        for (key, value) in patch {
            match value {
                Some((value, link)) => {
                    let token = MetadataToken {
                        value,
                        link,
                        expires_at,
                    };
                    if self.entries.get(&key) != Some(&token) {
                        self.entries.insert(key, token);
                        changed = true;
                    }
                }
                None => {
                    changed |= self.entries.remove(&key).is_some();
                }
            }
        }
        changed
    }

    pub(crate) fn key_count_after_patch(&self, patch: &MetadataTokenPatch) -> usize {
        let mut keys = self
            .values()
            .into_keys()
            .collect::<std::collections::HashSet<_>>();
        for (key, value) in patch {
            if value.is_some() {
                keys.insert(key.clone());
            } else {
                keys.remove(key);
            }
        }
        keys.len()
    }

    pub(crate) fn values(&self) -> HashMap<String, String> {
        self.entries
            .iter()
            .map(|(key, token)| (key.clone(), token.value.clone()))
            .collect()
    }

    /// Links of the tokens that have one, keyed by token name.
    pub(crate) fn links(&self) -> HashMap<String, String> {
        self.entries
            .iter()
            .filter_map(|(key, token)| Some((key.clone(), token.link.clone()?)))
            .collect()
    }

    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.entries
            .values()
            .filter_map(|token| token.expires_at)
            .min()
    }

    pub(crate) fn expire_at(&mut self, now: Instant) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|_, token| token.expires_at.is_none_or(|deadline| deadline > now));
        self.entries.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(items: &[(&str, Option<&str>)]) -> MetadataTokenPatch {
        items
            .iter()
            .map(|(key, value)| ((*key).into(), value.map(|v| (v.to_string(), None))))
            .collect()
    }

    fn linked(key: &str, value: &str, link: &str) -> MetadataTokenPatch {
        HashMap::from([(key.into(), Some((value.into(), Some(link.into()))))])
    }

    #[test]
    fn sequence_sources_are_bounded() {
        let mut sequences = HashMap::new();
        for index in 0..MAX_SEQUENCE_SOURCES {
            assert_eq!(
                accept_sequence(&mut sequences, &format!("source-{index}"), Some(1)),
                Ok(true)
            );
        }
        assert_eq!(
            accept_sequence(&mut sequences, "one-too-many", Some(1)),
            Err(())
        );
        assert_eq!(
            accept_sequence(&mut sequences, "source-0", Some(1)),
            Ok(false)
        );
        assert_eq!(
            accept_sequence(&mut sequences, "source-0", Some(2)),
            Ok(true)
        );
    }

    #[test]
    fn patches_and_clears_individual_keys() {
        let now = Instant::now();
        let mut tokens = MetadataTokens::default();
        tokens.patch(
            patch(&[("summary", Some("one")), ("model", Some("opus"))]),
            None,
            now,
        );
        tokens.patch(
            patch(&[("summary", Some("two")), ("model", None)]),
            None,
            now,
        );

        assert_eq!(
            tokens.values(),
            HashMap::from([("summary".into(), "two".into())])
        );
    }

    #[test]
    fn ttl_only_changes_keys_in_the_patch() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(1);
        let mut tokens = MetadataTokens::default();
        tokens.patch(
            patch(&[("short", Some("one"))]),
            Some(Duration::from_secs(1)),
            now,
        );
        tokens.patch(patch(&[("persistent", Some("two"))]), None, now);

        assert!(tokens.expire_at(deadline));
        assert_eq!(
            tokens.values(),
            HashMap::from([("persistent".into(), "two".into())])
        );
    }

    #[test]
    fn values_remain_stable_until_expiry_mutates_state() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(1);
        let mut tokens = MetadataTokens::default();
        tokens.patch(
            patch(&[("summary", Some("temporary"))]),
            Some(Duration::from_secs(1)),
            now,
        );

        assert_eq!(
            tokens.values(),
            HashMap::from([("summary".into(), "temporary".into())])
        );
        assert!(tokens.expire_at(deadline));
        assert!(tokens.values().is_empty());
    }

    #[test]
    fn delayed_expiry_sweep_removes_every_token_due_at_now() {
        let now = Instant::now();
        let mut tokens = MetadataTokens::default();
        tokens.patch(
            patch(&[("first", Some("one"))]),
            Some(Duration::from_secs(1)),
            now,
        );
        tokens.patch(
            patch(&[("second", Some("two"))]),
            Some(Duration::from_secs(2)),
            now,
        );

        assert!(tokens.expire_at(now + Duration::from_secs(10)));
        assert!(tokens.values().is_empty());
    }

    #[test]
    fn stale_expiry_does_not_clear_replacement() {
        let now = Instant::now();
        let first_deadline = now + Duration::from_secs(1);
        let mut tokens = MetadataTokens::default();
        tokens.patch(
            patch(&[("summary", Some("old"))]),
            Some(Duration::from_secs(1)),
            now,
        );
        tokens.patch(
            patch(&[("summary", Some("new"))]),
            Some(Duration::from_secs(5)),
            now,
        );

        assert!(!tokens.expire_at(first_deadline));
        assert_eq!(
            tokens.values(),
            HashMap::from([("summary".into(), "new".into())])
        );
    }

    #[test]
    fn update_without_ttl_cancels_previous_expiry() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(1);
        let mut tokens = MetadataTokens::default();
        tokens.patch(
            patch(&[("summary", Some("temporary"))]),
            Some(Duration::from_secs(1)),
            now,
        );
        tokens.patch(patch(&[("summary", Some("persistent"))]), None, now);

        assert!(!tokens.expire_at(deadline));
        assert_eq!(
            tokens.values(),
            HashMap::from([("summary".into(), "persistent".into())])
        );
    }

    #[test]
    fn link_is_stored_with_the_token_and_replaced_by_later_values() {
        let now = Instant::now();
        let mut tokens = MetadataTokens::default();
        assert!(tokens.patch(linked("pr", "#1", "https://example.com/1"), None, now));
        assert_eq!(
            tokens.links(),
            HashMap::from([("pr".into(), "https://example.com/1".into())])
        );

        // Same value and link is not a change.
        assert!(!tokens.patch(linked("pr", "#1", "https://example.com/1"), None, now));
        // A link-only change is a change.
        assert!(tokens.patch(linked("pr", "#1", "https://example.com/2"), None, now));
        assert_eq!(tokens.links()["pr"], "https://example.com/2");

        // Re-setting without a link clears it.
        assert!(tokens.patch(patch(&[("pr", Some("#1"))]), None, now));
        assert!(tokens.links().is_empty());
        assert_eq!(tokens.values(), HashMap::from([("pr".into(), "#1".into())]));
    }

    #[test]
    fn clearing_a_token_removes_its_link() {
        let now = Instant::now();
        let mut tokens = MetadataTokens::default();
        tokens.patch(linked("pr", "#1", "https://example.com/1"), None, now);
        assert!(tokens.patch(patch(&[("pr", None)]), None, now));
        assert!(tokens.values().is_empty());
        assert!(tokens.links().is_empty());
    }

    #[test]
    fn expiry_removes_the_link_with_the_token() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(1);
        let mut tokens = MetadataTokens::default();
        tokens.patch(
            linked("pr", "#1", "https://example.com/1"),
            Some(Duration::from_secs(1)),
            now,
        );
        assert!(tokens.expire_at(deadline));
        assert!(tokens.values().is_empty());
        assert!(tokens.links().is_empty());
    }
}
