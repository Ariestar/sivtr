//! Dual Input/Output content panes: text bodies, layout, and active-half access.
//!
//! One place owns empty placeholders, dynamic height split, and search→scroll mapping
//! so picker / render / help don't re-copy the same match arms.

use ratatui::layout::Rect;
use sivtr_core::record::{WorkAt, WorkPartIo};

use crate::tui::content::view::{content_view_line_count, ContentViewMode};

const EMPTY: &str = "<empty>";
/// Min pane height: top border + 1 content row + bottom border.
const MIN_PANE_H: u16 = 3;

/// Body text for one IO half (no section headers — panes own titles).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentIoTexts {
    pub(crate) input: String,
    pub(crate) output: String,
}

impl ContentIoTexts {
    pub(crate) fn join_displayed(&self) -> String {
        match (self.input_blank(), self.output_blank()) {
            (true, true) => EMPTY.to_string(),
            (false, true) => self.input.clone(),
            (true, false) => self.output.clone(),
            (false, false) => format!("{}\n\n{}", self.input, self.output),
        }
    }

    pub(crate) fn input_blank(&self) -> bool {
        self.input.trim().is_empty()
    }

    pub(crate) fn output_blank(&self) -> bool {
        self.output.trim().is_empty()
    }

    /// Text shown in a half pane (`<empty>` when blank).
    pub(crate) fn display(&self, half: ContentIoFocus) -> &str {
        let raw = match half {
            ContentIoFocus::Input => self.input.as_str(),
            ContentIoFocus::Output => self.output.as_str(),
        };
        if raw.trim().is_empty() {
            EMPTY
        } else {
            raw
        }
    }

    pub(crate) fn display_owned(&self, half: ContentIoFocus) -> String {
        self.display(half).to_string()
    }
}

/// Which content half keyboard / selection targets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContentIoFocus {
    #[default]
    Input,
    Output,
}

impl ContentIoFocus {
    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Input => Self::Output,
            Self::Output => Self::Input,
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Output => "Output",
        }
    }
}

/// Independent scroll offsets for the dual content panes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentScrolls {
    pub(crate) input: usize,
    pub(crate) output: usize,
}

impl ContentScrolls {
    pub(crate) fn get(self, focus: ContentIoFocus) -> usize {
        match focus {
            ContentIoFocus::Input => self.input,
            ContentIoFocus::Output => self.output,
        }
    }

    pub(crate) fn set(&mut self, focus: ContentIoFocus, value: usize) {
        match focus {
            ContentIoFocus::Input => self.input = value,
            ContentIoFocus::Output => self.output = value,
        }
    }

    pub(crate) fn get_mut(&mut self, focus: ContentIoFocus) -> &mut usize {
        match focus {
            ContentIoFocus::Input => &mut self.input,
            ContentIoFocus::Output => &mut self.output,
        }
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn clear_half(&mut self, focus: ContentIoFocus) {
        self.set(focus, 0);
    }

    pub(crate) fn clamp_to(&mut self, input_lines: usize, output_lines: usize) {
        self.input = self.input.min(input_lines.saturating_sub(1));
        self.output = self.output.min(output_lines.saturating_sub(1));
    }
}

/// Geometry of the dual content panes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ContentIoAreas {
    pub(crate) input: Rect,
    pub(crate) output: Rect,
}

impl ContentIoAreas {
    pub(crate) fn area(self, half: ContentIoFocus) -> Rect {
        match half {
            ContentIoFocus::Input => self.input,
            ContentIoFocus::Output => self.output,
        }
    }

    pub(crate) fn hit_test(self, column: u16, row: u16) -> Option<ContentIoFocus> {
        if rect_contains(self.input, column, row) {
            Some(ContentIoFocus::Input)
        } else if rect_contains(self.output, column, row) {
            Some(ContentIoFocus::Output)
        } else {
            None
        }
    }
}

/// Active half: area + display text + scroll slot.
pub(crate) struct ActiveHalf<'a> {
    pub(crate) area: Rect,
    pub(crate) text: &'a str,
    pub(crate) scroll: &'a mut usize,
}

/// Borrowed view of both halves for one frame (texts computed once).
pub(crate) struct ContentIoFrame<'a> {
    pub(crate) texts: &'a ContentIoTexts,
    pub(crate) areas: ContentIoAreas,
    pub(crate) input_lines: usize,
    pub(crate) output_lines: usize,
}

impl<'a> ContentIoFrame<'a> {
    pub(crate) fn build(
        area: Rect,
        texts: &'a ContentIoTexts,
        mode: ContentViewMode,
    ) -> Self {
        let areas = content_io_layout(area, texts, mode);
        let input_lines = content_view_line_count(areas.input, texts.display(ContentIoFocus::Input), mode)
            .max(1);
        let output_lines =
            content_view_line_count(areas.output, texts.display(ContentIoFocus::Output), mode)
                .max(1);
        Self {
            texts,
            areas,
            input_lines,
            output_lines,
        }
    }

    pub(crate) fn line_count(&self, half: ContentIoFocus) -> usize {
        match half {
            ContentIoFocus::Input => self.input_lines,
            ContentIoFocus::Output => self.output_lines,
        }
    }

    pub(crate) fn active(
        &'a self,
        half: ContentIoFocus,
        scrolls: &'a mut ContentScrolls,
    ) -> ActiveHalf<'a> {
        ActiveHalf {
            area: self.areas.area(half),
            text: self.texts.display(half),
            scroll: scrolls.get_mut(half),
        }
    }
}

/// Split content column by **display line weight** (dynamic pane heights).
///
/// Measures with a provisional 50/50 split (same width), then assigns height
/// proportional to each half's line count, with a shared minimum.
pub(crate) fn content_io_layout(
    area: Rect,
    texts: &ContentIoTexts,
    mode: ContentViewMode,
) -> ContentIoAreas {
    if area.height == 0 || area.width == 0 {
        return ContentIoAreas::default();
    }

    let provisional = split_vertical_equal(area);
    let in_lines = content_view_line_count(
        provisional.input,
        texts.display(ContentIoFocus::Input),
        mode,
    )
    .max(1);
    let out_lines = content_view_line_count(
        provisional.output,
        texts.display(ContentIoFocus::Output),
        mode,
    )
    .max(1);

    ContentIoAreas {
        input: split_top(area, weighted_top_height(area.height, in_lines, out_lines)),
        output: split_bottom(area, weighted_top_height(area.height, in_lines, out_lines)),
    }
}

fn weighted_top_height(total: u16, in_lines: usize, out_lines: usize) -> u16 {
    if total == 0 {
        return 0;
    }
    if total < MIN_PANE_H {
        return total / 2;
    }
    if total < MIN_PANE_H.saturating_mul(2) {
        // Not enough for two mins: give the heavier half more of what we have.
        let in_w = in_lines.max(1) as u32;
        let out_w = out_lines.max(1) as u32;
        return ((total as u32) * in_w / (in_w + out_w)).max(1) as u16;
    }
    let rem = total.saturating_sub(MIN_PANE_H.saturating_mul(2));
    let in_w = in_lines.max(1) as u32;
    let out_w = out_lines.max(1) as u32;
    let extra = (rem as u32) * in_w / (in_w + out_w);
    MIN_PANE_H.saturating_add(extra as u16)
}

fn split_vertical_equal(area: Rect) -> ContentIoAreas {
    let mid = weighted_top_height(area.height, 1, 1);
    ContentIoAreas {
        input: split_top(area, mid),
        output: split_bottom(area, mid),
    }
}

fn split_top(area: Rect, height: u16) -> Rect {
    let h = height.min(area.height);
    Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: h,
    }
}

fn split_bottom(area: Rect, top_height: u16) -> Rect {
    let top = top_height.min(area.height);
    Rect {
        x: area.x,
        y: area.y.saturating_add(top),
        width: area.width,
        height: area.height.saturating_sub(top),
    }
}

/// Map a search hit to (half, 0-based scroll in that half's displayed text).
///
/// - `WorkAt::Part` → half from `io`, line is part-local (best effort under fold).
/// - `WorkAt::Line` / `Whole` → prefer the non-blank half; if both have text,
///   treat `matched_line` as walking Input then Output display lines.
pub(crate) fn search_match_half(
    at: WorkAt,
    matched_line: usize,
    texts: &ContentIoTexts,
) -> (ContentIoFocus, usize) {
    let line0 = matched_line.saturating_sub(1);
    match at {
        WorkAt::Part {
            io: WorkPartIo::Output,
            ..
        } => (ContentIoFocus::Output, line0),
        WorkAt::Part { .. } => (ContentIoFocus::Input, line0),
        WorkAt::Line(_) | WorkAt::Whole => {
            let in_blank = texts.input_blank();
            let out_blank = texts.output_blank();
            if in_blank && !out_blank {
                return (ContentIoFocus::Output, line0);
            }
            if out_blank && !in_blank {
                return (ContentIoFocus::Input, line0);
            }
            let in_n = texts
                .display(ContentIoFocus::Input)
                .lines()
                .count()
                .max(1);
            if line0 < in_n {
                (ContentIoFocus::Input, line0)
            } else {
                (ContentIoFocus::Output, line0.saturating_sub(in_n))
            }
        }
    }
}

pub(crate) fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_trim_for_empty() {
        let texts = ContentIoTexts {
            input: "  \n".to_string(),
            output: "ok".to_string(),
        };
        assert!(texts.input_blank());
        assert_eq!(texts.display(ContentIoFocus::Input), EMPTY);
        assert_eq!(texts.display(ContentIoFocus::Output), "ok");
    }

    #[test]
    fn weighted_height_gives_more_to_heavier_half() {
        let h = weighted_top_height(40, 10, 2);
        assert!(h > 20);
        assert!(h <= 40 - MIN_PANE_H);
    }

    #[test]
    fn search_part_routes_by_io() {
        let texts = ContentIoTexts {
            input: "a".into(),
            output: "b\nc".into(),
        };
        let (half, scroll) = search_match_half(
            WorkAt::Part {
                io: WorkPartIo::Output,
                index: 1,
            },
            2,
            &texts,
        );
        assert_eq!(half, ContentIoFocus::Output);
        assert_eq!(scroll, 1);
    }

    #[test]
    fn search_line_walks_input_then_output() {
        let texts = ContentIoTexts {
            input: "i1\ni2".into(),
            output: "o1\no2".into(),
        };
        // 2 display lines in input; matched_line 3 (1-based) → output offset 0
        let (half, scroll) = search_match_half(WorkAt::Line(3), 3, &texts);
        assert_eq!(half, ContentIoFocus::Output);
        assert_eq!(scroll, 0);
        let (half, scroll) = search_match_half(WorkAt::Line(1), 1, &texts);
        assert_eq!(half, ContentIoFocus::Input);
        assert_eq!(scroll, 0);
    }
}

