//! Session storage and message history for Phase 1B.

use serde::{Deserialize, Serialize};

/// A stored chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: Option<String>,
    pub mode: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A single message in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub title: Option<String>,
    pub mode: Option<String>,
    pub message_count: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SessionTranscript {
    pub prefix: Vec<Message>,
    pub history: Vec<Message>,
}

#[derive(Clone)]
pub struct SessionStore {
    db: sled::Db,
}

impl SessionStore {
    pub fn open() -> anyhow::Result<Self> {
        let db = sled::open(Self::db_path())?;
        Ok(Self { db })
    }

    pub fn open_at(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let db = sled::open(path)?;
        Ok(Self { db })
    }

    pub fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>> {
        let mut sessions = self
            .db
            .open_tree("sessions")?
            .iter()
            .filter_map(|entry| entry.ok())
            .filter_map(|(_, value)| serde_json::from_slice::<Session>(value.as_ref()).ok())
            .map(|session| {
                let message_count = self.message_count(&session.id).unwrap_or(0);
                SessionSummary {
                    id: session.id,
                    title: session.title,
                    mode: session.mode,
                    message_count,
                    created_at: session.created_at,
                    updated_at: session.updated_at,
                }
            })
            .collect::<Vec<_>>();

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    pub fn get_session(&self, id: &str) -> anyhow::Result<Option<Session>> {
        let tree = self.db.open_tree("sessions")?;
        let Some(value) = tree.get(id.as_bytes())? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice(&value)?))
    }

    pub fn get_messages(&self, session_id: &str) -> anyhow::Result<Vec<Message>> {
        let mut messages = self
            .db
            .open_tree("messages")?
            .scan_prefix(format!("{session_id}:").as_bytes())
            .filter_map(|entry| entry.ok())
            .filter_map(|(_, value)| serde_json::from_slice::<Message>(value.as_ref()).ok())
            .collect::<Vec<_>>();

        messages.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(messages)
    }

    pub fn transcript(&self, session_id: &str) -> anyhow::Result<SessionTranscript> {
        let history = self.get_messages(session_id)?;
        let prefix = history.iter().take_while(|message| message.role != "user").cloned().collect();
        Ok(SessionTranscript { prefix, history })
    }

    pub fn save_session(&self, session: &Session) -> anyhow::Result<()> {
        self.db
            .open_tree("sessions")?
            .insert(session.id.as_bytes(), serde_json::to_vec(session)?)?;
        Ok(())
    }

    pub fn save_message(&self, message: &Message) -> anyhow::Result<()> {
        self.db
            .open_tree("messages")?
            .insert(self.message_key(&message.session_id, &message.id), serde_json::to_vec(message)?)?;
        Ok(())
    }

    pub fn ensure_session(&self, id: &str, mode: Option<String>) -> anyhow::Result<Session> {
        if let Some(session) = self.get_session(id)? {
            return Ok(session);
        }

        let now = now_string();
        let session = Session {
            id: id.to_string(),
            title: None,
            mode,
            created_at: now.clone(),
            updated_at: now,
        };
        self.save_session(&session)?;
        Ok(session)
    }

    pub fn append_message(&self, session_id: &str, role: &str, content: &str) -> anyhow::Result<Message> {
        let message = Message {
            id: format!("msg-{}", now_micros()),
            session_id: session_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            created_at: now_string(),
        };
        self.save_message(&message)?;
        self.touch_session(session_id)?;
        Ok(message)
    }

    fn message_count(&self, session_id: &str) -> anyhow::Result<usize> {
        Ok(self
            .db
            .open_tree("messages")?
            .scan_prefix(format!("{session_id}:").as_bytes())
            .count())
    }

    fn touch_session(&self, session_id: &str) -> anyhow::Result<()> {
        let Some(mut session) = self.get_session(session_id)? else {
            return Ok(());
        };
        session.updated_at = now_string();
        self.save_session(&session)
    }

    fn message_key(&self, session_id: &str, message_id: &str) -> String {
        format!("{session_id}:{message_id}")
    }

    fn db_path() -> std::path::PathBuf {
        crate::core::paths::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".config")
            .join("openrust")
            .join("sessions.db")
    }
}

fn now_string() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:09}", now.as_secs(), now.subsec_nanos())
}

fn now_micros() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_store_persists_sessions_and_messages() {
        let dir = tempfile::tempdir().unwrap();

        {
            let store = SessionStore::open_at(dir.path()).unwrap();
            store.ensure_session("session-1", Some("build".to_string())).unwrap();
            store.append_message("session-1", "user", "hello").unwrap();
            store.append_message("session-1", "assistant", "world").unwrap();
        }

        let store = SessionStore::open_at(dir.path()).unwrap();
        let sessions = store.list_sessions().unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "session-1");
        assert_eq!(sessions[0].mode.as_deref(), Some("build"));
        assert_eq!(sessions[0].message_count, 2);

        let messages = store.get_messages("session-1").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "world");
    }

    #[test]
    fn open_at_reuses_existing_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open_at(dir.path()).unwrap();
        store.ensure_session("session-2", None).unwrap();

        let session = store.ensure_session("session-2", Some("plan".to_string())).unwrap();
        assert_eq!(session.id, "session-2");
        assert_eq!(session.mode, None);
    }

    #[test]
    fn transcript_splits_fixed_prefix_and_dynamic_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open_at(dir.path()).unwrap();

        store.ensure_session("session-3", Some("build".to_string())).unwrap();
        store.append_message("session-3", "system", "stable prefix").unwrap();
        store.append_message("session-3", "assistant", "more prefix").unwrap();
        store.append_message("session-3", "user", "first prompt").unwrap();
        store.append_message("session-3", "assistant", "first reply").unwrap();

        let transcript = store.transcript("session-3").unwrap();

        assert_eq!(transcript.prefix.len(), 2);
        assert_eq!(transcript.prefix[0].content, "stable prefix");
        assert_eq!(transcript.prefix[1].content, "more prefix");
        assert_eq!(transcript.history.len(), 4);
        assert_eq!(transcript.history[2].content, "first prompt");
    }
}
