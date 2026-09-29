use crate::command::*;
use crate::message::*;

use super::get_arg_matches;
use clap;

pub struct ServerSessionsCommand {}

#[derive(Debug)]
struct SessionsArgs {
    action: String,
    id: Option<String>,
}

impl RbCommand for ServerSessionsCommand {
    fn name(&self) -> &'static str {
        "sessions"
    }

    fn command_type(&self) -> CommandType {
        CommandType::Server
    }

    fn description(&self) -> &'static str {
        "Manage sessions"
    }

    fn parse_args(&self, command_line: &str) -> Result<Box<dyn Any>, clap::Error> {
        let cmd = clap::Command::new(self.name())
            .about(self.description().to_string())
            .subcommand(clap::Command::new("list").about("List all sessions"))
            .subcommand(
                clap::Command::new("kill").about("Kill a session").arg(
                    clap::Arg::new("id")
                        .help("The ID of the session to kill")
                        .required(true),
                ),
            )
            .subcommand(
                clap::Command::new("use").about("Attach to a session").arg(
                    clap::Arg::new("id")
                        .help("The ID of the session to attach to")
                        .required(true),
                ),
            )
            .arg_required_else_help(true);

        let matches = get_arg_matches(&cmd, command_line)?;

        let mut args = SessionsArgs {
            action: String::new(),
            id: None,
        };

        // Parse subcommands
        if let Some(_) = matches.subcommand_matches("list") {
            args.action = "list".to_string();
        } else if let Some(sub_matches) = matches.subcommand_matches("kill") {
            args.action = "kill".to_string();
            args.id = sub_matches.get_one::<String>("id").cloned();
        } else if let Some(sub_matches) = matches.subcommand_matches("use") {
            args.action = "use".to_string();
            args.id = sub_matches.get_one::<String>("id").cloned();
        }

        Ok(Box::new(args))
    }

    fn execute_with_parsed_args(
        &self,
        context: &mut CommandContext,
        args: Box<dyn Any>,
    ) -> CommandResult {
        match args.downcast::<SessionsArgs>() {
            Ok(args) => {
                log::debug!("Executing sessions command with args: {:?}", args);

                match args.action.as_str() {
                    "list" => {
                        // Get all sessions from the store
                        let sessions = context.store.list_sessions();

                        if sessions.is_empty() {
                            return Ok(CommandOutput::Text("No active sessions".to_string()));
                        }

                        // Format as a table
                        let headers = vec![
                            "ID".to_string(),
                            "Hostname".to_string(),
                            "Address".to_string(),
                            "Last Seen".to_string(),
                            "Status".to_string(),
                        ];

                        let mut rows = Vec::new();

                        for session in sessions.iter() {
                            // Access session properties directly as it's an Arc<Session>, not mutex protected
                            rows.push(vec![
                                session.id().to_string(),
                                session.implant_hostname().to_string(),
                                session.address().to_string(),
                                session.last_seen().to_string(),
                                session.status().to_string(),
                            ]);
                        }

                        Ok(CommandOutput::Table { headers, rows })
                    }
                    "kill" => {
                        // Validate required parameters
                        let id_str = match &args.id {
                            Some(id) => id,
                            None => {
                                return Err(CommandError::ExecutionFailed(
                                    "Session ID required".to_string(),
                                ))
                            }
                        };

                        // Parse id as usize
                        let id = match id_str.parse::<usize>() {
                            Ok(id) => id,
                            Err(e) => {
                                return Err(CommandError::ExecutionFailed(format!(
                                    "Invalid session ID format: {}",
                                    e,
                                )))
                            }
                        };

                        let session_manager = context.store.clone();

                        // TODO: Make `remove_session` return a Result for better error handling
                        match session_manager.remove_session(&id) {
                            true => Ok(CommandOutput::Text(format!(
                                "Session with ID {} killed",
                                id
                            ))),
                            false => Err(CommandError::ExecutionFailed(format!(
                                "Failed to kill session with ID {}",
                                id
                            ))),
                        }
                    }
                    // This code is never even called lmao it does nothing
                    "use" => {
                        // Validate required parameters
                        let id_str = match &args.id {
                            Some(id) => id,
                            None => {
                                return Err(CommandError::ExecutionFailed(
                                    "Session ID required".to_string(),
                                ))
                            }
                        };

                        // Parse id as usize
                        let id = match id_str.parse::<usize>() {
                            Ok(id) => id,
                            Err(e) => {
                                return Err(CommandError::ExecutionFailed(format!(
                                    "Invalid session ID format: {}",
                                    e,
                                )))
                            }
                        };

                        // Check if session exists
                        let session_manager = context.store.clone();
                        match session_manager.activate_session(&id) {
                            Ok(_) => {
                                return Ok(CommandOutput::Text(format!(
                                    "Session with ID {} activated",
                                    id
                                )))
                            }
                            Err(_) => {
                                return Err(CommandError::ExecutionFailed(format!(
                                    "Failed to activate session with ID {}",
                                    id
                                )))
                            }
                        }
                    }
                    _ => Err(CommandError::ExecutionFailed(format!(
                        "Unknown action: {}",
                        args.action
                    ))),
                }
            }
            Err(_) => Err(CommandError::ExecutionFailed(
                "Failed to process arguments".to_string(),
            )),
        }
    }
}
