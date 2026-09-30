// mod help;
mod help;
mod listeners;
mod operator;
mod sessions;
// mod ping;
// Add more command modules here

mod payload_command;
pub use payload_command::PayloadCommand;
// pub use help::HelpCommand;
pub use help::ServerHelpCommand;
pub use listeners::ServerListenersCommand;
pub use operator::ServerOperatorCommand;
pub use sessions::ServerSessionsCommand;
// pub use ping::PingCommand;
// Re-export other commands here
//

pub fn get_arg_matches(
    cmd: &clap::Command,
    args_str: &str,
) -> Result<clap::ArgMatches, clap::Error> {
    cmd.clone()
        .try_get_matches_from(args_str.split_whitespace())
}
