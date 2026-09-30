//! Metadata for the commands the implant executes natively.
//!
//! The implementations live in `rb_implant`; the server only needs the list to show help and to
//! reject unknown commands before they become tasks.

/// A command the implant understands.
pub struct ImplantCommandInfo {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
}

/// Every command the implant executes natively. Anything else is rejected.
pub const IMPLANT_COMMANDS: &[ImplantCommandInfo] = &[
    ImplantCommandInfo {
        name: "pwd",
        usage: "pwd",
        description: "Print the working directory",
    },
    ImplantCommandInfo {
        name: "ls",
        usage: "ls [path]",
        description: "List a directory",
    },
    ImplantCommandInfo {
        name: "cd",
        usage: "cd <path>",
        description: "Change the working directory",
    },
    ImplantCommandInfo {
        name: "cat",
        usage: "cat <file>",
        description: "Print a file's contents",
    },
    ImplantCommandInfo {
        name: "download",
        usage: "download <file>",
        description: "Send a file back to the operator",
    },
    ImplantCommandInfo {
        name: "mkdir",
        usage: "mkdir <dir>",
        description: "Create a directory",
    },
    ImplantCommandInfo {
        name: "rm",
        usage: "rm [-r] <path>",
        description: "Remove a file or directory",
    },
    ImplantCommandInfo {
        name: "mv",
        usage: "mv <src> <dst>",
        description: "Move or rename a file",
    },
    ImplantCommandInfo {
        name: "cp",
        usage: "cp [-r] <src> <dst>",
        description: "Copy a file or directory",
    },
    ImplantCommandInfo {
        name: "touch",
        usage: "touch <file>",
        description: "Create an empty file",
    },
    ImplantCommandInfo {
        name: "systeminfo",
        usage: "systeminfo",
        description: "Show system information",
    },
    ImplantCommandInfo {
        name: "whoami",
        usage: "whoami",
        description: "Print the current user",
    },
    ImplantCommandInfo {
        name: "env",
        usage: "env",
        description: "List environment variables",
    },
    ImplantCommandInfo {
        name: "ps",
        usage: "ps",
        description: "List running processes",
    },
    ImplantCommandInfo {
        name: "kill",
        usage: "kill <pid>",
        description: "Kill a process",
    },
    ImplantCommandInfo {
        name: "sleep",
        usage: "sleep <seconds>",
        description: "Sleep on the implant",
    },
    ImplantCommandInfo {
        name: "netstat",
        usage: "netstat",
        description: "Show network connections",
    },
    ImplantCommandInfo {
        name: "ipconfig",
        usage: "ipconfig",
        description: "Show network interfaces",
    },
    ImplantCommandInfo {
        name: "shell",
        usage: "shell <command>",
        description: "Run a shell command on the implant",
    },
];

/// Look up an implant command by name.
pub fn find(name: &str) -> Option<&'static ImplantCommandInfo> {
    IMPLANT_COMMANDS.iter().find(|command| command.name == name)
}
