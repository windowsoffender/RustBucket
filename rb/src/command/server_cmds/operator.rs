/// Operator profile command module
use crate::command::*;
use crate::message::*;
use crate::store::OperatorInfo;
use std::time::SystemTime;

use super::get_arg_matches;
use clap;

#[derive(Debug)]
struct OperatorArgs {
    action: String,
    name: Option<String>,
    host: Option<String>,
    port: Option<u16>,
}

pub struct ServerOperatorCommand {}

impl RbCommand for ServerOperatorCommand {
    fn name(&self) -> &'static str {
        "operator"
    }

    fn command_type(&self) -> CommandType {
        CommandType::Server
    }

    fn description(&self) -> &'static str {
        "Manage operator profiles"
    }

    fn parse_args(&self, command_line: &str) -> Result<Box<dyn Any>, clap::Error> {
        let cmd = clap::Command::new(self.name())
            .about(self.description().to_string())
            .subcommand(
                clap::Command::new("new")
                    .about("Issue a new operator profile")
                    .arg(clap::Arg::new("name").help("Operator name").required(true))
                    .arg(
                        clap::Arg::new("host")
                            .long("host")
                            .help("Listener host to put in the profile"),
                    )
                    .arg(
                        clap::Arg::new("port")
                            .long("port")
                            .help("Listener port to put in the profile"),
                    ),
            )
            .subcommand(clap::Command::new("list").about("List operator profiles"))
            .subcommand(
                clap::Command::new("revoke")
                    .about("Revoke an operator profile")
                    .arg(clap::Arg::new("name").help("Operator name").required(true)),
            )
            .arg_required_else_help(true);

        let matches = get_arg_matches(&cmd, command_line)?;

        let mut args = OperatorArgs {
            action: String::new(),
            name: None,
            host: None,
            port: None,
        };

        if let Some(sub_matches) = matches.subcommand_matches("new") {
            args.action = "new".to_string();
            args.name = sub_matches.get_one::<String>("name").cloned();
            args.host = sub_matches.get_one::<String>("host").cloned();
            args.port = sub_matches
                .get_one::<String>("port")
                .and_then(|p| p.parse::<u16>().ok());
        } else if matches.subcommand_matches("list").is_some() {
            args.action = "list".to_string();
        } else if let Some(sub_matches) = matches.subcommand_matches("revoke") {
            args.action = "revoke".to_string();
            args.name = sub_matches.get_one::<String>("name").cloned();
        }

        Ok(Box::new(args))
    }

    fn execute_with_parsed_args(
        &self,
        context: &mut CommandContext,
        args: Box<dyn Any>,
    ) -> CommandResult {
        let args = match args.downcast::<OperatorArgs>() {
            Ok(args) => *args,
            Err(_) => {
                return Err(CommandError::ExecutionFailed(
                    "Failed to process arguments".to_string(),
                ))
            }
        };

        match args.action.as_str() {
            "new" => {
                let name = match &args.name {
                    Some(name) => name,
                    None => {
                        return Err(CommandError::ExecutionFailed(
                            "Operator name required".to_string(),
                        ))
                    }
                };

                let issued = match context.pki.issue_operator(name) {
                    Ok(issued) => issued,
                    Err(e) => {
                        return Err(CommandError::ExecutionFailed(format!(
                            "Failed to issue certificate: {}",
                            e
                        )))
                    }
                };

                let profile_dir = format!("operators/{}", name);
                if let Err(e) = std::fs::create_dir_all(&profile_dir) {
                    return Err(CommandError::ExecutionFailed(format!(
                        "Failed to create {}: {}",
                        profile_dir, e
                    )));
                }

                let cert_path = format!("{}/client-cert.pem", profile_dir);
                let key_path = format!("{}/client-key.pem", profile_dir);
                let profile_path = format!("operators/{}.toml", name);

                if let Err(e) = std::fs::write(&cert_path, &issued.cert_pem) {
                    return Err(CommandError::ExecutionFailed(format!(
                        "Failed to write {}: {}",
                        cert_path, e
                    )));
                }
                if let Err(e) = std::fs::write(&key_path, &issued.key_pem) {
                    return Err(CommandError::ExecutionFailed(format!(
                        "Failed to write {}: {}",
                        key_path, e
                    )));
                }

                let host = args
                    .host
                    .clone()
                    .unwrap_or_else(|| advertise_host(&context.server_host));
                let port = args.port.unwrap_or(context.server_port);
                let profile = format!(
                    "host = \"{}\"\nport = {}\nmtls = true\nca_path = \"certs/ca-cert.pem\"\ncert_path = \"{}\"\nkey_path = \"{}\"\n",
                    host, port, cert_path, key_path
                );
                if let Err(e) = std::fs::write(&profile_path, profile) {
                    return Err(CommandError::ExecutionFailed(format!(
                        "Failed to write {}: {}",
                        profile_path, e
                    )));
                }

                if let Err(e) = context.store.upsert_operator(OperatorInfo {
                    name: name.clone(),
                    serial_hex: issued.serial_hex.clone(),
                    created_at: SystemTime::now(),
                    revoked: false,
                }) {
                    return Err(CommandError::Internal(format!(
                        "Failed to record operator: {}",
                        e
                    )));
                }

                Ok(CommandOutput::Text(format!(
                    "Operator '{}' created (serial {}). Profile: {}",
                    name, issued.serial_hex, profile_path
                )))
            }
            "list" => {
                let operators = context.store.list_operators();
                if operators.is_empty() {
                    return Ok(CommandOutput::Text("No operators".to_string()));
                }

                let headers = vec![
                    "Name".to_string(),
                    "Serial".to_string(),
                    "Created".to_string(),
                    "Revoked".to_string(),
                ];
                let rows = operators
                    .iter()
                    .map(|operator| {
                        vec![
                            operator.name.clone(),
                            operator.serial_hex.clone(),
                            unix_seconds(operator.created_at),
                            operator.revoked.to_string(),
                        ]
                    })
                    .collect();

                Ok(CommandOutput::Table { headers, rows })
            }
            "revoke" => {
                let name = match &args.name {
                    Some(name) => name,
                    None => {
                        return Err(CommandError::ExecutionFailed(
                            "Operator name required".to_string(),
                        ))
                    }
                };

                if context.store.set_operator_revoked(name, true) {
                    Ok(CommandOutput::Text(format!(
                        "Operator '{}' revoked (the CRL updater will pick it up)",
                        name
                    )))
                } else {
                    Err(CommandError::TargetNotFound(format!(
                        "Operator '{}' not found",
                        name
                    )))
                }
            }
            _ => Err(CommandError::ExecutionFailed(
                "No arguments provided".to_string(),
            )),
        }
    }
}

fn advertise_host(host: &str) -> String {
    if host == "0.0.0.0" || host.is_empty() {
        "localhost".to_string()
    } else {
        host.to_string()
    }
}

fn unix_seconds(time: SystemTime) -> String {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "unknown".to_string(),
    }
}
