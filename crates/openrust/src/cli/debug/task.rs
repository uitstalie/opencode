use clap::Subcommand;

use crate::core::session::{SessionStore, TaskSummary};

fn now_micros() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
}

#[derive(Subcommand)]
pub enum Cmd {
    /// List tasks for a session
    List {
        #[arg(long)]
        session: String,
    },
    /// Add a task to a session
    Add {
        #[arg(long)]
        session: String,
        /// Task title
        title: String,
        /// Initial status (default: pending)
        #[arg(long, default_value = "pending")]
        status: String,
    },
    /// Delete a task from a session
    Delete {
        #[arg(long)]
        session: String,
        /// Task id
        id: String,
    },
    /// Mark a task completed
    Done {
        #[arg(long)]
        session: String,
        /// Task id
        id: String,
    },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let store = SessionStore::open()?;

    match cmd {
        Cmd::List { session } => {
            print_list(&session, &store.list_tasks(&session)?);
            Ok(())
        }
        Cmd::Add {
            session,
            title,
            status,
        } => {
            store.ensure_session(&session)?;
            let id = format!("task-{}", now_micros());
            store.upsert_task(
                &session,
                &id,
                None,
                title.clone(),
                status,
                None,
            )?;
            println!("Task added: {}", title);
            Ok(())
        }
        Cmd::Delete { session, id } => match store.delete_task(&session, &id)? {
            true => {
                println!("Task deleted: {}", id);
                Ok(())
            }
            false => anyhow::bail!("Task '{}' not found in session '{}'", id, session),
        },
        Cmd::Done { session, id } => match store.update_task_status(&session, &id, "completed")? {
            Some(task) => {
                println!("Task completed: {} [{}]", task.id, task.title);
                Ok(())
            }
            None => anyhow::bail!("Task '{}' not found in session '{}'", id, session),
        },
    }
}

fn print_list(session: &str, tasks: &[TaskSummary]) {
    if tasks.is_empty() {
        println!("No tasks for session {}.", session);
        return;
    }

    for task in tasks {
        println!(
            "{}  [{}] {}  agent={}  updated={}",
            task.id,
            task.status,
            task.title,
            task.agent.as_deref().unwrap_or("(default)"),
            task.updated_at,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_format_handles_empty_and_entries() {
        print_list("session-1", &[]);
        let tasks = [crate::core::session::TaskSummary {
            id: "task-1".to_string(),
            agent: Some("build".to_string()),
            title: "Write tests".to_string(),
            status: "pending".to_string(),
            created_at: "1".to_string(),
            updated_at: "2".to_string(),
        }];
        print_list("session-1", &tasks);
    }
}
