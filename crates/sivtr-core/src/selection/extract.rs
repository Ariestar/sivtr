use super::{Selection, SelectionMode};
use crate::buffer::cursor::Cursor;
use crate::buffer::Buffer;

/// Extract the selected text from the buffer based on the selection and current cursor.
pub fn extract_selection(buffer: &Buffer, selection: &Selection, cursor: &Cursor) -> String {
    match selection.mode {
        SelectionMode::Visual => extract_visual(buffer, selection, cursor),
        SelectionMode::VisualLine => extract_visual_line(buffer, selection, cursor),
        SelectionMode::VisualBlock => extract_visual_block(buffer, selection, cursor),
    }
}

/// Character-wise visual selection: from anchor to cursor, inclusive.
fn extract_visual(buffer: &Buffer, selection: &Selection, cursor: &Cursor) -> String {
    let (top_row, bot_row) = selection.row_range(cursor);

    // Determine which position comes first
    let (start, end) = if selection.anchor.row < cursor.row
        || (selection.anchor.row == cursor.row && selection.anchor.col <= cursor.col)
    {
        (selection.anchor, *cursor)
    } else {
        (*cursor, selection.anchor)
    };

    let mut result = String::new();

    for row in top_row..=bot_row {
        if let Some(line) = buffer.get_line(row) {
            if top_row == bot_row {
                // Single line selection
                let text = line.extract_by_display_cols(start.col, end.col + 1);
                result.push_str(&text);
            } else if row == top_row {
                // First line: from start col to end of line
                let text = line.extract_by_display_cols(start.col, line.display_width());
                result.push_str(&text);
                result.push('\n');
            } else if row == bot_row {
                // Last line: from beginning to end col
                let text = line.extract_by_display_cols(0, end.col + 1);
                result.push_str(&text);
            } else {
                // Middle lines: full line
                result.push_str(&line.content);
                result.push('\n');
            }
        }
    }

    result
}

/// Line-wise selection: full lines from top to bottom.
fn extract_visual_line(buffer: &Buffer, selection: &Selection, cursor: &Cursor) -> String {
    let (top, bot) = selection.row_range(cursor);
    let mut result = String::new();

    for row in top..=bot {
        if let Some(line) = buffer.get_line(row) {
            result.push_str(&line.content);
            result.push('\n');
        }
    }

    result
}

/// Block (rectangular) selection.
fn extract_visual_block(buffer: &Buffer, selection: &Selection, cursor: &Cursor) -> String {
    let (top, bot) = selection.row_range(cursor);
    let (left, right) = selection.col_range(cursor);

    let mut result = String::new();

    for row in top..=bot {
        if let Some(line) = buffer.get_line(row) {
            let text = line.extract_by_display_cols(left, right + 1);
            result.push_str(&text);
        }
        // Each row of the block is a separate line in the result
        if row < bot {
            result.push('\n');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn make_buffer(text: &str) -> Buffer {
        let lines = parse::parse_lines(text);
        Buffer::new(lines)
    }

    #[test]
    fn test_visual_single_line() {
        let buf = make_buffer("hello world");
        let sel = Selection::new(SelectionMode::Visual, Cursor::new(0, 0));
        let cursor = Cursor::new(0, 4);
        let text = extract_selection(&buf, &sel, &cursor);
        assert_eq!(text, "hello");
    }

    #[test]
    fn test_visual_line_mode() {
        let buf = make_buffer("line one\nline two\nline three");
        let sel = Selection::new(SelectionMode::VisualLine, Cursor::new(0, 0));
        let cursor = Cursor::new(1, 0);
        let text = extract_selection(&buf, &sel, &cursor);
        assert_eq!(text, "line one\nline two\n");
    }

    #[test]
    fn test_visual_block() {
        let buf = make_buffer("abcdef\nghijkl\nmnopqr");
        let sel = Selection::new(SelectionMode::VisualBlock, Cursor::new(0, 1));
        let cursor = Cursor::new(2, 3);
        let text = extract_selection(&buf, &sel, &cursor);
        // Should extract columns 1-3 from each row: "bcd", "hij", "nop"
        assert_eq!(text, "bcd\nhij\nnop");
    }

    #[test]
    fn test_visual_block_keeps_empty_rows_empty() {
        let buf = make_buffer("abcdef\n\nmnopqr");
        let sel = Selection::new(SelectionMode::VisualBlock, Cursor::new(0, 1));
        let cursor = Cursor::new(2, 3);
        let text = extract_selection(&buf, &sel, &cursor);
        assert_eq!(text, "bcd\n\nnop");
    }
}
