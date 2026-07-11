use clap::Subcommand;

use crate::core::session::SessionStore;

#[derive(Subcommand)]
pub enum Cmd {
    /// List saved sessions
    List,
    /// Show a session's message history
    Show {
        /// Session id
        id: String,
    },
    /// Delete a session and all its messages/tasks
    Delete {
        /// Session id
        id: String,
    },
    /// Delete all sessions (irreversible)
    DeleteAll,
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let store = SessionStore::open()?;

    match cmd {
        Cmd::List => {
            let sessions = store.list_sessions()?;
            print_list(&sessions);
            Ok(())
        }
        Cmd::Show { id } => {
            let Some(session) = store.get_session(&id)? else {
                anyhow::bail!("Session '{}' not found", id);
            };

            println!("Session: {}", session.id);
            println!(
                "Title:   {}",
                session.title.as_deref().unwrap_or("(untitled)")
            );
            println!("Created: {}", session.created_at);
            println!("Updated: {}", session.updated_at);
            println!();

            let messages = store.effective_messages(&id)?;
            if messages.is_empty() {
                println!("No messages stored.");
                return Ok(());
            }

            for message in messages {
                println!("[{}] {}: {}", message.id, message.role, message.content);
            }
            Ok(())
        }
        Cmd::Delete { id } => {
            store.delete_session(&id)?;
            println!("Deleted session {}.", id);
            Ok(())
        }
        Cmd::DeleteAll => {
            let sessions = store.list_sessions()?;
            let count = sessions.len();
            for session in &sessions {
                store.delete_session(&session.id)?;
            }
            println!("Deleted {} session(s).", count);
            Ok(())
        }
    }
}

fn print_list(sessions: &[crate::core::session::SessionSummary]) {
    if sessions.is_empty() {
        println!("No saved sessions.");
        return;
    }

    for session in sessions {
        println!(
            "{}  messages={}  agent={}  title={}  updated={}",
            session.id,
            session.message_count,
            session.agent.as_deref().unwrap_or("(default)"),
            session.title.as_deref().unwrap_or("(untitled)"),
            session.updated_at,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_format_handles_empty_and_entries() {
        print_list(&[]);
        let sessions = [crate::core::session::SessionSummary {
            id: "session-1".to_string(),
            title: Some("Build".to_string()),
            summary: None,
            agent: Some("review".to_string()),
            message_count: 2,
            created_at: "1".to_string(),
            updated_at: "2".to_string(),
        }];
        print_list(&sessions);
    }
}
