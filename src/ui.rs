use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Clear},
    Frame,
};

use crate::editor::Editor;

struct Theme {
    status_bg: Color,
    status_fg: Color,
    line_num: Color,
    cursor_line: Color,
    sel_bg: Color,
    output_success: Color,
    output_fail: Color,
    dlg_border: Color,
    dlg_sel: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            status_bg: Color::Blue,
            status_fg: Color::White,
            line_num: Color::DarkGray,
            cursor_line: Color::Rgb(40, 40, 50),
            sel_bg: Color::Rgb(60, 60, 120),
            output_success: Color::Green,
            output_fail: Color::Red,
            dlg_border: Color::Cyan,
            dlg_sel: Color::Rgb(60, 60, 120),
        }
    }
}

pub fn render(frame: &mut Frame, editor: &Editor) {
    let theme = Theme::default();
    let area = frame.area();
    let show_output = editor.show_output && editor.build_result.is_some();

    let main_height = if show_output {
        (area.height as f64 * 0.6) as u16
    } else {
        area.height.saturating_sub(1)
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(main_height),
            Constraint::Length(1),
        ])
        .split(area);

    render_text_area(frame, chunks[0], editor, &theme);

    if editor.prompt.is_some() {
        render_prompt_bar(frame, chunks[1], editor, &theme);
    } else {
        render_status_bar(frame, chunks[1], editor, &theme);
    }

    if show_output {
        if let Some(ref result) = editor.build_result {
            let output_height = area.height.saturating_sub(main_height).saturating_sub(1);
            let output_area = Rect::new(area.x, main_height + 1, area.width, output_height);
            render_output_panel(frame, output_area, result, &theme);
        }
    }

    if editor.file_dialog.is_some() {
        render_file_dialog(frame, area, editor, &theme);
    }
}

fn render_text_area(frame: &mut Frame, area: Rect, editor: &Editor, theme: &Theme) {
    let view_height = area.height as usize;
    let scroll = editor.scroll_offset(view_height);
    let line_num_width = editor.buffer.num_lines().to_string().len().max(3);

    let mut lines: Vec<Line> = Vec::with_capacity(view_height);

    for i in 0..view_height {
        let buf_row = scroll + i;
        if buf_row >= editor.buffer.num_lines() {
            lines.push(Line::from(""));
            continue;
        }

        let text = editor.buffer.line_as_str(buf_row);
        let line_str = format!("{:>width$} ", buf_row + 1, width = line_num_width);

        let line_num_style = Style::default().fg(theme.line_num);
        let mut spans = vec![Span::styled(line_str, line_num_style)];

        let is_cursor_line = buf_row == editor.cursor_row;
        let in_selection = editor.selection.map_or(false, |(r1, _, r2, _)| {
            buf_row >= r1 && buf_row <= r2
        });

        if is_cursor_line {
            let bg = if in_selection { theme.sel_bg } else { theme.cursor_line };
            spans.push(Span::styled(text.to_string(), Style::default().bg(bg)));
        } else if in_selection {
            let (r1, c1, r2, c2) = editor.selection.unwrap();
            let line_len = text.len();
            let sel_start = if buf_row == r1 { c1 } else { 0 };
            let sel_end = if buf_row == r2 { c2 } else { line_len };
            if sel_start > 0 {
                spans.push(Span::raw(text[..sel_start].to_string()));
            }
            if sel_start < sel_end {
                spans.push(Span::styled(
                    text[sel_start..sel_end].to_string(),
                    Style::default().bg(theme.sel_bg),
                ));
            }
            if sel_end < line_len {
                spans.push(Span::raw(text[sel_end..].to_string()));
            }
        } else {
            spans.push(Span::raw(text.to_string()));
        }
        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);

    let cursor_visible = editor.cursor_row >= scroll
        && editor.cursor_row < scroll + view_height;
    if cursor_visible && editor.file_dialog.is_none() && editor.prompt.is_none() {
        let x = area.x + line_num_width as u16 + 1 + editor.cursor_col as u16;
        let y = area.y + (editor.cursor_row - scroll) as u16;
        frame.set_cursor_position(ratatui::layout::Position { x, y });
    }
}

fn render_status_bar(frame: &mut Frame, area: Rect, editor: &Editor, theme: &Theme) {
    let filename = editor.buffer.filename();
    let modified = if editor.buffer.modified { " ●" } else { "" };
    let pos = format!("Ln {}, Col {}", editor.cursor_row + 1, editor.cursor_col + 1);
    let has_compiler = editor.compiler_info.cc.is_some() || editor.compiler_info.cxx.is_some();
    let hint = if has_compiler {
        " ^S保存  ^O打开  ^N新建  F5编译  F6运行 "
    } else {
        " ^S保存  ^O打开  ^N新建  [无编译器] "
    };

    let status_style = Style::default().bg(theme.status_bg).fg(theme.status_fg);

    let full = format!(" {} {} {}  {}  {}", filename, modified, pos, hint, editor.message);
    let paragraph = Paragraph::new(full).style(status_style);
    frame.render_widget(paragraph, area);
}

fn render_prompt_bar(frame: &mut Frame, area: Rect, editor: &Editor, theme: &Theme) {
    if let Some(ref prompt) = editor.prompt {
        let display = format!(" {} {}", prompt.label, prompt.input);
        let cursor_x = area.x + prompt.label.len() as u16 + 1 + prompt.input.len() as u16;
        let style = Style::default().bg(theme.status_bg).fg(theme.status_fg);

        let paragraph = Paragraph::new(display.as_str()).style(style);
        frame.render_widget(paragraph, area);
        frame.set_cursor_position(ratatui::layout::Position { x: cursor_x, y: area.y });
    }
}

fn render_file_dialog(frame: &mut Frame, area: Rect, editor: &Editor, theme: &Theme) {
    if let Some(ref dlg) = editor.file_dialog {
        let dlg_width = (area.width as f64 * 0.6).min(60.0) as u16;
        let dlg_height = (area.height as f64 * 0.7).min(30.0) as u16;
        let x = area.x + (area.width.saturating_sub(dlg_width)) / 2;
        let y = area.y + (area.height.saturating_sub(dlg_height)) / 4;

        let dlg_area = Rect::new(x, y, dlg_width, dlg_height);

        let title = format!(" File Explorer: {} ", dlg.cwd.display());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.dlg_border))
            .title(title.as_str())
            .title_style(Style::default().fg(theme.dlg_border).add_modifier(Modifier::BOLD));

        frame.render_widget(Clear, dlg_area);
        frame.render_widget(&block, dlg_area);

        let inner = block.inner(dlg_area);
        let view_h = inner.height as usize;

        let scroll = if dlg.selected >= view_h {
            dlg.selected - view_h + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = Vec::new();
        for i in scroll..(scroll + view_h).min(dlg.entries.len()) {
            let entry = &dlg.entries[i];
            let is_selected = i == dlg.selected;
            let prefix = if entry.is_dir { "  \u{1F4C1} " } else { "  \u{1F4C4} " };
            let suffix = if entry.is_dir { "/" } else { "" };
            let display = format!("{}{}{}", prefix, entry.name, suffix);

            if is_selected {
                let style = Style::default().bg(theme.dlg_sel).fg(Color::White).add_modifier(Modifier::BOLD);
                lines.push(Line::from(Span::styled(display, style)));
            } else {
                let style = if entry.is_dir {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(Span::styled(display, style)));
            }
        }

        if lines.is_empty() {
            lines.push(Line::from(Span::styled("  (empty directory)", Style::default().fg(Color::DarkGray))));
        }

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, inner);
    }
}

fn render_output_panel(frame: &mut Frame, area: Rect, result: &crate::builder::BuildResult, theme: &Theme) {
    let border_color = if result.success { theme.output_success } else { theme.output_fail };
    let title = if result.success { " Output (Success) " } else { " Output (Failed) " };

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(border_color))
        .title(title)
        .title_style(Style::default().fg(border_color).add_modifier(Modifier::BOLD));

    let inner = block.inner(area);
    frame.render_widget(&block, area);

    let cmd_line = format!("$ {}\n", result.command_line);
    let body = cmd_line + &result.output;

    let paragraph = Paragraph::new(body)
        .style(Style::default().fg(theme.status_fg));
    frame.render_widget(paragraph, inner);
}
