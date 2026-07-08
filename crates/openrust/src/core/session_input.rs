//! Session input admission — durable prompt tracking.
//!
//! Every user prompt is admitted as a `SessionInput` row before execution begins.
//! This enables:
//! - Duplicate detection (same id → skip)
//! - Promotion tracking (admitted → promoted into message history)
//! - Delivery mode (steer = inline, queue = FIFO buffer)

use serde::{Deserialize, Serialize};

/// Delivery mode for a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Delivery {
    /// Immediate — injected into the active turn.
    #[default]
    Steer,
    /// Queued — processed after the active turn settles.
    Queue,
}


/// An admitted (durable) session input row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInput {
    pub id: String,
    pub session_id: String,
    pub text: String,
    pub delivery: Delivery,
    pub admitted_seq: u64,
    pub promoted_seq: Option<u64>,
    pub time_created: String,
}

impl SessionInput {
    pub fn new(id: &str, session_id: &str, text: &str, delivery: Delivery) -> Self {
        Self {
            id: id.to_string(),
            session_id: session_id.to_string(),
            text: text.to_string(),
            delivery,
            admitted_seq: 0,
            promoted_seq: None,
            time_created: now_string(),
        }
    }
}

pub struct SessionInputStore {
    tree: sled::Tree,
}

impl SessionInputStore {
    pub fn open(db: &sled::Db) -> anyhow::Result<Self> {
        Ok(Self {
            tree: db.open_tree("session_inputs")?,
        })
    }

    /// Admit a prompt. Returns the existing input if already admitted.
    pub fn admit(&self, input: &SessionInput) -> anyhow::Result<SessionInput> {
        if let Some(existing) = self.find(&input.id)? {
            return Ok(existing);
        }
        let seq = self.next_seq(&input.session_id)? + 1;
        let mut admitted = input.clone();
        admitted.admitted_seq = seq;
        self.save(&admitted)?;
        Ok(admitted)
    }

    /// Promote an admitted input into message history (mark as consumed).
    pub fn promote(&self, id: &str, promoted_seq: u64) -> anyhow::Result<Option<SessionInput>> {
        let Some(mut input) = self.find(id)? else {
            return Ok(None);
        };
        if input.promoted_seq.is_some() {
            return Ok(Some(input));
        }
        input.promoted_seq = Some(promoted_seq);
        self.save(&input)?;
        Ok(Some(input))
    }

    pub fn find(&self, id: &str) -> anyhow::Result<Option<SessionInput>> {
        let Some(value) = self.tree.get(id.as_bytes())? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice(&value)?))
    }

    pub fn next_seq(&self, session_id: &str) -> anyhow::Result<u64> {
        Ok(self
            .tree
            .iter()
            .filter_map(|e| e.ok())
            .filter_map(|(_, v)| serde_json::from_slice::<SessionInput>(v.as_ref()).ok())
            .filter(|i| i.session_id == session_id)
            .map(|i| i.admitted_seq)
            .max()
            .unwrap_or(0))
    }

    /// List all unpromoted inputs for a session, ordered by admission sequence.
    pub fn pending(&self, session_id: &str) -> anyhow::Result<Vec<SessionInput>> {
        let mut pending: Vec<SessionInput> = self
            .tree
            .iter()
            .filter_map(|e| e.ok())
            .filter_map(|(_, v)| serde_json::from_slice::<SessionInput>(v.as_ref()).ok())
            .filter(|i| i.session_id == session_id && i.promoted_seq.is_none())
            .collect();
        pending.sort_by_key(|i| i.admitted_seq);
        Ok(pending)
    }

    fn save(&self, input: &SessionInput) -> anyhow::Result<()> {
        self.tree.insert(
            input.id.as_bytes(),
            serde_json::to_vec(input)?,
        )?;
        Ok(())
    }
}

fn now_string() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_store() -> (sled::Db, SessionInputStore) {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let store = SessionInputStore::open(&db).unwrap();
        (db, store)
    }

    #[test]
    fn admit_creates_input() {
        let (_db, store) = open_store();
        let input = SessionInput::new("input-1", "session-1", "hello", Delivery::Steer);
        let admitted = store.admit(&input).unwrap();
        assert_eq!(admitted.id, "input-1");
        assert!(admitted.admitted_seq > 0);
    }

    #[test]
    fn admit_is_idempotent() {
        let (_db, store) = open_store();
        let input = SessionInput::new("input-1", "session-1", "hello", Delivery::Steer);
        let a = store.admit(&input).unwrap();
        let b = store.admit(&input).unwrap();
        assert_eq!(a.admitted_seq, b.admitted_seq);
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn promote_marks_as_consumed() {
        let (_db, store) = open_store();
        let input = SessionInput::new("input-1", "session-1", "hello", Delivery::Steer);
        store.admit(&input).unwrap();
        let promoted = store.promote("input-1", 42).unwrap().unwrap();
        assert_eq!(promoted.promoted_seq, Some(42));
    }

    #[test]
    fn pending_lists_unpromoted() {
        let (_db, store) = open_store();
        let a = SessionInput::new("a", "session-1", "first", Delivery::Steer);
        let b = SessionInput::new("b", "session-1", "second", Delivery::Queue);
        store.admit(&a).unwrap();
        store.admit(&b).unwrap();
        store.promote("a", 1).unwrap();

        let pending = store.pending("session-1").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "b");
    }

    #[test]
    fn delivery_modes_are_distinct() {
        let input = SessionInput::new("q", "s", "text", Delivery::Queue);
        assert_eq!(input.delivery, Delivery::Queue);
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("queue"));
    }
}
