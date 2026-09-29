use crate::command::*;
use crate::message::*;

use clap;

#[derive(Debug)]
struct HelpArgs {
    command: Option<String>,
}

pub struct ServerHelpCommand {}

impl RbCommand for ServerHelpCommand {
    fn name(&self) -> &'static str {
        "help"
    }

    fn command_type(&self) -> CommandType {
        CommandType::Server
    }

    fn description(&self) -> &'static str {
        "Displays help menu"
    }

    fn parse_args(&self, command_line: &str) -> Result<Box<dyn Any>, clap::Error> {
        // Create the command with a String, not a reference
        let cmd = clap::Command::new(self.name())
            .about(self.description().to_string())
            .arg(
                clap::Arg::new("command")
                    .help("Show help for a specific command")
                    .required(false),
            );

        // Get the arguments part (skip the command name)
        let args_str = command_line
            .trim_start()
            .strip_prefix(self.name())
            .unwrap_or("")
            .trim_start();

        // Parse the arguments with owned strings
        let matches = cmd.try_get_matches_from(
            vec![self.name().to_string()]
                .into_iter()
                .chain(args_str.split_whitespace().map(String::from)),
        )?;

        let args = HelpArgs {
            command: matches.get_one::<String>("command").map(|s| s.to_string()),
        };

        Ok(Box::new(args))
    }

    fn execute_with_parsed_args(
        &self,
        context: &mut CommandContext,
        args: Box<dyn Any>,
    ) -> CommandResult {
        // Downcast the args to the specific type
        let args = match args.downcast::<HelpArgs>() {
            Ok(args) => *args,
            Err(_) => return Err(CommandError::Internal("Invalid arguments type".into())),
        };

        // Handle specific command help
        if let Some(command_name) = args.command {
            // Try to find the command in server commands first
            if let Some(command) = context.command_registry.get_server_command(&command_name) {
                // Generate help for this specific command
                return self.generate_command_help(command);
            }

            // Implant commands live on the implant; the server knows their metadata.
            if let Some(command) = crate::command::implant_cmds::find(&command_name) {
                return Ok(CommandOutput::Text(format!(
                    "{}\n\nUsage: {}",
                    command.description, command.usage
                )));
            }

            return Err(CommandError::TargetNotFound(format!(
                "Command '{}' not found",
                command_name
            )));
        }

        // General help - list all commands
        let mut result = String::from("Available commands:\n\n");
        result.push_str("Server Commands:\n");

        // List server commands
        let server_commands = context.command_registry.list_server_commands();
        for cmd_name in server_commands {
            if let Some(cmd) = context.command_registry.get_server_command(cmd_name) {
                result.push_str(&format!("  {:15} - {}\n", cmd_name, cmd.description()));
            }
        }

        result.push_str("\nImplant Commands (require active session):\n");

        // List implant commands
        for command in crate::command::implant_cmds::IMPLANT_COMMANDS {
            result.push_str(&format!("  {:15} - {}\n", command.name, command.description));
        }

        result
            .push_str("\nUse 'help <command>' for detailed information about a specific command.");

        Ok(CommandOutput::Text(result))
    }
}

impl ServerHelpCommand {
    // Helper method to generate help for a specific command
    fn generate_command_help(&self, command: &Box<dyn RbCommand>) -> CommandResult {
        // We need to recreate the clap command to get its help
        // This is a bit of a hack, but it's the simplest way to reuse clap's help generation

        // Create a placeholder command for help generation
        let mut help_cmd = match command.command_type() {
            CommandType::Server => {
                // Generate server command help
                match command.name() {
                    "listeners" => clap::Command::new("listeners")
                        .about("manage listeners")
                        .subcommand(clap::Command::new("list").about("List all active listeners"))
                        .subcommand(
                            clap::Command::new("start")
                                .about("Start a new listener")
                                .arg(
                                    clap::Arg::new("type")
                                        .help("Listener type (http, tcp)")
                                        .required(true),
                                )
                                .arg(
                                    clap::Arg::new("bind")
                                        .short('b')
                                        .long("bind")
                                        .help("Address to bind to")
                                        .default_value("0.0.0.0"),
                                )
                                .arg(
                                    clap::Arg::new("port")
                                        .short('p')
                                        .long("port")
                                        .help("Port to listen on")
                                        .required(true),
                                ),
                        )
                        .subcommand(
                            clap::Command::new("stop").about("Stop a listener").arg(
                                clap::Arg::new("id")
                                    .help("Listener ID to stop")
                                    .required(true),
                            ),
                        ),
                    "sessions" => clap::Command::new("sessions")
                        .about("manage sessions")
                        .subcommand(clap::Command::new("list").about("List all active sessions"))
                        .subcommand(
                            clap::Command::new("use")
                                .about("Interact with a session")
                                .arg(
                                    clap::Arg::new("id")
                                        .help("Session ID to interact with")
                                        .required(true),
                                ),
                        )
                        .subcommand(
                            clap::Command::new("kill").about("Terminate a session").arg(
                                clap::Arg::new("id")
                                    .help("Session ID to terminate")
                                    .required(true),
                            ),
                        ),
                    "help" => clap::Command::new("help").about("help desc lol idk").arg(
                        clap::Arg::new("command")
                            .help("Show help for a specific command")
                            .required(false),
                    ),
                    // Add other server commands here
                    _ => {
                        // Generic help for unknown commands
                        // clap::Command::new(command.name()).about(command.description())
                        clap::Command::new("unknown").about("idk")
                    }
                }
            }
            CommandType::Implant => {
                // Generate implant command help
                match command.name() {
                    "ls" => clap::Command::new("ls")
                        .about(command.description().to_string())
                        .arg(
                            clap::Arg::new("path")
                                .help("Path to list")
                                .default_value("."),
                        ),
                    "pwd" => clap::Command::new("pwd").about("Print working directory on implant"),
                    "cat" => clap::Command::new("cat")
                        .about("Display file contents on implant")
                        .arg(
                            clap::Arg::new("file")
                                .help("File to display")
                                .required(true),
                        ),
                    // Add other implant commands here
                    _ => {
                        // Generic help for unknown commands
                        clap::Command::new("unkown").about("idk")
                    }
                }
            }
        };

        // Generate and return the help text
        let mut help_buffer = Vec::new();
        help_cmd.write_help(&mut help_buffer).unwrap();
        let help_text = String::from_utf8(help_buffer).unwrap();

        Ok(CommandOutput::Text(help_text))
    }
}
