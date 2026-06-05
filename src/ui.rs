use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use std::sync::OnceLock;
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};
use unicode_width::UnicodeWidthStr;

use crate::editor::Editor;

struct Theme {
    status_bg: Color,
    status_fg: Color,
    line_num: Color,
    cursor_line: Color,
    sel_bg: Color,
    output_success: Color,
    output_fail: Color,
    diagnostic_error: Color,
    diagnostic_warning: Color,
    search_bg: Color,
    bracket_bg: Color,
    bracket_colors: [Color; 6],
    completion_bg: Color,
    completion_sel: Color,
    dlg_border: Color,
    dlg_sel: Color,
    keyword: Color,
    type_name: Color,
    string: Color,
    comment: Color,
    number: Color,
    preprocessor: Color,
    function: Color,
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
            diagnostic_error: Color::Rgb(255, 85, 85),
            diagnostic_warning: Color::Rgb(255, 184, 108),
            search_bg: Color::Rgb(90, 70, 20),
            bracket_bg: Color::Rgb(20, 120, 140),
            bracket_colors: [
                Color::Rgb(255, 184, 108),
                Color::Rgb(139, 233, 253),
                Color::Rgb(189, 147, 249),
                Color::Rgb(80, 250, 123),
                Color::Rgb(255, 121, 198),
                Color::Rgb(241, 250, 140),
            ],
            completion_bg: Color::Rgb(30, 32, 42),
            completion_sel: Color::Rgb(60, 60, 120),
            dlg_border: Color::Cyan,
            dlg_sel: Color::Rgb(60, 60, 120),
            keyword: Color::Rgb(255, 121, 198),
            type_name: Color::Rgb(139, 233, 253),
            string: Color::Rgb(241, 250, 140),
            comment: Color::Rgb(98, 114, 164),
            number: Color::Rgb(189, 147, 249),
            preprocessor: Color::Rgb(80, 250, 123),
            function: Color::Rgb(255, 184, 108),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HighlightMode {
    C,
    Cpp,
    Python,
    Syntect,
}

pub fn render(frame: &mut Frame, editor: &Editor) {
    let theme = Theme::default();
    let area = frame.area();
    let show_output = editor.show_output && editor.build_result.is_some();
    let show_help = editor.show_help;

    let main_height = if show_help {
        (area.height as f64 * 0.45) as u16
    } else if show_output {
        (area.height as f64 * 0.6) as u16
    } else {
        area.height.saturating_sub(1)
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(main_height), Constraint::Length(1)])
        .split(area);

    render_text_area(frame, chunks[0], editor, &theme);

    if editor.prompt.is_some() {
        render_prompt_bar(frame, chunks[1], editor, &theme);
    } else {
        render_status_bar(frame, chunks[1], editor, &theme);
    }

    if show_output && !show_help {
        if let Some(ref result) = editor.build_result {
            let output_height = area.height.saturating_sub(main_height).saturating_sub(1);
            let output_area = Rect::new(area.x, main_height + 1, area.width, output_height);
            render_output_panel(frame, output_area, result, editor, &theme);
        }
    }

    if show_help {
        let help_height = area.height.saturating_sub(main_height).saturating_sub(1);
        let help_area = Rect::new(area.x, main_height + 1, area.width, help_height);
        render_help_panel(frame, help_area);
    }

    if editor.file_dialog.is_some() {
        render_file_dialog(frame, area, editor, &theme);
    }

    render_completion_popup(frame, chunks[0], editor, &theme);
}

fn render_text_area(frame: &mut Frame, area: Rect, editor: &Editor, theme: &Theme) {
    let view_height = area.height as usize;
    let scroll = editor.scroll_offset(view_height);
    let line_num_width = editor.buffer.num_lines().to_string().len().max(3);
    let syntax_set = syntax_set();
    let theme_set = theme_set();
    let highlight_mode = highlight_mode(editor);
    let syntax = match highlight_mode {
        HighlightMode::Syntect => syntax_for_buffer(editor, syntax_set),
        _ => None,
    };
    let syntax_theme = theme_set.themes.get("base16-ocean.dark");
    let mut highlighter = syntax
        .zip(syntax_theme)
        .map(|(syntax, theme)| HighlightLines::new(syntax, theme));

    if let Some(ref mut highlighter) = highlighter {
        for row in 0..scroll.min(editor.buffer.num_lines()) {
            let _ = highlighter.highlight_line(editor.buffer.line_as_str(row), syntax_set);
        }
    }

    let mut lines: Vec<Line> = Vec::with_capacity(view_height);

    for i in 0..view_height {
        let buf_row = scroll + i;
        if buf_row >= editor.buffer.num_lines() {
            lines.push(Line::from(""));
            continue;
        }

        let text = editor.buffer.line_as_str(buf_row);
        let line_str = format!("{:>width$} ", buf_row + 1, width = line_num_width);

        let has_error = editor.diagnostics.iter().any(|d| d.row == buf_row);
        let line_num_style = if has_error {
            Style::default()
                .fg(theme.diagnostic_error)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.line_num)
        };
        let mut spans = vec![Span::styled(line_str, line_num_style)];

        let base_spans = highlight_spans(
            text,
            syntax_set,
            highlighter.as_mut(),
            highlight_mode,
            theme,
        );
        let line_spans = apply_line_backgrounds(base_spans, buf_row, editor, theme);
        for span in apply_ide_marks(line_spans, buf_row, editor, theme) {
            spans.push(span);
        }
        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);

    let cursor_visible = editor.cursor_row >= scroll && editor.cursor_row < scroll + view_height;
    if cursor_visible && editor.file_dialog.is_none() && editor.prompt.is_none() {
        let visual_col = editor
            .buffer
            .line_as_str(editor.cursor_row)
            .get(..editor.cursor_col)
            .map(UnicodeWidthStr::width)
            .unwrap_or(0);
        let x = area.x + line_num_width as u16 + 1 + visual_col as u16;
        let y = area.y + (editor.cursor_row - scroll) as u16;
        frame.set_cursor_position(ratatui::layout::Position { x, y });
    }
}

fn syntax_set() -> &'static SyntaxSet {
    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

fn highlight_mode(editor: &Editor) -> HighlightMode {
    match editor.buffer.extension() {
        "c" => HighlightMode::C,
        "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => HighlightMode::Cpp,
        "py" | "pyw" => HighlightMode::Python,
        _ if is_python_shebang(editor.buffer.line_as_str(0)) => HighlightMode::Python,
        _ => HighlightMode::Syntect,
    }
}

fn syntax_for_buffer<'a>(
    editor: &Editor,
    syntax_set: &'a SyntaxSet,
) -> Option<&'a SyntaxReference> {
    let extension = editor.buffer.extension();
    let syntax_name = match extension {
        "c" => Some("C"),
        "h" => Some("C++"),
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Some("C++"),
        "py" | "pyw" => Some("Python"),
        _ => None,
    };

    if let Some(name) = syntax_name {
        return syntax_set.find_syntax_by_name(name);
    }

    if is_python_shebang(editor.buffer.line_as_str(0)) {
        return syntax_set.find_syntax_by_name("Python");
    }

    syntax_set.find_syntax_by_extension(extension)
}

fn is_python_shebang(line: &str) -> bool {
    line.starts_with("#!/usr/bin/env python") || line.starts_with("#!/usr/bin/python")
}

fn highlight_spans<'a>(
    text: &'a str,
    syntax_set: &SyntaxSet,
    highlighter: Option<&mut HighlightLines>,
    mode: HighlightMode,
    theme: &Theme,
) -> Vec<Span<'a>> {
    match mode {
        HighlightMode::C | HighlightMode::Cpp => return highlight_c_like(text, mode, theme),
        HighlightMode::Python => return highlight_python(text, theme),
        HighlightMode::Syntect => {}
    }

    if let Some(highlighter) = highlighter {
        if let Ok(ranges) = highlighter.highlight_line(text, syntax_set) {
            return ranges
                .into_iter()
                .map(|(style, part)| Span::styled(part, syntect_style(style)))
                .collect();
        }
    }
    vec![Span::raw(text)]
}

fn highlight_c_like<'a>(text: &'a str, mode: HighlightMode, theme: &Theme) -> Vec<Span<'a>> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('#') {
        return vec![Span::styled(
            text,
            Style::default()
                .fg(theme.preprocessor)
                .add_modifier(Modifier::BOLD),
        )];
    }

    highlight_code_line(text, "//", theme, |word| {
        c_like_word_style(word, mode, theme)
    })
}

fn highlight_python<'a>(text: &'a str, theme: &Theme) -> Vec<Span<'a>> {
    highlight_code_line(text, "#", theme, |word| python_word_style(word, theme))
}

fn highlight_code_line<'a, F>(
    text: &'a str,
    line_comment: &str,
    theme: &Theme,
    word_style: F,
) -> Vec<Span<'a>>
where
    F: Fn(&str) -> Option<Style>,
{
    let mut spans = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if text[idx..].starts_with(line_comment) {
            push_plain(text, start, idx, &mut spans);
            spans.push(Span::styled(
                &text[idx..],
                Style::default()
                    .fg(theme.comment)
                    .add_modifier(Modifier::ITALIC),
            ));
            return spans;
        }

        if ch == '"' || ch == '\'' {
            push_plain(text, start, idx, &mut spans);
            let end = string_end(text, idx, ch);
            spans.push(Span::styled(
                &text[idx..end],
                Style::default().fg(theme.string),
            ));
            start = end;
            while chars.peek().is_some_and(|(next_idx, _)| *next_idx < end) {
                chars.next();
            }
            continue;
        }

        if ch.is_ascii_digit() {
            push_plain(text, start, idx, &mut spans);
            let end = token_end(text, idx, |c| {
                c.is_ascii_alphanumeric() || c == '.' || c == '_'
            });
            spans.push(Span::styled(
                &text[idx..end],
                Style::default().fg(theme.number),
            ));
            start = end;
            while chars.peek().is_some_and(|(next_idx, _)| *next_idx < end) {
                chars.next();
            }
            continue;
        }

        if is_ident_start(ch) {
            let end = token_end(text, idx, is_ident_continue);
            let word = &text[idx..end];
            if let Some(style) = word_style(word) {
                push_plain(text, start, idx, &mut spans);
                spans.push(Span::styled(word, style));
                start = end;
            } else if next_non_space(text, end) == Some('(') {
                push_plain(text, start, idx, &mut spans);
                spans.push(Span::styled(word, Style::default().fg(theme.function)));
                start = end;
            }
            while chars.peek().is_some_and(|(next_idx, _)| *next_idx < end) {
                chars.next();
            }
        }
    }

    push_plain(text, start, text.len(), &mut spans);
    spans
}

fn push_plain<'a>(text: &'a str, start: usize, end: usize, spans: &mut Vec<Span<'a>>) {
    if start < end {
        spans.push(Span::raw(&text[start..end]));
    }
}

fn string_end(text: &str, start: usize, quote: char) -> usize {
    let mut escaped = false;
    for (idx, ch) in text[start + quote.len_utf8()..].char_indices() {
        let idx = start + quote.len_utf8() + idx;
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return idx + quote.len_utf8();
        }
    }
    text.len()
}

fn token_end<F>(text: &str, start: usize, keep: F) -> usize
where
    F: Fn(char) -> bool,
{
    text[start..]
        .char_indices()
        .find_map(|(offset, ch)| (!keep(ch)).then_some(start + offset))
        .unwrap_or(text.len())
}

fn next_non_space(text: &str, start: usize) -> Option<char> {
    text[start..].chars().find(|ch| !ch.is_whitespace())
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn c_like_word_style(word: &str, mode: HighlightMode, theme: &Theme) -> Option<Style> {
    let is_cpp = mode == HighlightMode::Cpp;
    if matches!(
        word,
        "auto"
            | "break"
            | "case"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "else"
            | "enum"
            | "extern"
            | "for"
            | "goto"
            | "if"
            | "inline"
            | "register"
            | "return"
            | "sizeof"
            | "static"
            | "struct"
            | "switch"
            | "typedef"
            | "union"
            | "volatile"
            | "while"
    ) || (is_cpp
        && matches!(
            word,
            "class"
                | "namespace"
                | "new"
                | "delete"
                | "public"
                | "private"
                | "protected"
                | "template"
                | "typename"
                | "using"
                | "try"
                | "catch"
                | "throw"
                | "operator"
        ))
    {
        return Some(
            Style::default()
                .fg(theme.keyword)
                .add_modifier(Modifier::BOLD),
        );
    }

    if matches!(
        word,
        "bool"
            | "char"
            | "double"
            | "float"
            | "int"
            | "long"
            | "short"
            | "signed"
            | "unsigned"
            | "void"
    ) || (is_cpp
        && matches!(
            word,
            "string" | "vector" | "map" | "set" | "queue" | "stack" | "pair" | "size_t"
        ))
    {
        return Some(Style::default().fg(theme.type_name));
    }

    if is_cpp && matches!(word, "cin" | "cout" | "cerr" | "clog" | "endl" | "getline") {
        return Some(Style::default().fg(theme.function));
    }

    None
}

fn python_word_style(word: &str, theme: &Theme) -> Option<Style> {
    if matches!(
        word,
        "and"
            | "as"
            | "assert"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "False"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "None"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "True"
            | "try"
            | "while"
            | "with"
            | "yield"
    ) {
        return Some(
            Style::default()
                .fg(theme.keyword)
                .add_modifier(Modifier::BOLD),
        );
    }

    if matches!(
        word,
        "int" | "str" | "float" | "bool" | "list" | "dict" | "set" | "tuple"
    ) {
        return Some(Style::default().fg(theme.type_name));
    }

    None
}

fn syntect_style(style: SyntectStyle) -> Style {
    Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ))
}

fn apply_line_backgrounds<'a>(
    spans: Vec<Span<'a>>,
    row: usize,
    editor: &Editor,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let is_cursor_line = row == editor.cursor_row;
    let selection = editor
        .selection
        .filter(|(r1, _, r2, _)| row >= *r1 && row <= *r2);

    if is_cursor_line {
        let bg = if selection.is_some() {
            theme.sel_bg
        } else {
            theme.cursor_line
        };
        return spans
            .into_iter()
            .map(|span| span.patch_style(Style::default().bg(bg)))
            .collect();
    }

    let Some((r1, c1, r2, c2)) = selection else {
        return spans;
    };

    let sel_start = if row == r1 { c1 } else { 0 };
    let sel_end = if row == r2 {
        c2
    } else {
        spans.iter().map(|span| span.content.len()).sum()
    };

    apply_selection_background(spans, sel_start, sel_end, theme.sel_bg)
}

fn apply_ide_marks<'a>(
    spans: Vec<Span<'a>>,
    row: usize,
    editor: &Editor,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let mut marked = spans;
    for bracket in editor.bracket_color_marks().iter().filter(|m| m.row == row) {
        let fg = theme.bracket_colors[bracket.color_index % theme.bracket_colors.len()];
        marked = apply_bracket_color(marked, bracket.col, bracket.col + 1, fg);
    }
    for search_match in editor.search_matches.iter().filter(|m| m.row == row) {
        marked = apply_selection_background(
            marked,
            search_match.start_col,
            search_match.end_col,
            theme.search_bg,
        );
    }
    for bracket in editor.bracket_matches().iter().filter(|m| m.row == row) {
        marked = apply_bracket_style(marked, bracket.start_col, bracket.end_col, theme.bracket_bg);
    }
    for diagnostic in editor.diagnostics.iter().filter(|d| d.row == row) {
        let bg = match diagnostic.severity {
            crate::editor::DiagnosticSeverity::Error => theme.diagnostic_error,
            crate::editor::DiagnosticSeverity::Warning => theme.diagnostic_warning,
        };
        let line_len = editor.buffer.line_len(row);
        let start = diagnostic.col.min(line_len);
        let end = next_mark_end(editor.buffer.line_as_str(row), start);
        marked = apply_diagnostic_style(marked, start, end, bg);
    }
    marked
}

fn apply_bracket_color<'a>(
    spans: Vec<Span<'a>>,
    start: usize,
    end: usize,
    fg: Color,
) -> Vec<Span<'a>> {
    apply_range_style(
        spans,
        start,
        end,
        Style::default().fg(fg).add_modifier(Modifier::BOLD),
    )
}

fn apply_bracket_style<'a>(
    spans: Vec<Span<'a>>,
    start: usize,
    end: usize,
    bg: Color,
) -> Vec<Span<'a>> {
    apply_range_style(
        spans,
        start,
        end,
        Style::default()
            .fg(Color::White)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    )
}

fn apply_diagnostic_style<'a>(
    spans: Vec<Span<'a>>,
    start: usize,
    end: usize,
    color: Color,
) -> Vec<Span<'a>> {
    if start >= end {
        return spans;
    }
    apply_range_style(
        spans,
        start,
        end,
        Style::default()
            .fg(color)
            .add_modifier(Modifier::UNDERLINED),
    )
}

fn apply_range_style<'a>(
    spans: Vec<Span<'a>>,
    start: usize,
    end: usize,
    patch: Style,
) -> Vec<Span<'a>> {
    if start >= end {
        return spans;
    }
    let mut result = Vec::new();
    let mut offset = 0;
    for span in spans {
        let content = span.content;
        let style = span.style;
        let span_end = offset + content.len();
        if span_end <= start || offset >= end {
            result.push(Span::styled(content, style));
        } else {
            let local_start =
                clamp_to_char_boundary(content.as_ref(), start.saturating_sub(offset));
            let local_end =
                clamp_to_char_boundary(content.as_ref(), end.min(span_end).saturating_sub(offset));
            let content = content.as_ref();
            if local_start > 0 {
                result.push(Span::styled(content[..local_start].to_string(), style));
            }
            if local_start < local_end {
                result.push(Span::styled(
                    content[local_start..local_end].to_string(),
                    style.patch(patch),
                ));
            }
            if local_end < content.len() {
                result.push(Span::styled(content[local_end..].to_string(), style));
            }
        }
        offset = span_end;
    }
    result
}

fn next_mark_end(line: &str, start: usize) -> usize {
    if start >= line.len() {
        return line.len();
    }
    let mut end = start;
    while end < line.len() {
        let ch = line[end..].chars().next().unwrap_or_default();
        if !(ch == '_' || ch.is_ascii_alphanumeric()) {
            break;
        }
        end += ch.len_utf8();
    }
    if end == start {
        line[start..]
            .chars()
            .next()
            .map(|ch| start + ch.len_utf8())
            .unwrap_or(line.len())
    } else {
        end
    }
}

fn apply_selection_background<'a>(
    spans: Vec<Span<'a>>,
    sel_start: usize,
    sel_end: usize,
    bg: Color,
) -> Vec<Span<'a>> {
    if sel_start >= sel_end {
        return spans;
    }

    let mut result = Vec::new();
    let mut offset = 0;

    for span in spans {
        let content = span.content;
        let style = span.style;
        let end = offset + content.len();

        if end <= sel_start || offset >= sel_end {
            result.push(Span::styled(content, style));
        } else {
            let local_start =
                clamp_to_char_boundary(content.as_ref(), sel_start.saturating_sub(offset));
            let local_end =
                clamp_to_char_boundary(content.as_ref(), (sel_end.min(end)).saturating_sub(offset));
            let content = content.as_ref();

            if local_start > 0 {
                result.push(Span::styled(content[..local_start].to_string(), style));
            }
            if local_start < local_end {
                result.push(Span::styled(
                    content[local_start..local_end].to_string(),
                    style.patch(Style::default().bg(bg)),
                ));
            }
            if local_end < content.len() {
                result.push(Span::styled(content[local_end..].to_string(), style));
            }
        }

        offset = end;
    }

    result
}

fn clamp_to_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn render_status_bar(frame: &mut Frame, area: Rect, editor: &Editor, theme: &Theme) {
    let filename = editor.buffer.filename();
    let modified = if editor.buffer.modified { " ●" } else { "" };
    let pos = format!(
        "Ln {}, Col {}",
        editor.cursor_row + 1,
        editor
            .buffer
            .line_char_count(editor.cursor_row, editor.cursor_col)
            + 1
    );
    let hint =
        " F1帮助  F2 AI配置  ^S保存  ^E AI改  ^R AI聊  ^Z回退  ^O打开  ^N新建  F5检查  F6运行 ";

    let status_style = Style::default().bg(theme.status_bg).fg(theme.status_fg);

    let diag = if editor.diagnostics.is_empty() {
        String::new()
    } else {
        format!("  Problems {}", editor.diagnostics.len())
    };
    let message = editor
        .active_task_status()
        .unwrap_or_else(|| editor.message.clone());
    let full = format!(
        " {} {} {}  {}{}  {}",
        filename, modified, pos, hint, diag, message
    );
    let paragraph = Paragraph::new(full).style(status_style);
    frame.render_widget(paragraph, area);
}

fn render_completion_popup(frame: &mut Frame, text_area: Rect, editor: &Editor, theme: &Theme) {
    if editor.completions.is_empty() || editor.file_dialog.is_some() || editor.prompt.is_some() {
        return;
    }
    let view_height = text_area.height as usize;
    let scroll = editor.scroll_offset(view_height);
    if editor.cursor_row < scroll || editor.cursor_row >= scroll + view_height {
        return;
    }
    let line_num_width = editor.buffer.num_lines().to_string().len().max(3) as u16;
    let visual_col = editor
        .buffer
        .line_as_str(editor.cursor_row)
        .get(..editor.cursor_col)
        .map(UnicodeWidthStr::width)
        .unwrap_or(0) as u16;
    let width = editor
        .completions
        .iter()
        .map(|item| item.label.len())
        .max()
        .unwrap_or(8)
        .clamp(8, 28) as u16
        + 2;
    let height = editor.completions.len().min(8) as u16;
    let x = (text_area.x + line_num_width + 1 + visual_col)
        .min(text_area.x + text_area.width.saturating_sub(width));
    let y = (text_area.y + (editor.cursor_row - scroll) as u16 + 1)
        .min(text_area.y + text_area.height.saturating_sub(height.max(1)));
    let area = Rect::new(x, y, width, height);
    frame.render_widget(Clear, area);
    let lines: Vec<Line> = editor
        .completions
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let style = if idx == editor.completion_selected {
                Style::default().bg(theme.completion_sel).fg(Color::White)
            } else {
                Style::default().bg(theme.completion_bg).fg(theme.type_name)
            };
            Line::from(Span::styled(format!(" {}", item.label), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_prompt_bar(frame: &mut Frame, area: Rect, editor: &Editor, theme: &Theme) {
    if let Some(ref prompt) = editor.prompt {
        let shown_input = if prompt.secret {
            "*".repeat(prompt.input.chars().count())
        } else {
            prompt.input.clone()
        };
        let display = format!(" {} {}", prompt.label, shown_input);
        let cursor_offset =
            UnicodeWidthStr::width(format!(" {} {}", prompt.label, shown_input).as_str()) as u16;
        let cursor_x = area
            .x
            .saturating_add(cursor_offset)
            .min(area.x + area.width.saturating_sub(1));
        let style = Style::default().bg(theme.status_bg).fg(theme.status_fg);

        let paragraph = Paragraph::new(display.as_str()).style(style);
        frame.render_widget(paragraph, area);
        frame.set_cursor_position(ratatui::layout::Position {
            x: cursor_x,
            y: area.y,
        });
    }
}

fn render_file_dialog(frame: &mut Frame, area: Rect, editor: &Editor, theme: &Theme) {
    if let Some(ref dlg) = editor.file_dialog {
        let total_width = (area.width as f64 * 0.8).min(80.0) as u16;
        let total_height = (area.height as f64 * 0.8).min(35.0) as u16;
        let x = area.x + (area.width.saturating_sub(total_width)) / 2;
        let y = area.y + (area.height.saturating_sub(total_height)) / 4;

        let dlg_area = Rect::new(x, y, total_width, total_height);

        let title = format!(" File Explorer: {} ", dlg.cwd.display());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.dlg_border))
            .title(title.as_str())
            .title_style(
                Style::default()
                    .fg(theme.dlg_border)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(Clear, dlg_area);
        frame.render_widget(&block, dlg_area);

        let inner = block.inner(dlg_area);

        // Split the dialog area into two panes: left for file list, right for preview
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner);

        let list_chunk = chunks[0];
        let preview_chunk = chunks[1];

        // Split the left pane into file list and help text
        let list_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // File list takes remaining space
                Constraint::Length(1), // Help text takes 1 line
            ])
            .split(list_chunk);

        let file_list_chunk = list_chunks[0];
        let help_chunk = list_chunks[1];

        // Render file list in the left pane
        let view_h = file_list_chunk.height as usize;
        let scroll = if dlg.selected >= view_h {
            dlg.selected - view_h + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = Vec::new();
        for i in scroll..(scroll + view_h).min(dlg.entries.len()) {
            let entry = &dlg.entries[i];
            let is_selected = i == dlg.selected;
            let prefix = if entry.is_dir {
                "  \u{1F4C1} "
            } else {
                "  \u{1F4C4} "
            };
            let suffix = if entry.is_dir { "/" } else { "" };
            let display = format!("{}{}{}", prefix, entry.name, suffix);

            if is_selected {
                let style = Style::default()
                    .bg(theme.dlg_sel)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD);
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
            lines.push(Line::from(Span::styled(
                "  (empty directory)",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, file_list_chunk);

        // Render help text
        let help_text = "Space:预览 r:重命名 d:删除 n:新建 m:新建目录 Esc:退出";
        let help_paragraph = Paragraph::new(help_text).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(help_paragraph, help_chunk);

        // Render preview in the right pane
        let preview_title = if dlg.preview_content.is_some() {
            let path_str = dlg
                .preview_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            format!(" Preview: {} ", path_str)
        } else {
            " Preview (Press Space to preview) ".to_string()
        };

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.dlg_border))
            .title(preview_title.as_str())
            .title_style(
                Style::default()
                    .fg(theme.dlg_border)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(&preview_block, preview_chunk);

        if let Some(ref content) = dlg.preview_content {
            let preview_inner = preview_block.inner(preview_chunk);
            let paragraph = Paragraph::new(content.as_str());
            frame.render_widget(paragraph, preview_inner);
        }
    }
}

fn render_output_panel(
    frame: &mut Frame,
    area: Rect,
    result: &crate::builder::BuildResult,
    editor: &Editor,
    theme: &Theme,
) {
    let border_color = if result.success {
        theme.output_success
    } else {
        theme.output_fail
    };
    let title = if result.success {
        " Output (Success, PgUp/PgDn scroll) "
    } else {
        " Output (Failed, PgUp/PgDn scroll) "
    };

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(border_color))
        .title(title)
        .title_style(
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(area);
    frame.render_widget(&block, area);

    if editor.diagnostics.is_empty() {
        let cmd_line = format!("$ {}\n", result.command_line);
        let body = cmd_line + &result.output;
        let paragraph = Paragraph::new(scrolled_lines(&body, editor.output_scroll, inner.height))
            .style(Style::default().fg(theme.status_fg));
        frame.render_widget(paragraph, inner);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(editor.diagnostics.len().min(6) as u16 + 1),
            Constraint::Min(1),
        ])
        .split(inner);
    let mut problem_lines = vec![Line::from(Span::styled(
        " Problems",
        Style::default()
            .fg(theme.diagnostic_error)
            .add_modifier(Modifier::BOLD),
    ))];
    for (idx, diagnostic) in editor.diagnostics.iter().take(6).enumerate() {
        let active = editor.active_diagnostic == Some(idx);
        let severity = match diagnostic.severity {
            crate::editor::DiagnosticSeverity::Error => "error",
            crate::editor::DiagnosticSeverity::Warning => "warning",
        };
        let style = if active {
            Style::default()
                .bg(theme.sel_bg)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(match diagnostic.severity {
                crate::editor::DiagnosticSeverity::Error => theme.diagnostic_error,
                crate::editor::DiagnosticSeverity::Warning => theme.diagnostic_warning,
            })
        };
        problem_lines.push(Line::from(Span::styled(
            format!(
                " {}:{}:{} {} {}",
                idx + 1,
                diagnostic.row + 1,
                diagnostic.col + 1,
                severity,
                diagnostic.message
            ),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(problem_lines), chunks[0]);

    let cmd_line = format!("$ {}\n", result.command_line);
    let body = cmd_line + &result.output;
    frame.render_widget(
        Paragraph::new(scrolled_lines(
            &body,
            editor.output_scroll,
            chunks[1].height,
        ))
        .style(Style::default().fg(theme.status_fg)),
        chunks[1],
    );
}

fn scrolled_lines(text: &str, scroll: usize, height: u16) -> Vec<Line<'static>> {
    let lines: Vec<&str> = text.lines().collect();
    let visible = height as usize;
    let max_scroll = lines.len().saturating_sub(visible);
    let start = scroll.min(max_scroll);
    lines
        .into_iter()
        .skip(start)
        .map(|line| Line::from(line.to_string()))
        .collect()
}

fn render_help_panel(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" 帮助  F1关闭 ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(area);
    frame.render_widget(&block, area);

    if inner.width >= 96 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(inner);
        for (area, lines) in columns.iter().zip(help_columns()) {
            render_help_column(frame, *area, lines);
        }
    } else {
        render_help_column(frame, inner, compact_help_lines());
    }
}

fn render_help_column(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, area);
}

fn help_columns() -> Vec<Vec<Line<'static>>> {
    vec![
        vec![
            help_heading("文件 / 编辑"),
            help_row("^S", "保存并自动检查"),
            help_row("^O", "打开文件"),
            help_row("^N", "新建文件"),
            help_row("^Q", "退出"),
            help_row("^Z", "回退"),
            help_row("^A", "全选"),
            help_row("^X/^C/^V", "剪切 / 复制 / 粘贴"),
            help_row("Tab / Enter", "缩进或接受补全 / 换行"),
            help_row("Alt+h/l", "按词左移 / 右移"),
        ],
        vec![
            help_heading("搜索 / 诊断"),
            help_row("^F", "搜索"),
            help_row("F3", "下一个搜索结果"),
            help_row("Shift+F3", "上一个搜索结果"),
            help_row("^Space", "手动触发补全"),
            help_row("F5", "检查 / 编译"),
            help_row("F6", "编译并运行"),
            help_row("F8 / Shift+F8", "下一个 / 上一个错误"),
            help_row("Problems", "点击错误跳到行列"),
            help_row("PgUp/PgDn", "输出和 AI 预览翻页"),
        ],
        vec![
            help_heading("AI / 其他"),
            help_row("^R", "AI 聊天"),
            help_row("^E", "AI 修改当前文件"),
            help_row("F2", "配置 AI 地址 / 模型 / Key"),
            help_row("y / n", "应用 / 放弃 AI 修改"),
            help_row("Esc", "关闭面板或退出输入"),
            help_row("F1", "关闭帮助"),
            help_row("保存后", "自动检查当前文件"),
            help_row("运行输入", "支持 cin / scanf / input()"),
            help_row("缺工具", "自动下载 TCC / w64devkit / Zig / uv"),
        ],
    ]
}

fn compact_help_lines() -> Vec<Line<'static>> {
    vec![
        help_heading("常用快捷键"),
        help_row("^S / ^O / ^N / ^Q", "保存 / 打开 / 新建 / 退出"),
        help_row("^Z / ^A", "回退 / 全选"),
        help_row("^X / ^C / ^V", "剪切 / 复制 / 粘贴"),
        help_row("^F / F3 / Shift+F3", "搜索 / 下一个 / 上一个"),
        help_row("^Space / Tab", "补全 / 接受补全或缩进"),
        help_row("F5 / F6", "检查编译 / 运行"),
        help_row("F8 / Shift+F8", "下一个 / 上一个错误"),
        help_row("^R / ^E / F2", "AI 聊天 / AI 修改 / AI 配置"),
        help_row("PgUp/PgDn / Esc", "面板翻页 / 关闭面板"),
    ]
}

fn help_heading(title: &'static str) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!("  {}", title),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )])
}

fn help_row(key: &'static str, desc: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{:<15}", key),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(desc),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    fn editor_for_path(path: &str, first_line: &str) -> Editor {
        Editor::new(Buffer {
            lines: vec![first_line.to_string()],
            filepath: Some(path.to_string()),
            modified: false,
        })
    }

    #[test]
    fn syntax_detection_prefers_project_languages() {
        let syntax_set = syntax_set();

        let c_editor = editor_for_path("main.c", "int main(void) {}");
        let cpp_editor = editor_for_path("main.hpp", "#include <iostream>");
        let py_editor = editor_for_path("main.py", "print('hi')");

        assert_eq!(syntax_for_buffer(&c_editor, syntax_set).unwrap().name, "C");
        assert_eq!(
            syntax_for_buffer(&cpp_editor, syntax_set).unwrap().name,
            "C++"
        );
        assert_eq!(
            syntax_for_buffer(&py_editor, syntax_set).unwrap().name,
            "Python"
        );
    }

    #[test]
    fn syntax_detection_uses_python_shebang_without_extension() {
        let syntax_set = syntax_set();
        let editor = editor_for_path("script", "#!/usr/bin/env python3");

        assert_eq!(
            syntax_for_buffer(&editor, syntax_set).unwrap().name,
            "Python"
        );
    }

    #[test]
    fn cpp_highlight_colors_keywords_strings_and_comments() {
        let theme = Theme::default();

        let spans = highlight_c_like(
            r#"cout << "hi"; string name; int age; // comment"#,
            HighlightMode::Cpp,
            &theme,
        );

        assert!(spans
            .iter()
            .any(|span| span.content.as_ref() == "cout" && span.style.fg == Some(theme.function)));
        assert!(spans
            .iter()
            .any(|span| span.content.as_ref() == "\"hi\"" && span.style.fg == Some(theme.string)));
        assert!(spans.iter().any(
            |span| span.content.as_ref() == "string" && span.style.fg == Some(theme.type_name)
        ));
        assert!(spans
            .iter()
            .any(|span| span.content.as_ref() == "int" && span.style.fg == Some(theme.type_name)));
        assert!(spans
            .iter()
            .any(|span| span.content.as_ref() == "// comment"
                && span.style.fg == Some(theme.comment)));
    }

    #[test]
    fn highlight_mode_uses_cpp_for_cpp_files() {
        let editor = editor_for_path("demo.cpp", "#include <iostream>");

        assert_eq!(highlight_mode(&editor), HighlightMode::Cpp);
    }

    #[test]
    fn selection_background_clamps_to_utf8_boundaries() {
        let spans = vec![Span::raw("你好")];

        let highlighted = apply_selection_background(spans, 1, 5, Color::Blue);

        assert_eq!(highlighted.len(), 2);
        assert_eq!(highlighted[0].content.as_ref(), "你");
        assert_eq!(highlighted[1].content.as_ref(), "好");
    }
}
