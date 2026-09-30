use colored::*;
use rb::message::*;
use rb::command::*;
use rb::task::*;
use serde::{Deserialize, Serialize};

// Represents the result from server
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ServerResponse {
    Success(CommandOutput),
    Error(CommandError),
}


// pub fn display_task_result(result: &TaskResult) {
//     println!("{}: {}", "Task ID".bright_cyan().bold(), result.task_id);
//     println!("{}: {}", "Session ID".bright_cyan().bold(), result.session_id);
//     println!("{}: {}", "Status".bright_cyan().bold(), 
//              if result.status == TaskStatus::Completed { "Success".bright_green() } else { "Failed".bright_red() });
//
//     // if let Some(ref command) = result.command {
//     //     println!("{}: {}", "Command".bright_cyan().bold(), command);
//     // }
//
//     println!("{}", "Output:".bright_cyan().bold());
//     println!("{}", "-".repeat(80));
//
//     if result.status == TaskStatus::Completed {
//         println!("{}", result.output.bright_white());
//     } else {
//         println!("{}", result.output.bright_red());
//     }
//
//     // println!("{}", "-".repeat(80));
//     // println!("{}: {}", "Completed".bright_cyan().bold(), result.completed_at);
// }


// Display command output with nice formatting and colors
pub fn display_command_output(output: &CommandOutput) {
    match output {
        CommandOutput::Text(text) => {
            // Text output
            println!("{}", text);
        }
        CommandOutput::Table { headers, rows } => {
            // Enhanced table display with colors
            println!();
            let header_line = headers
                .iter()
                .map(|h| h.bright_green().bold().to_string())
                .collect::<Vec<_>>()
                .join(" | ");

            println!("{}", header_line);
            println!("{}", "=".repeat(header_line.len()).dimmed());

            // Alternate row colors for better readability
            for (i, row) in rows.iter().enumerate() {
                let row_str = row.join(" | ");
                if i % 2 == 0 {
                    println!("{}", row_str.cyan());
                } else {
                    println!("{}", row_str.blue());
                }
            }

            println!();
        }
        CommandOutput::Json(value) => {
            // Pretty-print JSON with yellow color
            if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                // Add syntax highlighting to JSON
                // This is a simple version - a real JSON highlighter would be more sophisticated
                let highlighted = pretty
                    .replace("{", "{".bright_yellow().to_string().as_str())
                    .replace("}", "}".bright_yellow().to_string().as_str())
                    .replace("[", "[".bright_yellow().to_string().as_str())
                    .replace("]", "]".bright_yellow().to_string().as_str())
                    .replace(":", ":".bright_yellow().to_string().as_str())
                    .replace(",", ",".bright_yellow().to_string().as_str());

                println!("{}", highlighted);
            } else {
                println!("{:?}", value.to_string().yellow());
            }
        }
        CommandOutput::Binary(data) => {
            println!("{}", "Binary data:".bright_green());

            // Display binary data as a hex dump with colors
            for (i, chunk) in data.chunks(16).enumerate() {
                // Print offset
                print!("{:08x}  ", i * 16);

                // Print hex values
                for (j, byte) in chunk.iter().enumerate() {
                    if j == 8 {
                        print!(" "); // Extra space in the middle
                    }
                    print!("{:02x} ", byte);
                }

                // Fill remaining space if chunk is not full
                for _ in chunk.len()..16 {
                    print!("   ");
                }

                // Extra space for alignment
                if chunk.len() <= 8 {
                    print!(" ");
                }

                // Print ASCII representation
                print!(" │");
                for &byte in chunk {
                    if byte >= 32 && byte <= 126 {
                        // Printable ASCII
                        print!("{}", (byte as char).to_string().blue());
                    } else {
                        // Non-printable
                        print!("{}", ".".dimmed());
                    }
                }
                println!("│");
            }
            println!("\n{} bytes", data.len().to_string().green());
        }
        CommandOutput::None => {
            println!(
                "{}",
                "Command executed successfully with no output.".bright_green()
            );
        }
        CommandOutput::File { name, data } => {
            let path = std::path::PathBuf::from(name);
            match std::fs::write(&path, data) {
                Ok(_) => {
                    let absolute = std::fs::canonicalize(&path).unwrap_or(path);
                    println!("{} {}", "Saved".bright_green().bold(), absolute.display());
                }
                Err(e) => eprintln!("{}: {}", "Failed to save file".bright_red(), e),
            }
        }
    }
}

// Display command errors with appropriate colors
pub fn display_command_error(error: &CommandError) {
    match error {
        CommandError::InvalidArguments(msg) => {
            eprintln!("{}: {}", "Invalid Arguments".bright_red().bold(), msg);
        }
        CommandError::PermissionDenied(msg) => {
            eprintln!("{}: {}", "Permission Denied".bright_red().bold(), msg);
        }
        CommandError::ExecutionFailed(msg) => {
            eprintln!("{}: {}", "Execution Failed".bright_red().bold(), msg);
        }
        CommandError::TargetNotFound(msg) => {
            eprintln!("{}: {}", "Target Not Found".bright_yellow().bold(), msg);
        }
        CommandError::NoActiveSession(msg) => {
            eprintln!("{}: {}", "No Active Session".bright_yellow().bold(), msg);
        }
        CommandError::SessionError(msg) => {
            eprintln!("{}: {}", "Session Error".bright_red().bold(), msg);
        }
        CommandError::Internal(msg) => {
            eprintln!("{}: {}", "Internal Error".bright_red().bold(), msg);
        }
        CommandError::Timeout(msg) => {
            eprintln!("{}: {}", "Timeout".bright_red().bold(), msg);
        }
    }
}
