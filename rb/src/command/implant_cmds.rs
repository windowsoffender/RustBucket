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
        description: "Print the implant's working directory",
    },
    ImplantCommandInfo {
        name: "ls",
        usage: "ls [path]",
        description: "List a directory",
    },
    ImplantCommandInfo {
        name: "cat",
        usage: "cat <file>",
        description: "Print a file's contents",
    },
    ImplantCommandInfo {
        name: "systeminfo",
        usage: "systeminfo",
        description: "Show system information",
    },
];

/// Look up an implant command by name.
pub fn find(name: &str) -> Option<&'static ImplantCommandInfo> {
    IMPLANT_COMMANDS.iter().find(|command| command.name == name)
}
