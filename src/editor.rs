use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::ai::AiTurn;
use crate::buffer::Buffer;
use crate::builder::{
    compile, compile_and_prepare_run, probe_compilers, select_compiler, BuildResult, CompilerInfo,
    InteractiveRun,
};

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
}

pub struct FileDialog {
    pub cwd: PathBuf,
    pub entries: Vec<FileEntry>,
    pub selected: usize,
    pub scroll: usize,
    pub preview_content: Option<String>,
    pub preview_path: Option<PathBuf>,
}

impl FileDialog {
    pub fn new(path: PathBuf) -> Self {
        let mut dlg = FileDialog {
            cwd: path,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            preview_content: None,
            preview_path: None,
        };
        dlg.refresh();
        dlg
    }

    pub fn refresh(&mut self) {
        let mut entries = Vec::new();
        if self.cwd.parent().is_some() {
            entries.push(FileEntry {
                name: "..".to_string(),
                is_dir: true,
            });
        }
        if let Ok(read_dir) = fs::read_dir(&self.cwd) {
            let mut dirs = Vec::new();
            let mut files = Vec::new();
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    dirs.push(FileEntry { name, is_dir: true });
                } else {
                    files.push(FileEntry {
                        name,
                        is_dir: false,
                    });
                }
            }
            dirs.sort_by(|a, b| a.name.cmp(&b.name));
            files.sort_by(|a, b| a.name.cmp(&b.name));
            entries.extend(dirs);
            entries.extend(files);
        }
        self.entries = entries;
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn selected_path(&self) -> PathBuf {
        let entry = &self.entries[self.selected];
        if entry.name == ".." {
            self.cwd.parent().unwrap_or(&self.cwd).to_path_buf()
        } else {
            self.cwd.join(&entry.name)
        }
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
            self.refresh();
        }
    }

    pub fn enter_dir(&mut self) {
        if self.entries.is_empty() || self.selected >= self.entries.len() {
            return;
        }
        let entry = &self.entries[self.selected];
        if entry.name == ".." {
            self.go_up();
        } else if entry.is_dir {
            self.cwd = self.cwd.join(&entry.name);
            self.refresh();
        }
    }

    pub fn rename_file(&mut self, old_name: &str, new_name: &str) -> Result<(), String> {
        validate_entry_name(new_name)?;
        if old_name == ".." {
            return Err("Cannot rename parent directory".to_string());
        }

        let old_path = self.cwd.join(old_name);
        let new_path = self.cwd.join(new_name);

        std::fs::rename(&old_path, &new_path).map_err(|e| format!("Failed to rename: {}", e))?;

        self.refresh();

        // Find the renamed file and select it
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.name == new_name {
                self.selected = i;
                break;
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub fn delete_selected(&mut self) -> Result<(), String> {
        if self.entries.is_empty() || self.selected >= self.entries.len() {
            return Err("No file selected".to_string());
        }

        let selected_name = self.entries[self.selected].name.clone();
        self.delete_entry(&selected_name)
    }

    pub fn delete_entry(&mut self, name: &str) -> Result<(), String> {
        if name == ".." {
            return Err("Cannot delete parent directory".to_string());
        }

        let path_to_delete = self.cwd.join(name);
        let is_dir = path_to_delete.is_dir();

        let result = if is_dir {
            std::fs::remove_dir_all(&path_to_delete)
        } else {
            std::fs::remove_file(&path_to_delete)
        };

        result.map_err(|e| format!("Failed to delete: {}", e))?;

        self.refresh();
        Ok(())
    }

    pub fn create_file(&mut self, name: &str) -> Result<(), String> {
        validate_entry_name(name)?;
        let path = self.cwd.join(name);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("Failed to create file: {}", e))?;
        self.refresh();
        Ok(())
    }

    pub fn create_directory(&mut self, name: &str) -> Result<(), String> {
        validate_entry_name(name)?;
        let path = self.cwd.join(name);
        std::fs::create_dir(&path).map_err(|e| format!("Failed to create directory: {}", e))?;
        self.refresh();
        Ok(())
    }

    pub fn preview_selected(&mut self) -> Result<(), String> {
        if self.entries.is_empty() || self.selected >= self.entries.len() {
            return Err("No file selected".to_string());
        }

        let selected_entry = &self.entries[self.selected];
        if selected_entry.name == ".." {
            return Err("Cannot preview parent directory".to_string());
        }

        let path = self.cwd.join(&selected_entry.name);

        if selected_entry.is_dir {
            // For directories, show basic info
            self.preview_content = Some("<DIRECTORY>".to_string());
            self.preview_path = Some(path);
        } else {
            // For files, attempt to read content (but limit size to keep lightweight)
            if let Ok(metadata) = std::fs::metadata(&path) {
                if metadata.len() > 100 * 1024 {
                    // 100KB limit
                    self.preview_content =
                        Some(format!("<FILE TOO LARGE: {} bytes>", metadata.len()));
                    self.preview_path = Some(path);
                    return Ok(());
                }
            }

            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    // Limit to first 100 lines to keep UI responsive
                    let limited_content: String =
                        content.lines().take(100).collect::<Vec<_>>().join("\n");

                    self.preview_content = Some(limited_content);
                    self.preview_path = Some(path);
                }
                Err(_) => {
                    self.preview_content = Some("<BINARY FILE>".to_string());
                    self.preview_path = Some(path);
                }
            }
        }

        Ok(())
    }
}

fn validate_entry_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Name cannot be empty".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("Name cannot contain path separators".to_string());
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err("Invalid name".to_string()),
    }
}

pub struct Prompt {
    pub label: String,
    pub input: String,
    pub secret: bool,
}

impl Prompt {
    pub fn new(label: &str) -> Self {
        Prompt {
            label: label.to_string(),
            input: String::new(),
            secret: false,
        }
    }

    pub fn secret(label: &str) -> Self {
        Prompt {
            label: label.to_string(),
            input: String::new(),
            secret: true,
        }
    }

    pub fn with_input(label: &str, input: &str) -> Self {
        Prompt {
            label: label.to_string(),
            input: input.to_string(),
            secret: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub row: usize,
    pub col: usize,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextMatch {
    pub row: usize,
    pub start_col: usize,
    pub end_col: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BracketColorMark {
    pub row: usize,
    pub col: usize,
    pub color_index: usize,
}

#[derive(Clone)]
struct EditSnapshot {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
    selection: Option<(usize, usize, usize, usize)>,
    modified: bool,
}

pub struct Editor {
    pub buffer: Buffer,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub selection: Option<(usize, usize, usize, usize)>,
    pub clipboard: String,
    pub message: String,
    pub quit: bool,
    pub show_output: bool,
    pub output_scroll: usize,
    pub show_help: bool,
    pub build_result: Option<BuildResult>,
    pub file_dialog: Option<FileDialog>,
    pub prompt: Option<Prompt>,
    pub pending_mkfile: Option<PathBuf>,
    pub pending_mkdir: Option<PathBuf>,
    pub pending_rename: Option<(PathBuf, String)>, // (directory, original_filename)
    pub pending_delete: Option<(PathBuf, String)>, // (directory, filename)
    pub pending_search: bool,
    pub pending_ai_key: bool,
    pub pending_ai_base_url: bool,
    pub pending_ai_model: bool,
    pub pending_ai_setup_key: Option<String>,
    pub pending_ai_setup_base_url: Option<String>,
    pub pending_ai_setup_model: Option<String>,
    pub pending_ai_setup_chat: bool,
    pub pending_ai_setup_only: bool,
    pub pending_ai_edit: bool,
    pub pending_ai_chat: bool,
    pub pending_ai_apply: bool,
    pub pending_ai_edit_content: Option<String>,
    pub ai_chat_history: Vec<AiTurn>,
    pub ai_edit_history: Vec<AiTurn>,
    pub pending_run: Option<InteractiveRun>,
    pub compiler_info: CompilerInfo,
    pub confirm_quit: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub active_diagnostic: Option<usize>,
    pub completions: Vec<CompletionItem>,
    pub completion_selected: usize,
    pub completion_prefix: String,
    pub completion_start_col: usize,
    pub search_query: String,
    pub search_matches: Vec<TextMatch>,
    pub active_search: Option<usize>,
    undo_stack: Vec<EditSnapshot>,
    last_key_time: Instant,
    last_key_code: Option<KeyCode>,
    last_key_modifiers: KeyModifiers,
}

impl Editor {
    pub fn new(buffer: Buffer) -> Self {
        Editor {
            buffer,
            cursor_row: 0,
            cursor_col: 0,
            selection: None,
            clipboard: String::new(),
            message: String::new(),
            quit: false,
            show_output: false,
            output_scroll: 0,
            show_help: false,
            build_result: None,
            file_dialog: None,
            prompt: None,
            pending_mkfile: None,
            pending_mkdir: None,
            pending_rename: None,
            pending_delete: None,
            pending_search: false,
            pending_ai_key: false,
            pending_ai_base_url: false,
            pending_ai_model: false,
            pending_ai_setup_key: None,
            pending_ai_setup_base_url: None,
            pending_ai_setup_model: None,
            pending_ai_setup_chat: false,
            pending_ai_setup_only: false,
            pending_ai_edit: false,
            pending_ai_chat: false,
            pending_ai_apply: false,
            pending_ai_edit_content: None,
            ai_chat_history: Vec::new(),
            ai_edit_history: Vec::new(),
            pending_run: None,
            compiler_info: probe_compilers(),
            confirm_quit: false,
            diagnostics: Vec::new(),
            active_diagnostic: None,
            completions: Vec::new(),
            completion_selected: 0,
            completion_prefix: String::new(),
            completion_start_col: 0,
            search_query: String::new(),
            search_matches: Vec::new(),
            active_search: None,
            undo_stack: Vec::new(),
            last_key_time: Instant::now(),
            last_key_code: None,
            last_key_modifiers: KeyModifiers::empty(),
        }
    }

    fn snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            lines: self.buffer.lines.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            selection: self.selection,
            modified: self.buffer.modified,
        }
    }

    fn restore_snapshot(&mut self, snapshot: EditSnapshot) {
        self.buffer.lines = snapshot.lines;
        self.cursor_row = snapshot.cursor_row;
        self.cursor_col = snapshot.cursor_col;
        self.selection = snapshot.selection;
        self.buffer.modified = snapshot.modified;
        self.clamp_cursor();
    }

    fn record_edit(&mut self) {
        self.undo_stack.push(self.snapshot());
    }

    pub fn clear_history(&mut self) {
        self.undo_stack.clear();
    }

    pub fn clear_ide_state(&mut self) {
        self.diagnostics.clear();
        self.active_diagnostic = None;
        self.close_completion();
        self.search_query.clear();
        self.search_matches.clear();
        self.active_search = None;
        self.pending_ai_key = false;
        self.pending_ai_base_url = false;
        self.pending_ai_model = false;
        self.pending_ai_setup_key = None;
        self.pending_ai_setup_base_url = None;
        self.pending_ai_setup_model = None;
        self.pending_ai_setup_chat = false;
        self.pending_ai_setup_only = false;
        self.pending_ai_edit = false;
        self.pending_ai_chat = false;
        self.pending_ai_apply = false;
        self.pending_ai_edit_content = None;
        self.ai_chat_history.clear();
        self.ai_edit_history.clear();
    }

    pub fn undo(&mut self) {
        let Some(snapshot) = self.undo_stack.pop() else {
            self.message = "Nothing to undo".to_string();
            return;
        };
        self.restore_snapshot(snapshot);
        self.message = "Undo".to_string();
        self.show_output = false;
    }

    pub fn clamp_cursor(&mut self) {
        let max_row = self.buffer.num_lines().max(1) - 1;
        self.cursor_row = self.cursor_row.min(max_row);
        self.cursor_col = clamp_to_char_boundary(
            self.buffer.line_as_str(self.cursor_row),
            self.cursor_col.min(self.buffer.line_len(self.cursor_row)),
        );
    }

    pub fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
        }
        self.close_completion();
        self.clamp_cursor();
    }
    pub fn move_down(&mut self) {
        if self.cursor_row + 1 < self.buffer.num_lines() {
            self.cursor_row += 1;
        }
        self.close_completion();
        self.clamp_cursor();
    }
    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col =
                prev_char_boundary(self.buffer.line_as_str(self.cursor_row), self.cursor_col)
                    .unwrap_or(0);
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.buffer.line_len(self.cursor_row);
        }
        self.close_completion();
    }
    pub fn move_right(&mut self) {
        let line = self.buffer.line_as_str(self.cursor_row);
        if self.cursor_col < line.len() {
            self.cursor_col = next_char_boundary(line, self.cursor_col).unwrap_or(line.len());
        } else if self.cursor_row + 1 < self.buffer.num_lines() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
        self.close_completion();
    }
    pub fn move_word_left(&mut self) {
        let line = self.buffer.line_as_str(self.cursor_row);
        let bytes = line.as_bytes();
        let mut pos = self.cursor_col.min(line.len());
        if pos == 0 {
            if self.cursor_row > 0 {
                self.cursor_row -= 1;
                self.cursor_col = self.buffer.line_len(self.cursor_row);
            }
            return;
        }
        pos = pos.saturating_sub(1);
        while pos > 0 && bytes[pos].is_ascii_whitespace() {
            pos -= 1;
        }
        let is_word = bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_';
        if is_word {
            while pos > 0 && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos -= 1;
            }
            if !(bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos += 1;
            }
        } else {
            while pos > 0
                && !bytes[pos].is_ascii_alphanumeric()
                && bytes[pos] != b'_'
                && !bytes[pos].is_ascii_whitespace()
            {
                pos -= 1;
            }
            if bytes[pos].is_ascii_alphanumeric()
                || bytes[pos] == b'_'
                || bytes[pos].is_ascii_whitespace()
            {
                pos += 1;
            }
        }
        self.cursor_col = clamp_to_char_boundary(line, pos);
        self.close_completion();
    }
    pub fn move_word_right(&mut self) {
        let line = self.buffer.line_as_str(self.cursor_row);
        let bytes = line.as_bytes();
        let mut pos = self.cursor_col;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            if self.cursor_row + 1 < self.buffer.num_lines() {
                self.cursor_row += 1;
                self.cursor_col = 0;
            }
            return;
        }
        let is_word = bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_';
        if is_word {
            while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos += 1;
            }
        } else {
            while pos < bytes.len()
                && !bytes[pos].is_ascii_alphanumeric()
                && bytes[pos] != b'_'
                && !bytes[pos].is_ascii_whitespace()
            {
                pos += 1;
            }
        }
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        self.cursor_col =
            clamp_to_char_boundary(line, if pos >= bytes.len() { bytes.len() } else { pos });
        self.close_completion();
    }
    pub fn home(&mut self) {
        self.cursor_col = 0;
        self.close_completion();
    }
    pub fn end(&mut self) {
        self.cursor_col = self.buffer.line_len(self.cursor_row);
        self.close_completion();
    }
    pub fn page_up(&mut self) {
        self.cursor_row = self.cursor_row.saturating_sub(20);
        self.close_completion();
        self.clamp_cursor();
    }
    pub fn page_down(&mut self) {
        self.cursor_row = (self.cursor_row + 20).min(self.buffer.num_lines().saturating_sub(1));
        self.close_completion();
        self.clamp_cursor();
    }

    pub fn scroll_output_up(&mut self, lines: usize) {
        self.output_scroll = self.output_scroll.saturating_sub(lines);
    }

    pub fn scroll_output_down(&mut self, lines: usize) {
        self.output_scroll = self.output_scroll.saturating_add(lines);
    }

    pub fn insert_char(&mut self, c: char) {
        self.record_edit();
        if let Some((r1, c1, r2, c2)) = self.selection.take() {
            self.delete_selection_range(r1, c1, r2, c2);
        }
        if c == '\n' {
            self.insert_newline_with_indent();
        } else if c == '\t' {
            for _ in 0..4 {
                self.buffer
                    .insert_char(self.cursor_row, self.cursor_col, ' ');
                self.cursor_col += 1;
            }
        } else {
            self.buffer.insert_char(self.cursor_row, self.cursor_col, c);
            self.cursor_col += c.len_utf8();
        }
        if is_ident_continue(c) {
            self.refresh_completions();
        } else {
            self.close_completion();
        }
    }

    fn insert_newline_with_indent(&mut self) {
        let indent = self.smart_indent_for_newline();
        self.buffer
            .insert_char(self.cursor_row, self.cursor_col, '\n');
        self.cursor_row += 1;
        self.cursor_col = 0;
        for ch in indent.chars() {
            self.buffer
                .insert_char(self.cursor_row, self.cursor_col, ch);
            self.cursor_col += ch.len_utf8();
        }
    }

    fn smart_indent_for_newline(&self) -> String {
        let line = self.buffer.line_as_str(self.cursor_row);
        let before_cursor = line.get(..self.cursor_col).unwrap_or(line);
        let after_cursor = line.get(self.cursor_col..).unwrap_or("");
        let mut indent = leading_whitespace(before_cursor).to_string();

        if should_increase_indent(before_cursor, self.buffer.extension()) {
            indent.push_str("    ");
        }

        if starts_with_closing_token(after_cursor) && indent.len() >= 4 {
            indent.truncate(indent.len() - 4);
        }

        indent
    }

    pub fn backspace(&mut self) {
        let before = self.snapshot();
        if let Some((r1, c1, r2, c2)) = self.selection.take() {
            self.delete_selection_range(r1, c1, r2, c2);
            self.undo_stack.push(before);
            self.close_completion();
            return;
        }
        let new_col = if self.cursor_col > 0 {
            prev_char_boundary(self.buffer.line_as_str(self.cursor_row), self.cursor_col)
        } else {
            None
        };
        let ok = self.buffer.backspace(self.cursor_row, self.cursor_col);
        if ok {
            self.undo_stack.push(before);
            if self.cursor_col > 0 {
                self.cursor_col = new_col.unwrap_or(0);
            } else if self.cursor_row > 0 {
                self.cursor_row -= 1;
                self.cursor_col = self.buffer.line_len(self.cursor_row);
            }
            self.refresh_completions();
        }
    }

    pub fn delete(&mut self) {
        let before = self.snapshot();
        if let Some((r1, c1, r2, c2)) = self.selection.take() {
            self.delete_selection_range(r1, c1, r2, c2);
            self.undo_stack.push(before);
            self.close_completion();
            return;
        }
        if self.buffer.delete_char(self.cursor_row, self.cursor_col) {
            self.undo_stack.push(before);
            self.clamp_cursor();
            self.refresh_completions();
        }
    }

    pub fn cut_line(&mut self) {
        self.record_edit();
        if let Some((r1, c1, r2, c2)) = self.selection {
            self.clipboard = self.extract_selection(r1, c1, r2, c2);
            self.delete_selection_range(r1, c1, r2, c2);
            self.selection = None;
        } else {
            self.clipboard = self.buffer.line_as_str(self.cursor_row).to_string() + "\n";
            self.buffer.remove_line(self.cursor_row);
            if self.cursor_row >= self.buffer.num_lines() && self.cursor_row > 0 {
                self.cursor_row -= 1;
            }
            self.clamp_cursor();
        }
        self.close_completion();
    }

    pub fn copy_line(&mut self) {
        if let Some((r1, c1, r2, c2)) = self.selection {
            self.clipboard = self.extract_selection(r1, c1, r2, c2);
        } else {
            self.clipboard = self.buffer.line_as_str(self.cursor_row).to_string() + "\n";
        }
    }

    pub fn paste(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        self.record_edit();
        if let Some((r1, c1, r2, c2)) = self.selection.take() {
            self.delete_selection_range(r1, c1, r2, c2);
        }

        let before = self.buffer.lines[self.cursor_row][..self.cursor_col].to_string();
        let after = self.buffer.lines[self.cursor_row][self.cursor_col..].to_string();
        let parts: Vec<&str> = self.clipboard.split('\n').collect();

        if parts.len() == 1 {
            self.buffer.lines[self.cursor_row] = before + parts[0] + &after;
            self.cursor_col += parts[0].len();
        } else {
            self.buffer.lines[self.cursor_row] = before + parts[0];

            for part in &parts[1..parts.len() - 1] {
                self.cursor_row += 1;
                self.buffer
                    .lines
                    .insert(self.cursor_row, (*part).to_string());
            }

            let last = parts[parts.len() - 1];
            self.cursor_row += 1;
            self.buffer
                .lines
                .insert(self.cursor_row, last.to_string() + &after);
            self.cursor_col = last.len();
        }

        self.buffer.modified = true;
        self.close_completion();
    }

    pub fn apply_ai_edit(&mut self, content: String) {
        self.record_edit();
        let mut lines: Vec<String> = content.lines().map(|line| line.to_string()).collect();
        if content.ends_with('\n') {
            lines.push(String::new());
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        self.buffer.lines = lines;
        self.buffer.modified = true;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.selection = None;
        self.diagnostics.clear();
        self.active_diagnostic = None;
        self.close_completion();
        self.search_query.clear();
        self.search_matches.clear();
        self.active_search = None;
        self.pending_ai_apply = false;
        self.pending_ai_edit_content = None;
        self.message = "AI edit applied. Ctrl+Z to undo, Ctrl+S to save.".to_string();
    }

    fn extract_selection(&self, r1: usize, c1: usize, r2: usize, c2: usize) -> String {
        let mut result = String::new();
        for r in r1..=r2 {
            let line = self.buffer.line_as_str(r);
            if r == r1 && r == r2 {
                result.push_str(&line[c1..c2]);
            } else if r == r1 {
                result.push_str(&line[c1..]);
                result.push('\n');
            } else if r == r2 {
                result.push_str(&line[..c2]);
            } else {
                result.push_str(line);
                result.push('\n');
            }
        }
        result
    }

    fn delete_selection_range(&mut self, r1: usize, c1: usize, r2: usize, c2: usize) {
        if r1 == r2 && c1 == c2 {
            return;
        }
        for r in (r1..=r2).rev() {
            if r == r1 && r == r2 {
                self.buffer.lines[r1].drain(c1..c2);
            } else if r == r1 {
                let rest = self.buffer.lines[r1].split_off(c1);
                let _ = rest;
            } else if r == r2 {
                self.buffer.lines[r2].drain(..c2);
            } else {
                self.buffer.lines.remove(r);
            }
        }
        if r1 < r2 && r1 + 1 < self.buffer.lines.len() {
            let tail = self.buffer.lines.remove(r1 + 1);
            self.buffer.lines[r1].push_str(&tail);
        }
        self.cursor_row = r1;
        self.cursor_col = c1;
        self.buffer.modified = true;
    }

    pub fn save(&mut self) -> bool {
        self.confirm_quit = false;
        if self.buffer.filepath.is_none() {
            self.prompt = Some(Prompt::new("Save as:"));
            return false;
        }
        if self.save_without_check() {
            self.auto_check_after_save();
            true
        } else {
            false
        }
    }

    fn save_without_check(&mut self) -> bool {
        match self.buffer.save() {
            Ok(()) => {
                self.message = format!("Saved {}", self.buffer.filename());
                true
            }
            Err(e) => {
                self.message = format!("Save error: {}", e);
                false
            }
        }
    }

    fn auto_check_after_save(&mut self) {
        if !is_checkable_extension(self.buffer.extension()) {
            return;
        }
        self.do_compile_internal(false);
    }

    pub fn do_compile(&mut self) {
        self.do_compile_internal(true);
    }

    fn do_compile_internal(&mut self, show_success_output: bool) {
        let path = self.buffer.filepath.clone().unwrap_or_default();
        if path.is_empty() {
            self.message = "Save the file first".to_string();
            return;
        }
        let ext = self.buffer.extension().to_string();
        let (compiler, hint) = select_compiler(&self.compiler_info, &ext);
        match compiler {
            None => {
                let msg = hint.unwrap_or_else(|| {
                    self.compiler_info
                        .problem
                        .clone()
                        .unwrap_or("No compiler found".into())
                });
                self.message = msg.clone();
                self.build_result = Some(BuildResult {
                    success: false,
                    output: msg,
                    command_line: String::new(),
                });
                self.output_scroll = 0;
                self.show_output = true;
            }
            Some(cc) => {
                self.save_without_check();
                self.message = if ext == "py" || ext == "pyw" {
                    "Checking Python...".to_string()
                } else {
                    "Compiling...".to_string()
                };
                match compile(&path, &cc) {
                    Ok(result) => {
                        self.apply_build_result(result, show_success_output);
                        if let Some(ref r) = self.build_result {
                            self.message = if r.success {
                                if ext == "py" || ext == "pyw" {
                                    "Python check passed".into()
                                } else {
                                    "Build succeeded".into()
                                }
                            } else {
                                if ext == "py" || ext == "pyw" {
                                    "Python check failed".into()
                                } else {
                                    "Build failed".into()
                                }
                            };
                            if !r.success && !self.diagnostics.is_empty() {
                                self.message = format!(
                                    "{} ({} issue(s))",
                                    self.message,
                                    self.diagnostics.len()
                                );
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("Compile error: {}", e);
                        self.message = msg.clone();
                        self.build_result = Some(BuildResult {
                            success: false,
                            output: msg,
                            command_line: String::new(),
                        });
                        self.output_scroll = 0;
                        self.diagnostics.clear();
                        self.active_diagnostic = None;
                        self.show_output = true;
                    }
                }
            }
        }
    }

    pub fn do_compile_run(&mut self) {
        let path = self.buffer.filepath.clone().unwrap_or_default();
        if path.is_empty() {
            self.message = "Save the file first".to_string();
            return;
        }
        let ext = self.buffer.extension().to_string();
        let (compiler, hint) = select_compiler(&self.compiler_info, &ext);
        match compiler {
            None => {
                let msg = hint.unwrap_or_else(|| {
                    self.compiler_info
                        .problem
                        .clone()
                        .unwrap_or("No compiler found".into())
                });
                self.message = msg.clone();
                self.build_result = Some(BuildResult {
                    success: false,
                    output: msg,
                    command_line: String::new(),
                });
                self.output_scroll = 0;
                self.show_output = true;
            }
            Some(cc) => {
                self.save_without_check();
                self.message = if ext == "py" || ext == "pyw" {
                    "Checking & running Python...".to_string()
                } else {
                    "Compiling & running...".to_string()
                };
                match compile_and_prepare_run(&path, &cc) {
                    Ok((result, run)) => {
                        if result.success {
                            self.pending_run = run;
                            self.show_output = false;
                            self.message = "Running in terminal...".into();
                            self.build_result = Some(result);
                            self.output_scroll = 0;
                            self.diagnostics.clear();
                            self.active_diagnostic = None;
                        } else {
                            self.apply_build_result(result, true);
                            if let Some(ref r) = self.build_result {
                                self.message = if r.success {
                                    "Ready to run".into()
                                } else {
                                    "Build/check failed".into()
                                };
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("Error: {}", e);
                        self.message = msg.clone();
                        self.build_result = Some(BuildResult {
                            success: false,
                            output: msg,
                            command_line: String::new(),
                        });
                        self.output_scroll = 0;
                        self.diagnostics.clear();
                        self.active_diagnostic = None;
                        self.show_output = true;
                    }
                }
            }
        }
    }

    fn apply_build_result(&mut self, result: BuildResult, show_success_output: bool) {
        self.diagnostics = parse_diagnostics(&result.output);
        self.active_diagnostic = if self.diagnostics.is_empty() {
            None
        } else {
            Some(0)
        };
        let success = result.success;
        self.build_result = Some(result);
        self.output_scroll = 0;
        self.show_output = !success || show_success_output;
        if self.active_diagnostic.is_some() {
            self.goto_active_diagnostic();
        }
    }

    pub fn next_diagnostic(&mut self) {
        if self.diagnostics.is_empty() {
            self.message = "No diagnostics".to_string();
            return;
        }
        let next = self
            .active_diagnostic
            .map(|idx| (idx + 1) % self.diagnostics.len())
            .unwrap_or(0);
        self.active_diagnostic = Some(next);
        self.goto_active_diagnostic();
    }

    pub fn previous_diagnostic(&mut self) {
        if self.diagnostics.is_empty() {
            self.message = "No diagnostics".to_string();
            return;
        }
        let prev = self
            .active_diagnostic
            .map(|idx| {
                if idx == 0 {
                    self.diagnostics.len() - 1
                } else {
                    idx - 1
                }
            })
            .unwrap_or(0);
        self.active_diagnostic = Some(prev);
        self.goto_active_diagnostic();
    }

    pub fn goto_diagnostic(&mut self, idx: usize) {
        if idx < self.diagnostics.len() {
            self.active_diagnostic = Some(idx);
            self.goto_active_diagnostic();
        }
    }

    fn goto_active_diagnostic(&mut self) {
        let Some(idx) = self.active_diagnostic else {
            return;
        };
        let Some(diagnostic) = self.diagnostics.get(idx) else {
            return;
        };
        self.cursor_row = diagnostic
            .row
            .min(self.buffer.num_lines().saturating_sub(1));
        self.cursor_col =
            clamp_to_char_boundary(self.buffer.line_as_str(self.cursor_row), diagnostic.col);
        self.show_output = true;
        self.message = format!(
            "{} {}/{}: {}",
            match diagnostic.severity {
                DiagnosticSeverity::Error => "Error",
                DiagnosticSeverity::Warning => "Warning",
            },
            idx + 1,
            self.diagnostics.len(),
            diagnostic.message
        );
    }

    pub fn accept_completion(&mut self) -> bool {
        let Some(item) = self.completions.get(self.completion_selected).cloned() else {
            return false;
        };
        let start = self.completion_start_col.min(self.cursor_col);
        let row = self.cursor_row;
        self.record_edit();
        self.buffer.lines[row].replace_range(start..self.cursor_col, &item.label);
        self.cursor_col = start + item.label.len();
        self.buffer.modified = true;
        self.close_completion();
        true
    }

    pub fn select_next_completion(&mut self) -> bool {
        if self.completions.is_empty() {
            return false;
        }
        self.completion_selected = (self.completion_selected + 1) % self.completions.len();
        true
    }

    pub fn select_previous_completion(&mut self) -> bool {
        if self.completions.is_empty() {
            return false;
        }
        self.completion_selected = if self.completion_selected == 0 {
            self.completions.len() - 1
        } else {
            self.completion_selected - 1
        };
        true
    }

    pub fn close_completion(&mut self) {
        self.completions.clear();
        self.completion_selected = 0;
        self.completion_prefix.clear();
        self.completion_start_col = self.cursor_col;
    }

    pub fn refresh_completions(&mut self) {
        let (prefix, start_col) =
            current_prefix(self.buffer.line_as_str(self.cursor_row), self.cursor_col);
        self.completion_prefix = prefix.clone();
        self.completion_start_col = start_col;
        if prefix.len() < 2 {
            self.completions.clear();
            self.completion_selected = 0;
            return;
        }
        self.completions = completion_labels(&self.buffer.lines, self.buffer.extension())
            .into_iter()
            .filter(|label| label.starts_with(&prefix) && label != &prefix)
            .take(8)
            .map(|label| CompletionItem { label })
            .collect();
        self.completion_selected = 0;
    }

    pub fn trigger_completion(&mut self) {
        self.refresh_completions();
        if self.completions.is_empty() {
            self.message = "No completions".to_string();
        }
    }

    pub fn set_search(&mut self, query: String) {
        self.search_query = query;
        self.search_matches.clear();
        self.active_search = None;
        if self.search_query.is_empty() {
            self.message = "Search cleared".to_string();
            return;
        }
        self.search_matches = find_literal_matches(&self.buffer.lines, &self.search_query);
        if self.search_matches.is_empty() {
            self.message = format!("No matches: {}", self.search_query);
            return;
        }
        self.active_search = Some(0);
        self.goto_active_search();
    }

    pub fn next_search(&mut self) {
        if self.search_matches.is_empty() {
            if self.search_query.is_empty() {
                self.message = "No search query".to_string();
            } else {
                self.set_search(self.search_query.clone());
            }
            return;
        }
        let next = self
            .active_search
            .map(|idx| (idx + 1) % self.search_matches.len())
            .unwrap_or(0);
        self.active_search = Some(next);
        self.goto_active_search();
    }

    pub fn previous_search(&mut self) {
        if self.search_matches.is_empty() {
            if self.search_query.is_empty() {
                self.message = "No search query".to_string();
            } else {
                self.set_search(self.search_query.clone());
            }
            return;
        }
        let prev = self
            .active_search
            .map(|idx| {
                if idx == 0 {
                    self.search_matches.len() - 1
                } else {
                    idx - 1
                }
            })
            .unwrap_or(0);
        self.active_search = Some(prev);
        self.goto_active_search();
    }

    fn goto_active_search(&mut self) {
        let Some(idx) = self.active_search else {
            return;
        };
        let Some(m) = self.search_matches.get(idx) else {
            return;
        };
        self.cursor_row = m.row;
        self.cursor_col = m.start_col;
        self.message = format!(
            "Search {}/{}: {}",
            idx + 1,
            self.search_matches.len(),
            self.search_query
        );
    }

    pub fn bracket_matches(&self) -> Vec<TextMatch> {
        bracket_pair_at(&self.buffer.lines, self.cursor_row, self.cursor_col)
            .into_iter()
            .flat_map(|pair| pair.into_iter())
            .map(|(row, col)| TextMatch {
                row,
                start_col: col,
                end_col: col + 1,
            })
            .collect()
    }

    pub fn bracket_color_marks(&self) -> Vec<BracketColorMark> {
        bracket_color_marks(&self.buffer.lines)
    }

    pub fn should_ignore_key(&mut self, key: &KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return true;
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_key_time).as_millis();
        let same_key = self.last_key_code.as_ref() == Some(&key.code)
            && self.last_key_modifiers == key.modifiers;
        let duplicate_press = cfg!(target_os = "windows")
            && key.kind == KeyEventKind::Press
            && same_key
            && elapsed < 90;

        if duplicate_press {
            return true;
        }

        self.last_key_time = now;
        self.last_key_code = Some(key.code);
        self.last_key_modifiers = key.modifiers;
        false
    }

    pub fn scroll_offset(&self, view_height: usize) -> usize {
        self.cursor_row.saturating_sub(view_height / 2)
    }
}

fn prev_char_boundary(line: &str, col: usize) -> Option<usize> {
    if col == 0 || col > line.len() || !line.is_char_boundary(col) {
        return None;
    }
    line[..col].char_indices().last().map(|(idx, _)| idx)
}

fn next_char_boundary(line: &str, col: usize) -> Option<usize> {
    if col > line.len() || !line.is_char_boundary(col) {
        return None;
    }
    line[col..].chars().next().map(|ch| col + ch.len_utf8())
}

fn clamp_to_char_boundary(line: &str, col: usize) -> usize {
    let mut col = col.min(line.len());
    while col > 0 && !line.is_char_boundary(col) {
        col -= 1;
    }
    col
}

fn leading_whitespace(line: &str) -> &str {
    let end = line
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace() || *ch == '\n' || *ch == '\r')
        .map(|(idx, _)| idx)
        .unwrap_or(line.len());
    &line[..end]
}

fn should_increase_indent(before_cursor: &str, extension: &str) -> bool {
    let trimmed = before_cursor.trim_end();
    if trimmed.is_empty() || is_line_comment(trimmed) {
        return false;
    }

    if matches!(extension, "py" | "pyw") && trimmed.ends_with(':') {
        return true;
    }

    trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[')
}

fn starts_with_closing_token(after_cursor: &str) -> bool {
    matches!(
        after_cursor.trim_start().chars().next(),
        Some('}' | ')' | ']')
    )
}

fn is_line_comment(trimmed: &str) -> bool {
    trimmed.starts_with("//") || trimmed.starts_with('#')
}

fn is_checkable_extension(ext: &str) -> bool {
    matches!(
        ext,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" | "py" | "pyw"
    )
}

pub fn parse_diagnostics(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut pending_python: Option<(usize, usize)> = None;

    for line in output.lines() {
        if let Some((row, col)) = parse_python_file_line(line) {
            pending_python = Some((row, col));
            continue;
        }

        if let Some(diagnostic) = parse_gcc_style_diagnostic(line) {
            diagnostics.push(diagnostic);
            continue;
        }

        if let Some((row, col)) = pending_python {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with("File ")
                || trimmed.starts_with('^')
                || !trimmed.contains("Error")
            {
                continue;
            }
            diagnostics.push(Diagnostic {
                row,
                col,
                severity: DiagnosticSeverity::Error,
                message: trimmed.to_string(),
            });
            pending_python = None;
        }
    }

    diagnostics
}

fn parse_gcc_style_diagnostic(line: &str) -> Option<Diagnostic> {
    let parts: Vec<&str> = line.splitn(5, ':').collect();
    if parts.len() < 4 {
        return None;
    }

    let line_no = parts.get(1)?.trim().parse::<usize>().ok()?;
    let (col_no, severity_part, message_part) =
        if let Ok(col) = parts.get(2)?.trim().parse::<usize>() {
            (col, *parts.get(3)?, parts.get(4).copied().unwrap_or(""))
        } else {
            (1, *parts.get(2)?, parts.get(3).copied().unwrap_or(""))
        };

    let severity_text = severity_part.trim().to_ascii_lowercase();
    let severity = if severity_text.contains("warning") {
        DiagnosticSeverity::Warning
    } else if severity_text.contains("error") || severity_text.contains("fatal") {
        DiagnosticSeverity::Error
    } else {
        return None;
    };

    Some(Diagnostic {
        row: line_no.saturating_sub(1),
        col: col_no.saturating_sub(1),
        severity,
        message: message_part.trim().to_string(),
    })
}

fn parse_python_file_line(line: &str) -> Option<(usize, usize)> {
    let trimmed = line.trim();
    if !trimmed.starts_with("File ") {
        return None;
    }
    let line_marker = ", line ";
    let line_start = trimmed.find(line_marker)? + line_marker.len();
    let rest = &trimmed[line_start..];
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    let row = digits.parse::<usize>().ok()?.saturating_sub(1);
    Some((row, 0))
}

fn current_prefix(line: &str, cursor_col: usize) -> (String, usize) {
    let col = clamp_to_char_boundary(line, cursor_col);
    let mut start = col;
    while let Some(prev) = prev_char_boundary(line, start) {
        let ch = line[prev..start].chars().next().unwrap_or_default();
        if !is_ident_continue(ch) {
            break;
        }
        start = prev;
    }
    (line[start..col].to_string(), start)
}

fn completion_labels(lines: &[String], extension: &str) -> Vec<String> {
    let mut labels = BTreeSet::new();
    for keyword in language_keywords(extension) {
        labels.insert(keyword.to_string());
    }
    for line in lines {
        for word in identifiers_in_line(line) {
            if word.len() >= 2 {
                labels.insert(word);
            }
        }
    }
    labels.into_iter().collect()
}

fn language_keywords(extension: &str) -> &'static [&'static str] {
    match extension {
        "py" | "pyw" => &[
            "and", "as", "break", "class", "continue", "def", "elif", "else", "except", "False",
            "for", "from", "if", "import", "in", "input", "len", "None", "print", "range",
            "return", "self", "True", "while", "with",
        ],
        "c" | "h" => &[
            "break", "case", "char", "const", "continue", "default", "double", "else", "enum",
            "float", "for", "if", "include", "int", "long", "printf", "return", "scanf", "short",
            "sizeof", "static", "struct", "switch", "typedef", "void", "while",
        ],
        _ => &[
            "auto",
            "bool",
            "break",
            "case",
            "char",
            "class",
            "const",
            "continue",
            "cout",
            "cin",
            "double",
            "else",
            "endl",
            "false",
            "float",
            "for",
            "getline",
            "if",
            "include",
            "int",
            "long",
            "namespace",
            "private",
            "public",
            "return",
            "short",
            "sizeof",
            "static",
            "string",
            "struct",
            "template",
            "true",
            "using",
            "vector",
            "void",
            "while",
        ],
    }
}

fn identifiers_in_line(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = None;
    for (idx, ch) in line.char_indices() {
        if start.is_none() && is_ident_start(ch) {
            start = Some(idx);
        } else if start.is_some() && !is_ident_continue(ch) {
            let s = start.take().unwrap();
            result.push(line[s..idx].to_string());
        }
    }
    if let Some(s) = start {
        result.push(line[s..].to_string());
    }
    result
}

fn find_literal_matches(lines: &[String], query: &str) -> Vec<TextMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        let mut offset = 0;
        while let Some(found) = line[offset..].find(query) {
            let start = offset + found;
            let end = start + query.len();
            matches.push(TextMatch {
                row,
                start_col: start,
                end_col: end,
            });
            offset = end.max(start + 1);
        }
    }
    matches
}

fn bracket_color_marks(lines: &[String]) -> Vec<BracketColorMark> {
    let mut marks = Vec::new();
    let mut stack: Vec<(char, usize, usize, usize)> = Vec::new();

    for (row, line) in lines.iter().enumerate() {
        for (col, ch) in line.char_indices() {
            if matches!(ch, '(' | '[' | '{') {
                let color_index = stack.len() + bracket_kind_index(ch);
                stack.push((ch, row, col, color_index));
            } else if matches!(ch, ')' | ']' | '}') {
                let Some(open) = matching_open_bracket(ch) else {
                    continue;
                };
                if let Some(pos) = stack
                    .iter()
                    .rposition(|(open_ch, _, _, _)| *open_ch == open)
                {
                    let (_, open_row, open_col, color_index) = stack.remove(pos);
                    marks.push(BracketColorMark {
                        row: open_row,
                        col: open_col,
                        color_index,
                    });
                    marks.push(BracketColorMark {
                        row,
                        col,
                        color_index,
                    });
                }
            }
        }
    }

    marks.sort_by_key(|mark| (mark.row, mark.col));
    marks
}

fn bracket_pair_at(lines: &[String], row: usize, col: usize) -> Option<[(usize, usize); 2]> {
    let candidates = bracket_candidates(lines, row, col);
    for (bracket_row, bracket_col, ch) in candidates {
        if let Some(pair) = find_matching_bracket(lines, bracket_row, bracket_col, ch) {
            return Some([(bracket_row, bracket_col), pair]);
        }
    }
    None
}

fn bracket_candidates(lines: &[String], row: usize, col: usize) -> Vec<(usize, usize, char)> {
    let mut candidates = Vec::new();
    let Some(line) = lines.get(row) else {
        return candidates;
    };
    let col = clamp_to_char_boundary(line, col);
    if let Some(ch) = line[col..].chars().next() {
        if is_bracket(ch) {
            candidates.push((row, col, ch));
        }
    }
    if let Some(prev) = prev_char_boundary(line, col) {
        let ch = line[prev..col].chars().next().unwrap_or_default();
        if is_bracket(ch) {
            candidates.push((row, prev, ch));
        }
    }
    candidates
}

fn find_matching_bracket(
    lines: &[String],
    row: usize,
    col: usize,
    bracket: char,
) -> Option<(usize, usize)> {
    let (open, close, forward) = match bracket {
        '(' => ('(', ')', true),
        '[' => ('[', ']', true),
        '{' => ('{', '}', true),
        ')' => ('(', ')', false),
        ']' => ('[', ']', false),
        '}' => ('{', '}', false),
        _ => return None,
    };
    if forward {
        let mut depth = 0usize;
        for (r, line) in lines.iter().enumerate().skip(row) {
            for (c, ch) in line.char_indices() {
                if r == row && c < col {
                    continue;
                }
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some((r, c));
                    }
                }
            }
        }
    } else {
        let mut depth = 0usize;
        for r in (0..=row).rev() {
            let line = &lines[r];
            for (c, ch) in line.char_indices().rev() {
                if r == row && c > col {
                    continue;
                }
                if ch == close {
                    depth += 1;
                } else if ch == open {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some((r, c));
                    }
                }
            }
        }
    }
    None
}

fn matching_open_bracket(ch: char) -> Option<char> {
    match ch {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

fn bracket_kind_index(ch: char) -> usize {
    match ch {
        '(' | ')' => 0,
        '[' | ']' => 1,
        '{' | '}' => 2,
        _ => 0,
    }
}

fn is_bracket(ch: char) -> bool {
    matches!(ch, '(' | ')' | '[' | ']' | '{' | '}')
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("tinyvim-test-{}-{}", std::process::id(), stamp));
            std::fs::create_dir(&path).unwrap();
            TestDir { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn select_entry(dialog: &mut FileDialog, name: &str) {
        dialog.selected = dialog
            .entries
            .iter()
            .position(|entry| entry.name == name)
            .expect("entry should exist");
    }

    #[test]
    fn paste_inserts_single_line_at_cursor() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["hello world".to_string()],
            filepath: None,
            modified: false,
        });
        editor.cursor_col = 5;
        editor.clipboard = ", tiny".to_string();

        editor.paste();

        assert_eq!(editor.buffer.lines, vec!["hello, tiny world"]);
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 11);
        assert!(editor.buffer.modified);
    }

    #[test]
    fn paste_inserts_multiline_text_and_preserves_suffix() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["hello world".to_string()],
            filepath: None,
            modified: false,
        });
        editor.cursor_col = 6;
        editor.clipboard = "tiny\nvim".to_string();

        editor.paste();

        assert_eq!(editor.buffer.lines, vec!["hello tiny", "vimworld"]);
        assert_eq!(editor.cursor_row, 1);
        assert_eq!(editor.cursor_col, 3);
        assert!(editor.buffer.modified);
    }

    #[test]
    fn paste_line_clipboard_inserts_new_line_before_suffix() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["hello world".to_string()],
            filepath: None,
            modified: false,
        });
        editor.cursor_col = 6;
        editor.clipboard = "tiny\n".to_string();

        editor.paste();

        assert_eq!(editor.buffer.lines, vec!["hello tiny", "world"]);
        assert_eq!(editor.cursor_row, 1);
        assert_eq!(editor.cursor_col, 0);
        assert!(editor.buffer.modified);
    }

    #[test]
    fn delete_removes_selected_range_at_once_and_undo_restores_it() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["one".to_string(), "two".to_string(), "three".to_string()],
            filepath: None,
            modified: false,
        });
        editor.selection = Some((0, 0, 2, 5));
        editor.cursor_row = 2;
        editor.cursor_col = 5;

        editor.delete();

        assert_eq!(editor.buffer.lines, vec![""]);
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 0);
        assert!(editor.selection.is_none());

        editor.undo();

        assert_eq!(editor.buffer.lines, vec!["one", "two", "three"]);
        assert_eq!(editor.selection, Some((0, 0, 2, 5)));
    }

    #[test]
    fn typing_replaces_selected_range() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["hello world".to_string()],
            filepath: None,
            modified: false,
        });
        editor.selection = Some((0, 0, 0, 5));
        editor.cursor_row = 0;
        editor.cursor_col = 5;

        editor.insert_char('H');

        assert_eq!(editor.buffer.lines, vec!["H world"]);
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 1);
        assert!(editor.selection.is_none());
    }

    #[test]
    fn paste_replaces_selected_range() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["hello world".to_string()],
            filepath: None,
            modified: false,
        });
        editor.selection = Some((0, 6, 0, 11));
        editor.cursor_row = 0;
        editor.cursor_col = 11;
        editor.clipboard = "tiny".to_string();

        editor.paste();

        assert_eq!(editor.buffer.lines, vec!["hello tiny"]);
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 10);
        assert!(editor.selection.is_none());
    }

    #[test]
    fn unicode_insert_backspace_and_delete_stay_on_char_boundaries() {
        let mut editor = Editor::new(Buffer::new());

        editor.insert_char('你');
        editor.insert_char('好');
        editor.insert_char('a');

        assert_eq!(editor.buffer.lines, vec!["你好a"]);
        assert_eq!(editor.cursor_col, "你好a".len());
        assert_eq!(editor.buffer.line_char_count(0, editor.cursor_col), 3);

        editor.backspace();
        assert_eq!(editor.buffer.lines, vec!["你好"]);
        assert_eq!(editor.cursor_col, "你好".len());

        editor.move_left();
        assert_eq!(editor.cursor_col, "你".len());

        editor.delete();
        assert_eq!(editor.buffer.lines, vec!["你"]);
        assert_eq!(editor.cursor_col, "你".len());
    }

    #[test]
    fn undo_reverts_inserted_text_cursor_and_modified_state() {
        let mut editor = Editor::new(Buffer::new());

        editor.insert_char('a');
        editor.insert_char('b');

        assert_eq!(editor.buffer.lines, vec!["ab"]);
        assert!(editor.buffer.modified);

        editor.undo();

        assert_eq!(editor.buffer.lines, vec!["a"]);
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 1);
        assert!(editor.buffer.modified);

        editor.undo();

        assert_eq!(editor.buffer.lines, vec![""]);
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 0);
        assert!(!editor.buffer.modified);
    }

    #[test]
    fn undo_reverts_backspace() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["hello".to_string()],
            filepath: None,
            modified: false,
        });
        editor.cursor_col = 5;

        editor.backspace();

        assert_eq!(editor.buffer.lines, vec!["hell"]);
        assert_eq!(editor.cursor_col, 4);

        editor.undo();

        assert_eq!(editor.buffer.lines, vec!["hello"]);
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 5);
        assert!(!editor.buffer.modified);
    }

    #[test]
    fn undo_with_empty_history_keeps_buffer_unchanged() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["stable".to_string()],
            filepath: None,
            modified: false,
        });
        editor.cursor_col = 3;

        editor.undo();

        assert_eq!(editor.buffer.lines, vec!["stable"]);
        assert_eq!(editor.cursor_col, 3);
        assert_eq!(editor.message, "Nothing to undo");
        assert!(!editor.buffer.modified);
    }

    #[test]
    fn parses_gcc_style_diagnostics() {
        let diagnostics = parse_diagnostics("main.cpp:12:7: error: expected ';'\n");

        assert_eq!(
            diagnostics,
            vec![Diagnostic {
                row: 11,
                col: 6,
                severity: DiagnosticSeverity::Error,
                message: "expected ';'".to_string(),
            }]
        );
    }

    #[test]
    fn parses_python_traceback_diagnostics() {
        let diagnostics = parse_diagnostics(
            "  File \"main.py\", line 3\n    print(\nSyntaxError: '(' was never closed\n",
        );

        assert_eq!(
            diagnostics,
            vec![Diagnostic {
                row: 2,
                col: 0,
                severity: DiagnosticSeverity::Error,
                message: "SyntaxError: '(' was never closed".to_string(),
            }]
        );
    }

    #[test]
    fn completion_accepts_current_file_identifier() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["studentName = 1".to_string(), "stu".to_string()],
            filepath: Some("main.py".to_string()),
            modified: false,
        });
        editor.cursor_row = 1;
        editor.cursor_col = 3;

        editor.refresh_completions();
        assert!(editor
            .completions
            .iter()
            .any(|item| item.label == "studentName"));
        assert!(editor.accept_completion());

        assert_eq!(editor.buffer.lines[1], "studentName");
        assert_eq!(editor.cursor_col, "studentName".len());
    }

    #[test]
    fn search_collects_literal_matches_and_jumps() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["alpha beta".to_string(), "beta gamma".to_string()],
            filepath: None,
            modified: false,
        });

        editor.set_search("beta".to_string());

        assert_eq!(editor.search_matches.len(), 2);
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 6);

        editor.next_search();

        assert_eq!(editor.cursor_row, 1);
        assert_eq!(editor.cursor_col, 0);
    }

    #[test]
    fn bracket_matches_find_pair_around_cursor() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["if (ok) {".to_string(), "}".to_string()],
            filepath: Some("main.cpp".to_string()),
            modified: false,
        });
        editor.cursor_col = 3;

        let matches = editor.bracket_matches();

        assert_eq!(
            matches,
            vec![
                TextMatch {
                    row: 0,
                    start_col: 3,
                    end_col: 4,
                },
                TextMatch {
                    row: 0,
                    start_col: 6,
                    end_col: 7,
                },
            ]
        );
    }

    #[test]
    fn bracket_matches_find_multiline_braces() {
        let mut editor = Editor::new(Buffer {
            lines: vec![
                "int main() {".to_string(),
                "    return 0;".to_string(),
                "}".to_string(),
            ],
            filepath: Some("main.cpp".to_string()),
            modified: false,
        });
        editor.cursor_row = 0;
        editor.cursor_col = 11;

        let matches = editor.bracket_matches();

        assert_eq!(
            matches,
            vec![
                TextMatch {
                    row: 0,
                    start_col: 11,
                    end_col: 12,
                },
                TextMatch {
                    row: 2,
                    start_col: 0,
                    end_col: 1,
                },
            ]
        );
    }

    #[test]
    fn bracket_color_marks_pair_matching_braces() {
        let editor = Editor::new(Buffer {
            lines: vec![
                "int main() {".to_string(),
                "    return 0;".to_string(),
                "}".to_string(),
            ],
            filepath: Some("main.cpp".to_string()),
            modified: false,
        });

        let marks = editor.bracket_color_marks();

        assert_eq!(
            marks,
            vec![
                BracketColorMark {
                    row: 0,
                    col: 8,
                    color_index: 0,
                },
                BracketColorMark {
                    row: 0,
                    col: 9,
                    color_index: 0,
                },
                BracketColorMark {
                    row: 0,
                    col: 11,
                    color_index: 2,
                },
                BracketColorMark {
                    row: 2,
                    col: 0,
                    color_index: 2,
                },
            ]
        );
    }

    #[test]
    fn clamp_cursor_moves_invalid_utf8_offset_back_to_boundary() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["你a".to_string()],
            filepath: None,
            modified: false,
        });
        editor.cursor_col = 2;

        editor.clamp_cursor();

        assert_eq!(editor.cursor_col, 0);
    }

    #[test]
    fn enter_preserves_existing_indent() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["    let value = 1;".to_string()],
            filepath: Some("main.cpp".to_string()),
            modified: false,
        });
        editor.cursor_col = editor.buffer.line_len(0);

        editor.insert_char('\n');

        assert_eq!(editor.buffer.lines, vec!["    let value = 1;", "    "]);
        assert_eq!(editor.cursor_row, 1);
        assert_eq!(editor.cursor_col, 4);
    }

    #[test]
    fn enter_indents_after_cpp_block_start() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["if (ok) {".to_string()],
            filepath: Some("main.cpp".to_string()),
            modified: false,
        });
        editor.cursor_col = editor.buffer.line_len(0);

        editor.insert_char('\n');

        assert_eq!(editor.buffer.lines, vec!["if (ok) {", "    "]);
        assert_eq!(editor.cursor_col, 4);
    }

    #[test]
    fn enter_indents_after_python_colon() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["if ok:".to_string()],
            filepath: Some("main.py".to_string()),
            modified: false,
        });
        editor.cursor_col = editor.buffer.line_len(0);

        editor.insert_char('\n');

        assert_eq!(editor.buffer.lines, vec!["if ok:", "    "]);
        assert_eq!(editor.cursor_col, 4);
    }

    #[test]
    fn enter_before_closing_brace_dedents_new_line() {
        let mut editor = Editor::new(Buffer {
            lines: vec!["    }".to_string()],
            filepath: Some("main.cpp".to_string()),
            modified: false,
        });
        editor.cursor_col = 4;

        editor.insert_char('\n');

        assert_eq!(editor.buffer.lines, vec!["    ", "}"]);
        assert_eq!(editor.cursor_row, 1);
        assert_eq!(editor.cursor_col, 0);
    }

    #[test]
    fn file_dialog_creates_renames_and_deletes_file() {
        let dir = TestDir::new();
        let mut dialog = FileDialog::new(dir.path.clone());

        dialog.create_file("main.c").unwrap();
        assert!(dir.path.join("main.c").is_file());

        dialog.rename_file("main.c", "app.c").unwrap();
        assert!(!dir.path.join("main.c").exists());
        assert!(dir.path.join("app.c").is_file());

        select_entry(&mut dialog, "app.c");
        dialog.delete_selected().unwrap();
        assert!(!dir.path.join("app.c").exists());
    }

    #[test]
    fn file_dialog_creates_and_deletes_directory() {
        let dir = TestDir::new();
        let mut dialog = FileDialog::new(dir.path.clone());

        dialog.create_directory("src").unwrap();
        assert!(dir.path.join("src").is_dir());

        select_entry(&mut dialog, "src");
        dialog.delete_selected().unwrap();
        assert!(!dir.path.join("src").exists());
    }

    #[test]
    fn file_dialog_rejects_path_like_names() {
        let dir = TestDir::new();
        let mut dialog = FileDialog::new(dir.path.clone());

        assert!(dialog.create_file("../bad.c").is_err());
        assert!(dialog.create_directory("src/nested").is_err());
        assert!(dialog.rename_file("missing.c", "../bad.c").is_err());
    }
}
