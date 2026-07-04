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
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let store = SessionStore::open()?;

    match cmd {
        Cmd::List => {
            let sessions = store.list_sessions()?;
            if sessions.is_empty() {
                println!("No saved sessions.");
                return Ok(());
            }

            for session in sessions {
                println!(
                    "{}  messages={}  mode={}  title={}  updated={}",
                    session.id,
                    session.message_count,
                    session.mode.as_deref().unwrap_or("(unset)"),
                    session.title.as_deref().unwrap_or("(untitled)"),
                    session.updated_at,
                );
            }
            Ok(())
        }
        Cmd::Show { id } => {
            let Some(session) = store.get_session(&id)? else {
                anyhow::bail!("Session '{}' not found", id);
            };

            println!("Session: {}", session.id);
            println!("Title:   {}", session.title.as_deref().unwrap_or("(untitled)"));
            println!("Mode:    {}", session.mode.as_deref().unwrap_or("(unset)"));
            println!("Created: {}", session.created_at);
            println!("Updated: {}", session.updated_at);
            println!();

            let messages = store.get_messages(&id)?;
            if messages.is_empty() {
                println!("No messages stored.");
                return Ok(());
            }

            for message in messages {
                println!("[{}] {}: {}", message.id, message.role, message.content);
            }
            Ok(())
        }
    }
}
