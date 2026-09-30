//! Command metadata plus tab-completion and syntax highlighting for the REPL.
//!
//! Server command names come from the `rb` command registry and implant command names from the
//! shared `IMPLANT_COMMANDS` list, so the client stays in sync with what the server accepts.

use nu_ansi_term::{Color, Style};
use reedline::{Completer, Highlighter, Span, StyledText, Suggestion};

/// A command name and its description.
#[derive(Clone)]
pub struct CommandSpec {
    pub name: String,
    pub description: String,
}

/// Every word the client knows about: its own commands, server commands, and implant commands.
pub fn command_specs() -> Vec<CommandSpec> {
    let mut specs = vec![CommandSpec {
        name: "exit".to_string(),
        description: "Leave session mode or the client".to_string(),
    }];

    let registry = rb::command::CommandRegistry::new();
    for name in registry.list_server_commands() {
        let description = registry
            .get_server_command(name)
            .map(|command| command.description().to_string())
            .unwrap_or_default();
        specs.push(CommandSpec {
            name: name.to_string(),
            description,
        });
    }

    for command in rb::command::implant_cmds::IMPLANT_COMMANDS {
        specs.push(CommandSpec {
            name: command.name.to_string(),
            description: command.description.to_string(),
        });
    }

    specs
}

fn subcommands(first: &str) -> &'static [&'static str] {
    match first {
        "sessions" => &["list", "use", "kill"],
        "listeners" => &["list", "start", "stop"],
        "payload" => &["new"],
        _ => &[],
    }
}

/// Completes command names on the first word and subcommands on later words.
pub struct RbCompleter {
    specs: Vec<CommandSpec>,
}

impl RbCompleter {
    pub fn new(specs: Vec<CommandSpec>) -> Self {
        RbCompleter { specs }
    }
}

impl Completer for RbCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let before = &line[..pos.min(line.len())];
        let ends_with_space = before.ends_with(char::is_whitespace);
        let tokens: Vec<&str> = before.split_whitespace().collect();

        // Which word is being completed, and its byte span in the buffer.
        let (prefix, start) = if ends_with_space {
            ("", before.len())
        } else {
            match tokens.last() {
                Some(token) => (token.trim(), before.len() - token.len()),
                None => ("", before.len()),
            }
        };

        let first_word = tokens.is_empty() || (tokens.len() == 1 && !ends_with_space);

        let candidates: Vec<(String, String)> = if first_word {
            self.specs
                .iter()
                .map(|spec| (spec.name.clone(), spec.description.clone()))
                .collect()
        } else if let Some(first) = tokens.first() {
            subcommands(first)
                .iter()
                .map(|name| (name.to_string(), String::new()))
                .collect()
        } else {
            Vec::new()
        };

        candidates
            .into_iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, description)| Suggestion {
                value: name,
                description: (!description.is_empty()).then_some(description),
                style: None,
                extra: None,
                span: Span { start, end: pos },
                append_whitespace: true,
            })
            .collect()
    }
}

/// Colors a known command green and an unknown one red.
pub struct RbHighlighter {
    names: Vec<String>,
}

impl RbHighlighter {
    pub fn new(specs: Vec<CommandSpec>) -> Self {
        RbHighlighter {
            names: specs.into_iter().map(|spec| spec.name).collect(),
        }
    }
}

impl Highlighter for RbHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        if line.is_empty() {
            return styled;
        }

        let (first, rest) = match line.find(char::is_whitespace) {
            Some(index) => (&line[..index], &line[index..]),
            None => (line, ""),
        };

        let command_style = if self.names.iter().any(|name| name == first) {
            Style::new().fg(Color::Green).bold()
        } else {
            Style::new().fg(Color::Red)
        };

        styled.push((command_style, first.to_string()));
        if !rest.is_empty() {
            styled.push((Style::new(), rest.to_string()));
        }
        styled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_include_registry_and_implant_commands() {
        let names: Vec<String> = command_specs().into_iter().map(|spec| spec.name).collect();
        assert!(names.contains(&"sessions".to_string()));
        assert!(names.contains(&"listeners".to_string()));
        assert!(names.contains(&"pwd".to_string()));
        assert!(names.contains(&"exit".to_string()));
    }

    #[test]
    fn completes_the_first_word() {
        let mut completer = RbCompleter::new(command_specs());
        let suggestions = completer.complete("sess", 4);
        assert!(suggestions.iter().any(|s| s.value == "sessions"));
    }

    #[test]
    fn completes_subcommands() {
        let mut completer = RbCompleter::new(command_specs());
        let suggestions = completer.complete("sessions ", 9);
        let values: Vec<&str> = suggestions.iter().map(|s| s.value.as_str()).collect();
        assert_eq!(values, vec!["list", "use", "kill"]);
    }

    #[test]
    fn unknown_prefix_has_no_matches() {
        let mut completer = RbCompleter::new(command_specs());
        assert!(completer.complete("zzzz", 4).is_empty());
    }

    #[test]
    fn highlighter_marks_known_and_unknown() {
        let highlighter = RbHighlighter::new(command_specs());

        let known = highlighter.highlight("pwd", 0);
        assert_eq!(known.buffer[0].1, "pwd");

        let unknown = highlighter.highlight("nope", 0);
        assert_eq!(unknown.buffer[0].1, "nope");

        assert_ne!(known.buffer[0].0, unknown.buffer[0].0);
    }

    #[test]
    fn highlighter_preserves_the_line() {
        let highlighter = RbHighlighter::new(command_specs());
        let line = "ls /tmp";
        let styled = highlighter.highlight(line, 0);
        let rebuilt: String = styled.buffer.iter().map(|(_, text)| text.as_str()).collect();
        assert_eq!(rebuilt, line);
    }
}

