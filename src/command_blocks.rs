use anyhow::Result;
use sivtr_core::capture::scrollback;
use sivtr_core::session::{self, SessionEntry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyTarget {
    Block,
    Input,
    Output,
    Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectTarget {
    Block,
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommandBlock {
    pub input_with_prompt: String,
    pub input_without_prompt: String,
    pub output: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBlockSpan {
    pub line_start: usize,
    pub line_end: usize,
    pub input_line_range: Option<(usize, usize)>,
    pub output_line_range: Option<(usize, usize)>,
    pub parsed: ParsedCommandBlock,
}

impl CommandBlockSpan {
    pub fn text_for(&self, target: CopyTarget) -> Option<String> {
        let text = match target {
            CopyTarget::Block => match (
                self.parsed.input_with_prompt.is_empty(),
                self.parsed.output.is_empty(),
            ) {
                (false, false) => {
                    format!("{}\n{}", self.parsed.input_with_prompt, self.parsed.output)
                }
                (false, true) => self.parsed.input_with_prompt.clone(),
                (true, false) => self.parsed.output.clone(),
                (true, true) => String::new(),
            },
            CopyTarget::Input => self.parsed.input_with_prompt.clone(),
            CopyTarget::Output => self.parsed.output.clone(),
            CopyTarget::Command => self.parsed.command.clone(),
        };

        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub fn line_range_for(&self, target: SelectTarget) -> Option<(usize, usize)> {
        match target {
            SelectTarget::Block => Some((self.line_start, self.line_end)),
            SelectTarget::Input => self.input_line_range,
            SelectTarget::Output => self.output_line_range,
        }
    }
}

pub fn load_from_session_log() -> Result<Option<Vec<CommandBlockSpan>>> {
    let log_path = scrollback::session_log_path();
    if !log_path.exists() {
        return Ok(None);
    }

    let entries = session::load_entries(&log_path)?;
    if entries.is_empty() {
        return Ok(Some(Vec::new()));
    }

    Ok(Some(build_from_entries(&entries)))
}

pub fn build_from_entries(entries: &[SessionEntry]) -> Vec<CommandBlockSpan> {
    let mut blocks = Vec::with_capacity(entries.len());
    let mut line_start = 0usize;

    for entry in entries {
        let input_with_prompt = session::render_input(&entry.prompt, &entry.command);
        let input_without_prompt = entry.command.replace("\r\n", "\n").trim_end().to_string();
        let output = entry.output.replace("\r\n", "\n").trim_end_matches('\n').to_string();

        let input_line_count = line_count(&input_with_prompt);
        let output_line_count = line_count(&output);
        let block_line_count = input_line_count + output_line_count;
        if block_line_count == 0 {
            continue;
        }

        let input_line_range = if input_line_count > 0 {
            Some((line_start, line_start + input_line_count - 1))
        } else {
            None
        };
        let output_line_range = if output_line_count > 0 {
            Some((line_start + input_line_count, line_start + block_line_count - 1))
        } else {
            None
        };

        blocks.push(CommandBlockSpan {
            line_start,
            line_end: line_start + block_line_count - 1,
            input_line_range,
            output_line_range,
            parsed: ParsedCommandBlock {
                input_with_prompt,
                input_without_prompt: input_without_prompt.clone(),
                output,
                command: input_without_prompt,
            },
        });

        line_start += block_line_count;
    }

    blocks
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_line_ranges_from_structured_entries() {
        let entries = vec![
            SessionEntry {
                prompt: "PS C:\\repo> ".to_string(),
                command: "git status".to_string(),
                output: "clean".to_string(),
            },
            SessionEntry {
                prompt: "repo on main\n❯  ".to_string(),
                command: "cargo test".to_string(),
                output: "ok".to_string(),
            },
        ];

        let blocks = build_from_entries(&entries);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].line_range_for(SelectTarget::Block), Some((0, 1)));
        assert_eq!(blocks[0].line_range_for(SelectTarget::Input), Some((0, 0)));
        assert_eq!(blocks[0].line_range_for(SelectTarget::Output), Some((1, 1)));
        assert_eq!(blocks[1].line_range_for(SelectTarget::Block), Some((2, 4)));
        assert_eq!(blocks[1].line_range_for(SelectTarget::Input), Some((2, 3)));
        assert_eq!(blocks[1].line_range_for(SelectTarget::Output), Some((4, 4)));
    }

    #[test]
    fn preserves_output_only_blocks() {
        let entries = vec![SessionEntry {
            prompt: String::new(),
            command: String::new(),
            output: "warning: something happened\nwarning: still bad".to_string(),
        }];

        let blocks = build_from_entries(&entries);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].line_range_for(SelectTarget::Input), None);
        assert_eq!(blocks[0].line_range_for(SelectTarget::Output), Some((0, 1)));
        assert_eq!(
            blocks[0].parsed.output,
            "warning: something happened\nwarning: still bad"
        );
    }
}
