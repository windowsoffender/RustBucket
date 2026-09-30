use clap::{Arg, Command};
use colored::*;
use nu_ansi_term::Style;
use reedline::{
    ColumnarMenu, DefaultHinter, FileBackedHistory, MenuBuilder, Prompt, PromptEditMode,
    PromptHistorySearch, Reedline, ReedlineMenu, Signal,
};
use rustls::{ClientConfig, ClientConnection, RootCertStore, Stream};
use serde::Deserialize;
use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use rb::message::{CommandError, CommandOutput, CommandRequest};

mod commands;
mod utils;

// Custom prompt implementation with the requested name
struct RustBucketPrompt {
    // Current session ID if we're interacting with a session
    active_session: Option<usize>,
    // Info about the active session
    session_info: Option<String>,
}

impl RustBucketPrompt {
    fn new() -> Self {
        RustBucketPrompt {
            active_session: None,
            session_info: None,
        }
    }

    // Set an active session
    fn set_session(&mut self, session_id: usize, info: String) {
        self.active_session = Some(session_id);
        self.session_info = Some(info);
    }

    // Clear the active session
    fn clear_session(&mut self) {
        self.active_session = None;
        self.session_info = None;
    }
}

impl Prompt for RustBucketPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        match (self.active_session, &self.session_info) {
            (Some(id), Some(info)) => {
                // Session-specific prompt with session info
                Cow::Owned(format!(
                    "{}[{}]> ",
                    "Session".cyan().bold(),
                    info.bright_cyan()
                ))
            }
            (Some(id), None) => {
                // Session-specific prompt with just ID
                Cow::Owned(format!(
                    "{}[{}]> ",
                    "Session".cyan().bold(),
                    id.to_string().bright_cyan()
                ))
            }
            _ => {
                // Default prompt
                Cow::Owned(format!("{} ", "RustBucket>".white().bold()))
            }
        }
    }

    fn render_prompt_right(&self) -> Cow<str> {
        "".into()
    }

    fn render_prompt_indicator(&self, _mode: PromptEditMode) -> Cow<str> {
        "".into()
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<str> {
        "... ".into()
    }

    fn render_prompt_history_search_indicator(
        &self,
        _history_search: PromptHistorySearch,
    ) -> Cow<str> {
        ": ".into()
    }
}

fn main() -> io::Result<()> {
    // Define command line arguments using clap
    let matches = Command::new("RustBucket Client")
        .version("1.0")
        .author("CaveiraGamingHD")
        .about("Command and Control Client for RustBucket Server")
        .arg(
            Arg::new("host")
                .short('H')
                .long("host")
                .value_name("HOST")
                .help("C2 server hostname or IP address")
                .default_value("localhost"),
        )
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .value_name("PORT")
                .help("C2 server port")
                .default_value("6666"),
        )
        .arg(
            Arg::new("mtls")
                .short('m')
                .long("mtls")
                .help("Use mutual TLS authentication")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("ca-path")
                .long("ca-path")
                .value_name("CA_FILE")
                .help("Path to CA certificate file")
                .default_value("ca.pem"),
        )
        .arg(
            Arg::new("cert-path")
                .long("cert-path")
                .value_name("CERT_FILE")
                .help("Path to client certificate file")
                .default_value("client.pem"),
        )
        .arg(
            Arg::new("key-path")
                .long("key-path")
                .value_name("KEY_FILE")
                .help("Path to client key file")
                .default_value("client.key"),
        )
        .get_matches();

    // Extract all needed values from matches to avoid lifetime issues
    let host = matches.get_one::<String>("host").unwrap().clone();
    let port = matches.get_one::<String>("port").unwrap().clone();
    let server_address = format!("{}:{}", host, port);
    let use_mtls = matches.get_flag("mtls");

    // Extract paths early if mTLS is enabled
    let ca_path = matches.get_one::<String>("ca-path").unwrap().clone();
    let cert_path = matches.get_one::<String>("cert-path").unwrap().clone();
    let key_path = matches.get_one::<String>("key-path").unwrap().clone();

    // Connection type will be either plain TCP or mTLS
    enum ConnectionType {
        Plain(TcpStream),
        // Box everything so we have ownership and stable memory locations
        Mtls(Box<ClientConnection>, Box<TcpStream>),
    }

    // Connect to the server based on the connection type
    let mut connection = if use_mtls {
        // Use the paths extracted earlier

        // Load CA certificate
        println!("Loading CA certificate from {}...", ca_path.bright_cyan());
        let mut ca_reader = BufReader::new(match File::open(ca_path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!(
                    "{}: {}",
                    "Failed to open CA certificate file".bright_red(),
                    e
                );
                return Err(e);
            }
        });

        let mut root_store = RootCertStore::empty();
        let ca_certs = rustls_pemfile::certs(&mut ca_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                eprintln!("{}: {}", "Failed to parse CA certificate".bright_red(), e);
                io::Error::new(io::ErrorKind::InvalidData, e)
            })?;

        for cert in ca_certs {
            if let Err(e) = root_store.add(cert) {
                eprintln!(
                    "{}: {}",
                    "Failed to add CA certificate to store".bright_red(),
                    e
                );
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid CA cert",
                ));
            }
        }

        // Load client certificate
        println!(
            "Loading client certificate from {}...",
            cert_path.bright_cyan()
        );
        let mut cert_reader = BufReader::new(match File::open(cert_path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!(
                    "{}: {}",
                    "Failed to open client certificate file".bright_red(),
                    e
                );
                return Err(e);
            }
        });
        let client_certs = rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                eprintln!(
                    "{}: {}",
                    "Failed to parse client certificate".bright_red(),
                    e
                );
                io::Error::new(io::ErrorKind::InvalidData, e)
            })?;

        // Load client key
        println!("Loading client key from {}...", key_path.bright_cyan());
        let mut key_reader = BufReader::new(match File::open(key_path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("{}: {}", "Failed to open client key file".bright_red(), e);
                return Err(e);
            }
        });
        let mut keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                eprintln!("{}: {}", "Failed to parse client key".bright_red(), e);
                io::Error::new(io::ErrorKind::InvalidData, e)
            })?;

        if keys.is_empty() {
            eprintln!("{}", "No private keys found in key file".bright_red());
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "No private keys found",
            ));
        }

        // Create TLS configuration
        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_client_auth_cert(
                client_certs,
                rustls_pki_types::PrivateKeyDer::Pkcs8(keys.remove(0)),
            )
            .map_err(|e| {
                eprintln!("{}: {}", "Failed to configure TLS client".bright_red(), e);
                io::Error::new(io::ErrorKind::InvalidData, e)
            })?;

        // Connect with TLS
        let config = Arc::new(config);
        let tcp_stream = match TcpStream::connect(&server_address) {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!(
                    "{} {}: {}",
                    "Failed to connect to RustBucket C2 server at".bright_red(),
                    server_address.bright_red(),
                    e
                );
                return Err(e);
            }
        };

        // Use the host string directly for DNS name creation
        let server_name = rustls_pki_types::ServerName::DnsName(
            rustls_pki_types::DnsName::try_from(host)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid DNS name"))?,
        );

        // Create TLS connection
        let client = ClientConnection::new(config, server_name)
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionAborted, e))?;

        // Box the connection and stream for stable memory locations
        let boxed_client = Box::new(client);
        let boxed_stream = Box::new(tcp_stream);

        println!(
            "{} {} {}",
            "Connected to RustBucket C2 server at".green(),
            server_address.bright_green(),
            "(with mTLS)".bright_cyan()
        );

        ConnectionType::Mtls(boxed_client, boxed_stream)
    } else {
        // Connect with plain TCP
        match TcpStream::connect(&server_address) {
            Ok(stream) => {
                println!(
                    "{} {} {}",
                    "Connected to RustBucket C2 server at".green(),
                    server_address.bright_green(),
                    "(unencrypted)".yellow()
                );
                ConnectionType::Plain(stream)
            }
            Err(e) => {
                eprintln!(
                    "{} {}: {}",
                    "Failed to connect to RustBucket C2 server at".bright_red(),
                    server_address.bright_red(),
                    e
                );
                return Err(e);
            }
        }
    };

    // Create a new Reedline engine with history, completion and highlighting
    let specs = commands::command_specs();
    let history = match FileBackedHistory::with_file(1000, history_path()) {
        Ok(history) => history,
        Err(_) => FileBackedHistory::new(1000).expect("failed to create history"),
    };
    let mut line_editor = Reedline::create()
        .with_history(Box::new(history))
        .with_completer(Box::new(commands::RbCompleter::new(specs.clone())))
        .with_highlighter(Box::new(commands::RbHighlighter::new(specs)))
        .with_hinter(Box::new(
            DefaultHinter::default().with_style(Style::new().fg(nu_ansi_term::Color::DarkGray)),
        ))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(
            ColumnarMenu::default().with_name("completion_menu"),
        )))
        .with_ansi_colors(true);
    let mut prompt = RustBucketPrompt::new();

    let banner = r"
    ____             __  ____             __        __ 
   / __ \__  _______/ /_/ __ )__  _______/ /_____  / /_
  / /_/ / / / / ___/ __/ __  / / / / ___/ //_/ _ \/ __/
 / _, _/ /_/ (__  ) /_/ /_/ / /_/ / /__/ ,< /  __/ /_  
/_/ |_|\__,_/____/\__/_____/\__,_/\___/_/|_|\___/\__/  
                                                       
";

    println!("{}", banner.bright_yellow().bold());

    // Track current active session
    let mut active_session: Option<usize> = None;

    loop {
        let sig = line_editor.read_line(&prompt)?;
        match sig {
            Signal::Success(buffer) => {
                let input = buffer.trim();
                if input.is_empty() {
                    continue;
                }

                // Check for session management commands
                if input.starts_with("sessions use") {
                    // Parse session ID
                    let parts: Vec<&str> = input.split_whitespace().collect();
                    if parts.len() < 3 {
                        eprintln!("{}", "Usage: session use <session_id>".bright_red());
                        continue;
                    }

                    if let Ok(session_id) = parts[2].parse::<usize>() {
                        // Update active session
                        active_session = Some(session_id);

                        // Update prompt
                        prompt.set_session(session_id, format!("Session {}", session_id));

                        println!(
                            "{} {}",
                            "Interacting with session".green(),
                            session_id.to_string().bright_green()
                        );
                        continue;
                    } else {
                        eprintln!("{}", "Invalid session ID".bright_red());
                        continue;
                    }
                } else if input == "exit" || input == "quit" {
                    if active_session.is_some() {
                        // Leave session mode and return to the main console.
                        active_session = None;
                        prompt.clear_session();
                        println!("{}", "Returned to main console".green());
                        continue;
                    } else {
                        // Quit the client.
                        println!("\n{}", "Disconnecting from server...".yellow());
                        if let ConnectionType::Mtls(client, _) = &mut connection {
                            let _ = client.send_close_notify();
                        }
                        break Ok(());
                    }
                }

                // Create a CommandRequest
                let request = CommandRequest {
                    command_line: input.to_string(),
                    session_id: active_session,
                };

                // Serialize request to JSON
                let json_request = match serde_json::to_string(&request) {
                    Ok(json) => json,
                    Err(e) => {
                        eprintln!("{}: {}", "Failed to serialize command".bright_red(), e);
                        continue;
                    }
                };

                // Send the command to the server with a newline terminator
                let command = format!("{}\n", json_request);
                match &mut connection {
                    ConnectionType::Plain(stream) => {
                        stream.write_all(command.as_bytes())?;
                        stream.flush()?;
                    }
                    ConnectionType::Mtls(client, tcp_stream) => {
                        // Create a temporary Stream for writing
                        let mut stream = Stream::new(&mut **client, &mut **tcp_stream);
                        stream.write_all(command.as_bytes())?;
                        stream.flush()?;
                    }
                }

                // Read the response from the server
                let mut response_data = Vec::new();
                let mut buffer = [0; 1024];

                // Read in a loop until we have a complete response
                loop {
                    let n = match &mut connection {
                        ConnectionType::Plain(stream) => match stream.read(&mut buffer) {
                            Ok(0) => {
                                eprintln!("{}", "Server closed the connection".bright_red());
                                return Ok(());
                            }
                            Ok(n) => n,
                            Err(e) => {
                                eprintln!("{}: {}", "Failed to receive data".bright_red(), e);
                                return Err(e);
                            }
                        },
                        ConnectionType::Mtls(client, tcp_stream) => {
                            // Create a temporary Stream for reading
                            let mut stream = Stream::new(&mut **client, &mut **tcp_stream);
                            match stream.read(&mut buffer) {
                                Ok(0) => {
                                    eprintln!("{}", "Server closed the connection".bright_red());
                                    return Ok(());
                                }
                                Ok(n) => n,
                                Err(e) => {
                                    eprintln!("{}: {}", "Failed to receive data".bright_red(), e);
                                    return Err(e);
                                }
                            }
                        }
                    };

                    response_data.extend_from_slice(&buffer[..n]);

                    // Try to parse what we have so far to see if it's complete
                    if let Ok(response) =
                        serde_json::from_slice::<utils::ServerResponse>(&response_data)
                    {
                        // We have a complete response
                        match response {
                            utils::ServerResponse::Success(output) => {
                                utils::display_command_output(&output);
                            }
                            utils::ServerResponse::Error(error) => {
                                utils::display_command_error(&error);

                                // If we got a session error and we're in session mode, exit it
                                if let CommandError::SessionError(_)
                                | CommandError::NoActiveSession(_) = error
                                {
                                    if active_session.is_some() {
                                        active_session = None;
                                        prompt.clear_session();
                                        println!("{}", "Session interaction ended".yellow());
                                    }
                                }
                            }
                        }
                        break;
                    }
                    // If we couldn't parse yet, continue reading
                }
            }
            Signal::CtrlD => {
                println!("\n{}", "Disconnecting from server...".yellow());

                // Ensure proper shutdown if using TLS
                if let ConnectionType::Mtls(client, _) = &mut connection {
                    // Send close_notify to properly close the TLS connection
                    let _ = client.send_close_notify();
                    // We don't need to wait for the peer's close_notify
                    // since we're terminating anyway
                }

                break Ok(());
            }
            Signal::CtrlC => {
                println!("\n{}", "You sure you want to exit? (y/N)".yellow());
                let mut input = String::new();
                io::stdin()
                    .read_line(&mut input)
                    .expect("Failed to read line");
                if input.trim().eq_ignore_ascii_case("y") {
                    println!("{}", "Exiting...".red());
                    break Ok(());
                } else {
                    println!("{}", "Continuing...".green());
                }
            }
        }
    }
}

/// Where the REPL keeps its command history.
fn history_path() -> std::path::PathBuf {
    match std::env::var("HOME") {
        Ok(home) => std::path::PathBuf::from(home).join(".rustbucket_history"),
        Err(_) => std::path::PathBuf::from(".rustbucket_history"),
    }
}
