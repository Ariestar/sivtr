//! Project a dialogue (WorkRecord) into clipboard text.

use anyhow::{Context, Result};
use sivtr_core::record::{RecordTextMode, WorkAt, WorkRecord, WorkRef};

use crate::commands::browse::record_text_to_pair;
use crate::tui::workspace::TextPair;

use super::plan::Projection;

pub(super) fn project_record(
    record: &WorkRecord,
    projection: Projection,
    prompt_override: Option<&str>,
) -> Result<TextPair> {
    match projection {
        Projection::Exact(at) => exact_text(record, at),
        Projection::Both => Ok(record_text_to_pair(record.copy_text_with_prompt(
            RecordTextMode::Combined,
            true,
            prompt_override,
        ))),
        Projection::Input => Ok(record_text_to_pair(record.copy_text_with_prompt(
            RecordTextMode::Input,
            true,
            prompt_override,
        ))),
        Projection::Output => Ok(record_text_to_pair(
            record.copy_text(RecordTextMode::Output, false),
        )),
        Projection::Command => Ok(record_text_to_pair(
            record.copy_text(RecordTextMode::Command, false),
        )),
    }
}

fn exact_text(record: &WorkRecord, at: WorkAt) -> Result<TextPair> {
    let plain = record
        .content_for_at(at)
        .with_context(|| missing_at_message(&record.work_ref, at))?;
    Ok(TextPair {
        ansi: plain.clone(),
        plain,
    })
}

fn missing_at_message(work_ref: &WorkRef, at: WorkAt) -> String {
    match at {
        WorkAt::Whole => format!("No content for `{}`", work_ref.whole()),
        WorkAt::Line(line) => format!("No line {line} in `{}`", work_ref.whole()),
        WorkAt::Part { io, index } => {
            let label = match io {
                sivtr_core::record::WorkPartIo::Input => "input",
                sivtr_core::record::WorkPartIo::Output => "output",
            };
            format!("No {label} part {index} in `{}`", work_ref.whole())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::WorkPartIo;
    use sivtr_core::session::SessionEntry;
    use std::path::Path;

    #[test]
    fn projects_terminal_modes() {
        let record = WorkRecord::terminal(
            &SessionEntry::new("PS C:\\repo>", "git status --all -a", "clean"),
            Path::new("current"),
            0,
        )
        .unwrap();
        assert_eq!(
            project_record(&record, Projection::Both, None)
                .unwrap()
                .plain,
            "PS C:\\repo> git status --all -a\nclean"
        );
        assert_eq!(
            project_record(&record, Projection::Input, None)
                .unwrap()
                .plain,
            "PS C:\\repo> git status --all -a"
        );
        assert_eq!(
            project_record(&record, Projection::Output, None)
                .unwrap()
                .plain,
            "clean"
        );
        assert_eq!(
            project_record(&record, Projection::Command, None)
                .unwrap()
                .plain,
            "git status --all -a"
        );
    }

    #[test]
    fn rewrites_prompt_on_input() {
        let record = WorkRecord::terminal(
            &SessionEntry::new("PS C:\\repo>", "cargo test", "ok"),
            Path::new("current"),
            0,
        )
        .unwrap();
        assert_eq!(
            project_record(&record, Projection::Input, Some(":"))
                .unwrap()
                .plain,
            ": cargo test"
        );
    }

    #[test]
    fn exact_part_projection() {
        let record = WorkRecord::terminal(
            &SessionEntry::new("PS C:\\repo>", "cargo test", "ok"),
            Path::new("current"),
            0,
        )
        .unwrap();
        let text = project_record(
            &record,
            Projection::Exact(WorkAt::Part {
                io: WorkPartIo::Output,
                index: 1,
            }),
            None,
        )
        .unwrap();
        assert_eq!(text.plain, "ok");
    }
}
