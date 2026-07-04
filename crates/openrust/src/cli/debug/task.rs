use clap::Subcommand;

use crate::core::session::SessionStore;

#[derive(Subcommand)]
pub enum Cmd {
    /// List tasks for a session
    List {
        /// Session id
        #[arg(long)]
        session: String,
    },
    /// Mark a task completed
    Done {
        /// Session id
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
        Cmd::Done { session, id } => {
            match store.update_task_status(&session, &id, "completed")? {
                Some(task) => {
                    println!("Task completed: {} [{}]", task.id, task.title);
                    Ok(())
                }
                None => anyhow::bail!("Task '{}' not found in session '{}'", id, session),
            }
        }
    }
}

fn print_list(session: &str, tasks: &[crate::core::session::TaskSummary]) {
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
