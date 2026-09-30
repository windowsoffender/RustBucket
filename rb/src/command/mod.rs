use crate::listener::http_listener::HttpListener;
use crate::listener::*;
use crate::message::{CommandError, CommandRequest, CommandResult};
use crate::pki::PkiAuthority;
use crate::store::Store;
use std::any::Any;
use std::collections::HashMap;
use std::result::Result;
use std::sync::Arc;
use std::sync::Mutex;
use uuid::Uuid;

use crate::task::TaskStatus;

use clap;

pub mod implant_cmds;
mod server_cmds;

// Define command types
pub enum CommandType {
    Server,  // Commands that control the server
    Implant, // Commands sent to implants
}

// Base Command trait that both types will implement
pub trait RbCommand: Send + Sync {
    fn name(&self) -> &'static str;
    fn command_type(&self) -> CommandType;
    fn description(&self) -> &'static str;
    fn clap_command(&self) -> clap::Command {
        let name = self.name();
        let description = self.description();

        clap::Command::new(name)
            .about(description)
            .arg_required_else_help(true)
    }
    fn parse_args(&self, command_line: &str) -> Result<Box<dyn Any>, clap::Error>;
    fn execute_with_parsed_args(
        &self,
        context: &mut CommandContext,
        args: Box<dyn Any>,
    ) -> CommandResult;
}

// Context passed to commands (can contain server state, active session, etc.)
pub struct CommandContext {
    pub store: Arc<dyn Store>,
    pub pki: Arc<dyn PkiAuthority>,
    pub server_host: String,
    pub server_port: u16,
    // pub active_session: Option<Arc<Session>>,
    pub command_registry: Arc<CommandRegistry>,
    // pub listeners: Arc<Mutex<HashMap<Uuid, Arc<Mutex<Box<dyn Listener>>>>>>, // Should switch to a generic listener type like this later
    pub listeners: Arc<Mutex<HashMap<Uuid, Arc<Mutex<Box<HttpListener>>>>>>, // For now, only
                                                                             // HTTP listeners
}

// Command Registry for server-side commands. Implant command metadata lives in
// `implant_cmds`.
pub struct CommandRegistry {
    server_commands: HashMap<String, Box<dyn RbCommand>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut registry = CommandRegistry {
            server_commands: HashMap::new(),
        };

        // Register built-in server commands
        registry.register(Box::new(server_cmds::ServerListenersCommand {}));
        registry.register(Box::new(server_cmds::ServerSessionsCommand {}));
        registry.register(Box::new(server_cmds::ServerHelpCommand {}));
        registry.register(Box::new(server_cmds::PayloadCommand {}));
        registry.register(Box::new(server_cmds::ServerOperatorCommand {}));

        registry
    }

    pub fn register(&mut self, command: Box<dyn RbCommand>) {
        self.server_commands
            .insert(command.name().to_string(), command);
    }

    pub fn get_server_command(&self, name: &str) -> Option<&Box<dyn RbCommand>> {
        self.server_commands.get(name)
    }

    pub fn list_server_commands(&self) -> Vec<&str> {
        self.server_commands.keys().map(|k| k.as_str()).collect()
    }

    // Execute a command with proper routing
    pub async fn execute(
        &self,
        context: &mut CommandContext,
        command_request: CommandRequest,
    ) -> CommandResult {
        let CommandRequest {
            command_line,
            session_id,
            data,
        } = command_request;

        // Parse the command
        let parts: Vec<&str> = command_line.split_whitespace().collect();
        if parts.is_empty() {
            return Err(CommandError::InvalidArguments(
                "No command specified".into(),
            ));
        }

        let command_name = parts[0];

        // Check if we have a session_id to determine command type
        if let Some(session_id) = session_id {
            // Execute implant command on a specific session
            // Verify the session exists
            let session_exists = context.store.get_session(&session_id).is_some();

            if !session_exists {
                return Err(CommandError::TargetNotFound(format!(
                    "Session with ID '{}' not found",
                    session_id
                )));
            }

            return self
                .execute_implant_command(context, &command_line, session_id, data)
                .await;
        } else {
            // No session_id, so it's a server command
            if let Some(command) = self.get_server_command(command_name) {
                return self
                    .execute_server_command(command, context, &command_line)
                    .await;
            } else {
                return Err(CommandError::TargetNotFound(format!(
                    "Server command '{}' not found",
                    command_name
                )));
            }
        }
    }

    async fn execute_server_command(
        &self,
        command: &Box<dyn RbCommand>,
        context: &mut CommandContext,
        command_line: &str,
    ) -> CommandResult {
        // Parse arguments with clap
        let args_result = command.parse_args(command_line);

        match args_result {
            Ok(parsed_args) => {
                // Execute command with the parsed arguments
                command.execute_with_parsed_args(context, parsed_args)
            }
            Err(e) => Err(CommandError::InvalidArguments(format!(
                "Failed to parse arguments\n\n{}",
                e,
            ))),
        }
    }

    async fn execute_implant_command(
        &self,
        context: &mut CommandContext,
        command_line: &str,
        session_id: usize,
        data: Option<Vec<u8>>,
    ) -> CommandResult {
        // Make sure the session exists before creating a task for it.
        if context.store.get_session(&session_id).is_none() {
            return Err(CommandError::TargetNotFound(format!(
                "Session with ID '{}' not found",
                session_id
            )));
        }

        let command_name = command_line.split_whitespace().next().unwrap_or("");

        // Only known implant commands become tasks.
        if implant_cmds::find(command_name).is_none() {
            let supported = implant_cmds::IMPLANT_COMMANDS
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(CommandError::InvalidArguments(format!(
                "Unknown implant command '{}'. Supported commands: {}",
                command_name, supported
            )));
        }

        let args = command_line
            .split_whitespace()
            .skip(1) // Skip the command name
            .map(|s| s.to_string())
            .collect::<Vec<String>>();

        log::debug!("Command name: {}, args: {:?}", command_name, args);

        let task_id = match context.store.create_task(session_id, command_name.to_string(), args, data) {
            Ok(id) => id,
            Err(err) => {
                return Err(CommandError::Internal(format!(
                    "Failed to create task: {}",
                    err
                )))
            }
        };

        let timeout = std::time::Duration::from_secs(10);
        let start_time = std::time::Instant::now();

        loop {
            match context.store.get_task(&task_id) {
                Some(task) if task.status == TaskStatus::Completed => break,
                Some(task)
                    if task.status == TaskStatus::Failed
                        || task.status == TaskStatus::Cancelled =>
                {
                    return Err(CommandError::ExecutionFailed(format!(
                        "Task {} ended with status {}",
                        task_id,
                        task.status.to_string()
                    )));
                }
                Some(_) => {}
                None => {
                    return Err(CommandError::Internal(format!(
                        "Task {} disappeared",
                        task_id
                    )))
                }
            }

            if start_time.elapsed() > timeout {
                return Err(CommandError::Timeout(format!(
                    "Task with ID '{}' timed out",
                    task_id
                )));
            }

            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        match context.store.get_result(&task_id) {
            Some(result) => Ok(result.output),
            None => Err(CommandError::Internal(format!(
                "Task {} completed without a result",
                task_id
            ))),
        }
    }
}
