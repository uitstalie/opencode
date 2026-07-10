//! Session storage and message history for Phase 1B.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// A stored chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
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
    #[serde(default)]
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub agent: Option<String>,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub agent: Option<String>,
    pub message_count: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SessionTranscript {
    pub prefix: Vec<Message>,
    pub history: Vec<Message>,
}

#[derive(Debug, Clone)]
pub struct TaskSummary {
    pub id: String,
    pub agent: Option<String>,
    pub title: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct SessionStore {
    db: sled::Db,
    next_seq: Arc<AtomicU64>,
}

impl SessionStore {
    pub fn open() -> anyhow::Result<Self> {
        let db = sled::open(Self::db_path())?;
        let next_seq = Arc::new(AtomicU64::new(now_micros() as u64));
        Ok(Self { db, next_seq })
    }

    pub fn open_at(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let db = sled::open(path)?;
        let next_seq = Arc::new(AtomicU64::new(now_micros() as u64));
        Ok(Self { db, next_seq })
    }

    pub fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>> {
        let sessions_tree = self.db.open_tree("sessions")?;
        let messages_tree = self.db.open_tree("messages")?;

        // Build message-count per session in a single pass (avoids N+1 scans).
        let mut msg_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for entry in messages_tree.iter() {
            match entry {
                Ok((key, _)) => {
                    let session_id = std::str::from_utf8(&key)
                        .ok()
                        .and_then(|k| k.split(':').next())
                        .unwrap_or("");
                    if !session_id.is_empty() {
                        *msg_counts.entry(session_id.to_string()).or_insert(0) += 1;
                    }
                }
                Err(e) => tracing::warn!(error = %e, "sled iteration error in messages tree"),
            }
        }

        let mut sessions = sessions_tree
            .iter()
            .filter_map(|entry| match entry {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(error = %e, "sled iteration error in sessions tree");
                    None
                }
            })
            .filter_map(|(_, value)| match serde_json::from_slice::<Session>(value.as_ref()) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to deserialize session");
                    None
                }
            })
            .map(|session| SessionSummary {
                message_count: msg_counts.get(&session.id).copied().unwrap_or(0),
                id: session.id,
                title: session.title,
                summary: session.summary,
                agent: session.agent,
                created_at: session.created_at,
                updated_at: session.updated_at,
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
            .filter_map(|entry| match entry {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(error = %e, "sled iteration error in get_messages");
                    None
                }
            })
            .filter_map(|(_, value)| match serde_json::from_slice::<Message>(value.as_ref()) {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to deserialize message");
                    None
                }
            })
            .collect::<Vec<_>>();

        messages.sort_by(|a, b| {
            a.seq
                .cmp(&b.seq)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        Ok(messages)
    }

    pub fn effective_messages(&self, session_id: &str) -> anyhow::Result<Vec<Message>> {
        let messages = self.get_messages(session_id)?;
        let Some(index) = messages
            .iter()
            .rposition(|message| message.summary.is_some())
        else {
            return Ok(messages);
        };
        Ok(messages[index..].to_vec())
    }

    pub fn transcript(&self, session_id: &str) -> anyhow::Result<SessionTranscript> {
        let history = self.get_messages(session_id)?;
        let prefix = history
            .iter()
            .take_while(|message| message.role != "user")
            .cloned()
            .collect();
        Ok(SessionTranscript { prefix, history })
    }

    pub fn save_session(&self, session: &Session) -> anyhow::Result<()> {
        self.db
            .open_tree("sessions")?
            .insert(session.id.as_bytes(), serde_json::to_vec(session)?)?;
        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        let prefix = format!("{session_id}:");

        let sessions = self.db.open_tree("sessions")?;
        sessions.remove(session_id.as_bytes())?;

        // Batch-remove all messages for this session atomically.
        let messages = self.db.open_tree("messages")?;
        let mut msg_batch = sled::Batch::default();
        for key in messages.scan_prefix(prefix.as_bytes()).keys() {
            msg_batch.remove(key?);
        }
        messages.apply_batch(msg_batch)?;

        // Batch-remove all tasks for this session atomically.
        let tasks = self.db.open_tree("tasks")?;
        let mut task_batch = sled::Batch::default();
        for key in tasks.scan_prefix(prefix.as_bytes()).keys() {
            task_batch.remove(key?);
        }
        tasks.apply_batch(task_batch)?;

        Ok(())
    }

    pub fn cleanup_old_sessions(&self, max_age_secs: u64) -> anyhow::Result<usize> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(max_age_secs);

        let old: Vec<String> = self
            .list_sessions()?
            .into_iter()
            .filter(|s| {
                s.updated_at
                    .parse::<f64>()
                    .map(|ts| (ts as u64) < cutoff)
                    .unwrap_or(false)
            })
            .map(|s| s.id)
            .collect();

        let count = old.len();
        let mut deleted = 0usize;
        for id in &old {
            if self.delete_session(id).is_ok() {
                deleted += 1;
            }
        }
        if deleted < count {
            tracing::warn!(total = count, deleted, "some old sessions failed to delete");
        }
        Ok(deleted)
    }

    pub fn save_message(&self, message: &Message) -> anyhow::Result<()> {
        self.db.open_tree("messages")?.insert(
            self.message_key(&message.session_id, &message.id),
            serde_json::to_vec(message)?,
        )?;
        Ok(())
    }

    pub fn replace_messages(&self, session_id: &str, messages: &[Message]) -> anyhow::Result<()> {
        let tree = self.db.open_tree("messages")?;
        let mut batch = sled::Batch::default();
        for key in tree
            .scan_prefix(format!("{session_id}:").as_bytes())
            .keys()
            .flatten()
        {
            batch.remove(key);
        }
        for message in messages {
            batch.insert(
                self.message_key(&message.session_id, &message.id).as_bytes(),
                serde_json::to_vec(message)?,
            );
        }
        tree.apply_batch(batch)?;
        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn ensure_session(&self, id: &str) -> anyhow::Result<Session> {
        if let Some(session) = self.get_session(id)? {
            return Ok(session);
        }

        let now = now_string();
        let session = Session {
            id: id.to_string(),
            title: None,
            summary: None,
            agent: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.save_session(&session)?;
        Ok(session)
    }

    pub fn append_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> anyhow::Result<Message> {
        self.append_message_detail(session_id, role, content, None, None, None)
    }

    pub fn append_message_detail(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        name: Option<String>,
        tool_call_id: Option<String>,
        tool_calls: Option<serde_json::Value>,
    ) -> anyhow::Result<Message> {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let message = Message {
            id: format!("msg-{seq}"),
            session_id: session_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            seq,
            name,
            tool_call_id,
            tool_calls,
            summary: None,
            recent: None,
            created_at: now_string(),
        };
        self.save_message(&message)?;
        self.touch_session(session_id)?;
        Ok(message)
    }

    pub fn append_compaction(
        &self,
        session_id: &str,
        summary: String,
        recent: String,
    ) -> anyhow::Result<Message> {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let message = Message {
            id: format!("compact-{seq}"),
            session_id: session_id.to_string(),
            role: "system".to_string(),
            content: summary.clone(),
            seq,
            name: None,
            tool_call_id: None,
            tool_calls: None,
            summary: Some(summary),
            recent: Some(recent),
            created_at: now_string(),
        };
        self.save_message(&message)?;
        self.touch_session(session_id)?;
        Ok(message)
    }

    pub fn set_session_agent(&self, session_id: &str, agent: Option<String>) -> anyhow::Result<()> {
        self.update_session(session_id, |s| {
            s.agent = agent.clone();
            s.updated_at = now_string();
        })?;
        Ok(())
    }

    pub fn get_session_agent(&self, session_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .get_session(session_id)?
            .and_then(|session| session.agent))
    }

    pub fn set_title(&self, session_id: &str, title: String) -> anyhow::Result<()> {
        self.update_session(session_id, |s| {
            s.title = Some(title.clone());
            s.updated_at = now_string();
        })?;
        Ok(())
    }

    pub fn set_summary(&self, session_id: &str, summary: String) -> anyhow::Result<()> {
        self.update_session(session_id, |s| {
            s.summary = Some(summary.clone());
            s.updated_at = now_string();
        })?;
        Ok(())
    }

    pub fn list_tasks(&self, session_id: &str) -> anyhow::Result<Vec<TaskSummary>> {
        let mut tasks = self
            .db
            .open_tree("tasks")?
            .scan_prefix(format!("{session_id}:").as_bytes())
            .filter_map(|entry| match entry {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(error = %e, "sled iteration error in list_tasks");
                    None
                }
            })
            .filter_map(|(_, value)| match serde_json::from_slice::<Task>(value.as_ref()) {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to deserialize task");
                    None
                }
            })
            .map(|task| TaskSummary {
                id: task.id,
                agent: task.agent,
                title: task.title,
                status: task.status,
                created_at: task.created_at,
                updated_at: task.updated_at,
            })
            .collect::<Vec<_>>();

        tasks.sort_by(|a, b| a.updated_at.cmp(&b.updated_at));
        Ok(tasks)
    }

    pub fn save_task(&self, session_id: &str, task: &Task) -> anyhow::Result<()> {
        self.db.open_tree("tasks")?.insert(
            self.task_key(session_id, &task.id),
            serde_json::to_vec(task)?,
        )?;
        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn upsert_task(
        &self,
        session_id: &str,
        id: &str,
        agent: Option<String>,
        title: String,
        status: String,
        priority: Option<String>,
    ) -> anyhow::Result<Task> {
        let now = now_string();
        let task = Task {
            id: id.to_string(),
            agent,
            title,
            status,
            priority,
            created_at: now.clone(),
            updated_at: now,
        };
        self.save_task(session_id, &task)?;
        Ok(task)
    }

    pub fn replace_tasks(&self, session_id: &str, tasks: &[Task]) -> anyhow::Result<()> {
        let tree = self.db.open_tree("tasks")?;
        let mut batch = sled::Batch::default();
        for key in tree
            .scan_prefix(format!("{session_id}:").as_bytes())
            .keys()
            .flatten()
        {
            batch.remove(key);
        }
        for task in tasks {
            batch.insert(
                self.task_key(session_id, &task.id).as_bytes(),
                serde_json::to_vec(task)?,
            );
        }
        tree.apply_batch(batch)?;
        self.touch_session(session_id)?;
        Ok(())
    }

    pub fn update_task_status(
        &self,
        session_id: &str,
        id: &str,
        status: &str,
    ) -> anyhow::Result<Option<Task>> {
        let key = self.task_key(session_id, id);
        let tree = self.db.open_tree("tasks")?;
        let Some(value) = tree.get(key.as_bytes())? else {
            return Ok(None);
        };
        let mut task: Task = serde_json::from_slice(&value)?;
        task.status = status.to_string();
        task.updated_at = now_string();
        tree.insert(key.as_bytes(), serde_json::to_vec(&task)?)?;
        self.touch_session(session_id)?;
        Ok(Some(task))
    }

    pub fn task_count(&self, session_id: &str) -> anyhow::Result<usize> {
        Ok(self
            .db
            .open_tree("tasks")?
            .scan_prefix(format!("{session_id}:").as_bytes())
            .count())
    }

    /// Atomically update a Session record using CAS retries to avoid
    /// lost-update races between concurrent writers (e.g. background
    /// title/summary threads vs. main-thread touch_session).
    fn update_session<F>(&self, session_id: &str, f: F) -> anyhow::Result<Option<Session>>
    where
        F: Fn(&mut Session),
    {
        let tree = self.db.open_tree("sessions")?;
        let key = session_id.as_bytes();
        for _ in 0..16 {
            let old = tree.get(key)?;
            let Some(old_value) = old else {
                return Ok(None);
            };
            let mut session: Session = serde_json::from_slice(&old_value)?;
            f(&mut session);
            let new_bytes = serde_json::to_vec(&session)?;
            match tree.compare_and_swap(key, Some(old_value), Some(new_bytes))? {
                Ok(()) => return Ok(Some(session)),
                Err(_) => continue,
            }
        }
        tracing::warn!(session_id, "session CAS retries exhausted, falling back to blind write");
        let old = tree.get(key)?;
        let Some(old_value) = old else {
            return Ok(None);
        };
        let mut session: Session = serde_json::from_slice(&old_value)?;
        f(&mut session);
        self.save_session(&session)?;
        Ok(Some(session))
    }

    fn touch_session(&self, session_id: &str) -> anyhow::Result<()> {
        self.update_session(session_id, |s| {
            s.updated_at = now_string();
        })?;
        Ok(())
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.db.flush()?;
        Ok(())
    }

    fn message_key(&self, session_id: &str, message_id: &str) -> String {
        format!("{session_id}:{message_id}")
    }

    fn task_key(&self, session_id: &str, task_id: &str) -> String {
        format!("{session_id}:{task_id}")
    }

    pub fn delete_task(&self, session_id: &str, id: &str) -> anyhow::Result<bool> {
        let tree = self.db.open_tree("tasks")?;
        let key = self.task_key(session_id, id);
        let existed = tree.get(key.as_bytes())?.is_some();
        if existed {
            tree.remove(key.as_bytes())?;
            self.touch_session(session_id)?;
        }
        Ok(existed)
    }

    fn db_path() -> std::path::PathBuf {
        crate::core::platform::PlatformPaths::detect().sessions_db_path()
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
            store
                .ensure_session("session-1")
                .unwrap();
            store.append_message("session-1", "user", "hello").unwrap();
            store
                .append_message("session-1", "assistant", "world")
                .unwrap();
            drop(store);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));

        let store = SessionStore::open_at(dir.path()).unwrap();
        let sessions = store.list_sessions().unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "session-1");
        assert_eq!(sessions[0].message_count, 2);

        let messages = store.get_messages("session-1").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "world");
    }

    #[test]
    fn session_store_persists_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open_at(dir.path()).unwrap();
        store.ensure_session("session-4").unwrap();
        store
            .upsert_task(
                "session-4",
                "task-1",
                Some("build".to_string()),
                "Implement flow".to_string(),
                "pending".to_string(),
                None,
            )
            .unwrap();
        drop(store);
        std::thread::sleep(std::time::Duration::from_millis(25));

        let store = SessionStore::open_at(dir.path()).unwrap();
        let tasks = store.list_tasks("session-4").unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "task-1");
        assert_eq!(tasks[0].agent.as_deref(), Some("build"));
        assert_eq!(tasks[0].status, "pending");
    }

    #[test]
    fn session_store_persists_agent_selection() {
        let dir = tempfile::tempdir().unwrap();

        {
            let store = SessionStore::open_at(dir.path()).unwrap();
            store
                .ensure_session("session-5")
                .unwrap();
            store
                .set_session_agent("session-5", Some("review".to_string()))
                .unwrap();
            drop(store);
        }

        std::thread::sleep(std::time::Duration::from_millis(25));

        let store = SessionStore::open_at(dir.path()).unwrap();
        assert_eq!(
            store.get_session_agent("session-5").unwrap().as_deref(),
            Some("review")
        );
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions[0].agent.as_deref(), Some("review"));
    }

    #[test]
    fn replace_messages_overwrites_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open_at(dir.path()).unwrap();
        store.ensure_session("session-6").unwrap();
        store.append_message("session-6", "user", "one").unwrap();
        store
            .append_message("session-6", "assistant", "two")
            .unwrap();

        let replacement = [Message {
            id: "summary-1".to_string(),
            session_id: "session-6".to_string(),
            role: "system".to_string(),
            content: "summary".to_string(),
            seq: 0,
            name: None,
            tool_call_id: None,
            tool_calls: None,
            summary: None,
            recent: None,
            created_at: "3".to_string(),
        }];
        store.replace_messages("session-6", &replacement).unwrap();

        let messages = store.get_messages("session-6").unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content, "summary");
    }

    #[test]
    fn effective_messages_follow_latest_compaction_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open_at(dir.path()).unwrap();
        store.ensure_session("session-7").unwrap();
        store
            .append_message("session-7", "user", "old one")
            .unwrap();
        store
            .append_message("session-7", "assistant", "old two")
            .unwrap();
        store
            .append_compaction(
                "session-7",
                "checkpoint".to_string(),
                "old one\nold two".to_string(),
            )
            .unwrap();
        store
            .append_message("session-7", "user", "new one")
            .unwrap();

        let messages = store.effective_messages("session-7").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].summary.as_deref(), Some("checkpoint"));
        assert_eq!(messages[1].content, "new one");
    }

    #[test]
    fn open_at_reuses_existing_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open_at(dir.path()).unwrap();
        store.ensure_session("session-2").unwrap();

        let session = store
            .ensure_session("session-2")
            .unwrap();
        assert_eq!(session.id, "session-2");
    }

    #[test]
    fn transcript_splits_fixed_prefix_and_dynamic_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open_at(dir.path()).unwrap();

        store
            .ensure_session("session-3")
            .unwrap();
        store
            .append_message("session-3", "system", "stable prefix")
            .unwrap();
        store
            .append_message("session-3", "assistant", "more prefix")
            .unwrap();
        store
            .append_message("session-3", "user", "first prompt")
            .unwrap();
        store
            .append_message("session-3", "assistant", "first reply")
            .unwrap();

        let transcript = store.transcript("session-3").unwrap();

        assert_eq!(transcript.prefix.len(), 2);
        assert_eq!(transcript.prefix[0].content, "stable prefix");
        assert_eq!(transcript.prefix[1].content, "more prefix");
        assert_eq!(transcript.history.len(), 4);
        assert_eq!(transcript.history[2].content, "first prompt");
    }
}


