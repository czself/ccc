use std::path::PathBuf;
use std::fs;

use crate::buffer::Buffer;
use crate::builder::{compile, compile_and_run, select_compiler, BuildResult, CompilerInfo, probe_compilers};

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
}

pub struct FileDialog {
    pub cwd: PathBuf,
    pub entries: Vec<FileEntry>,
    pub selected: usize,
    pub scroll: usize,
}

impl FileDialog {
    pub fn new(path: PathBuf) -> Self {
        let mut dlg = FileDialog {
            cwd: path,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
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
                    files.push(FileEntry { name, is_dir: false });
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
        let entry = &self.entries[self.selected];
        if entry.name == ".." {
            self.go_up();
        } else if entry.is_dir {
            self.cwd = self.cwd.join(&entry.name);
            self.refresh();
        }
    }
}

pub struct Prompt {
    pub label: String,
    pub input: String,
}

impl Prompt {
    pub fn new(label: &str) -> Self {
        Prompt {
            label: label.to_string(),
            input: String::new(),
        }
    }
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
    pub build_result: Option<BuildResult>,
    pub file_dialog: Option<FileDialog>,
    pub prompt: Option<Prompt>,
    pub pending_mkfile: Option<PathBuf>,
    pub pending_mkdir: Option<PathBuf>,
    pub compiler_info: CompilerInfo,
    pub confirm_quit: bool,
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
            build_result: None,
            file_dialog: None,
            prompt: None,
            pending_mkfile: None,
            pending_mkdir: None,
            compiler_info: probe_compilers(),
            confirm_quit: false,
        }
    }

    pub fn clamp_cursor(&mut self) {
        let max_row = self.buffer.num_lines().max(1) - 1;
        self.cursor_row = self.cursor_row.min(max_row);
        self.cursor_col = self.cursor_col.min(self.buffer.line_len(self.cursor_row));
    }

    pub fn move_up(&mut self) { if self.cursor_row > 0 { self.cursor_row -= 1; } self.clamp_cursor(); }
    pub fn move_down(&mut self) { if self.cursor_row + 1 < self.buffer.num_lines() { self.cursor_row += 1; } self.clamp_cursor(); }
    pub fn move_left(&mut self) {
        if self.cursor_col > 0 { self.cursor_col -= 1; }
        else if self.cursor_row > 0 { self.cursor_row -= 1; self.cursor_col = self.buffer.line_len(self.cursor_row); }
    }
    pub fn move_right(&mut self) {
        if self.cursor_col < self.buffer.line_len(self.cursor_row) { self.cursor_col += 1; }
        else if self.cursor_row + 1 < self.buffer.num_lines() { self.cursor_row += 1; self.cursor_col = 0; }
    }
    pub fn move_word_left(&mut self) {
        let line = self.buffer.line_as_str(self.cursor_row);
        let bytes = line.as_bytes();
        let mut pos = self.cursor_col.min(line.len());
        if pos == 0 { if self.cursor_row > 0 { self.cursor_row -= 1; self.cursor_col = self.buffer.line_len(self.cursor_row); } return; }
        pos = pos.saturating_sub(1);
        while pos > 0 && bytes[pos].is_ascii_whitespace() { pos -= 1; }
        let is_word = bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_';
        if is_word {
            while pos > 0 && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') { pos -= 1; }
            if !(bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') { pos += 1; }
        } else {
            while pos > 0 && !bytes[pos].is_ascii_alphanumeric() && bytes[pos] != b'_' && !bytes[pos].is_ascii_whitespace() { pos -= 1; }
            if bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_' || bytes[pos].is_ascii_whitespace() { pos += 1; }
        }
        self.cursor_col = pos;
    }
    pub fn move_word_right(&mut self) {
        let line = self.buffer.line_as_str(self.cursor_row);
        let bytes = line.as_bytes();
        let mut pos = self.cursor_col;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
        if pos >= bytes.len() { if self.cursor_row + 1 < self.buffer.num_lines() { self.cursor_row += 1; self.cursor_col = 0; } return; }
        let is_word = bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_';
        if is_word {
            while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') { pos += 1; }
        } else {
            while pos < bytes.len() && !bytes[pos].is_ascii_alphanumeric() && bytes[pos] != b'_' && !bytes[pos].is_ascii_whitespace() { pos += 1; }
        }
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
        self.cursor_col = if pos >= bytes.len() { bytes.len() } else { pos };
    }
    pub fn home(&mut self) { self.cursor_col = 0; }
    pub fn end(&mut self) { self.cursor_col = self.buffer.line_len(self.cursor_row); }
    pub fn page_up(&mut self) { self.cursor_row = self.cursor_row.saturating_sub(20); self.clamp_cursor(); }
    pub fn page_down(&mut self) { self.cursor_row = (self.cursor_row + 20).min(self.buffer.num_lines().saturating_sub(1)); self.clamp_cursor(); }

    pub fn insert_char(&mut self, c: char) {
        if c == '\n' {
            self.buffer.insert_char(self.cursor_row, self.cursor_col, '\n');
            self.cursor_row += 1;
            self.cursor_col = 0;
        } else if c == '\t' {
            for _ in 0..4 {
                self.buffer.insert_char(self.cursor_row, self.cursor_col, ' ');
                self.cursor_col += 1;
            }
        } else {
            self.buffer.insert_char(self.cursor_row, self.cursor_col, c);
            self.cursor_col += 1;
        }
    }

    pub fn backspace(&mut self) {
        let ok = self.buffer.backspace(self.cursor_row, self.cursor_col);
        if ok {
            if self.cursor_col > 0 { self.cursor_col -= 1; }
            else if self.cursor_row > 0 { self.cursor_row -= 1; self.cursor_col = self.buffer.line_len(self.cursor_row); }
        }
    }

    pub fn delete(&mut self) {
        self.buffer.delete_char(self.cursor_row, self.cursor_col);
        self.clamp_cursor();
    }

    pub fn cut_line(&mut self) {
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
    }

    pub fn copy_line(&mut self) {
        if let Some((r1, c1, r2, c2)) = self.selection {
            self.clipboard = self.extract_selection(r1, c1, r2, c2);
        } else {
            self.clipboard = self.buffer.line_as_str(self.cursor_row).to_string() + "\n";
        }
    }

    pub fn paste(&mut self) {
        if self.clipboard.is_empty() { return; }
        let has_newline = self.clipboard.ends_with('\n') || self.clipboard.contains('\n');
        if has_newline && self.clipboard.ends_with('\n') {
            let text = self.clipboard.trim_end_matches('\n');
            self.buffer.lines[self.cursor_row] = self.buffer.lines[self.cursor_row][..self.cursor_col].to_string()
                + text + &self.buffer.lines[self.cursor_row][self.cursor_col..];
            self.buffer.lines.insert(self.cursor_row + 1, String::new());
            self.cursor_row += 1;
            self.cursor_col = 0;
            self.buffer.modified = true;
        } else {
            for c in self.clipboard.chars() {
                self.buffer.insert_char(self.cursor_row, self.cursor_col, c);
                self.cursor_col += 1;
            }
        }
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
        match self.buffer.save() {
            Ok(()) => { self.message = format!("Saved {}", self.buffer.filename()); true }
            Err(e) => { self.message = format!("Save error: {}", e); false }
        }
    }

    pub fn do_compile(&mut self) {
        let path = self.buffer.filepath.clone().unwrap_or_default();
        if path.is_empty() {
            self.message = "Save the file first".to_string();
            return;
        }
        let ext = self.buffer.extension().to_string();
        let (compiler, hint) = select_compiler(&self.compiler_info, &ext);
        match compiler {
            None => {
                let msg = hint.unwrap_or_else(|| self.compiler_info.problem.clone().unwrap_or("No compiler found".into()));
                self.message = msg.clone();
                self.build_result = Some(BuildResult {
                    success: false,
                    output: msg,
                    command_line: String::new(),
                });
                self.show_output = true;
            }
            Some(cc) => {
                self.save();
                self.message = "Compiling...".to_string();
                match compile(&path, &cc) {
                    Ok(result) => {
                        self.show_output = true;
                        self.build_result = Some(result);
                        if let Some(ref r) = self.build_result {
                            self.message = if r.success { "Build succeeded".into() } else { "Build failed".into() };
                        }
                    }
                    Err(e) => { self.message = format!("Compile error: {}", e); }
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
                let msg = hint.unwrap_or_else(|| self.compiler_info.problem.clone().unwrap_or("No compiler found".into()));
                self.message = msg.clone();
                self.build_result = Some(BuildResult {
                    success: false,
                    output: msg,
                    command_line: String::new(),
                });
                self.show_output = true;
            }
            Some(cc) => {
                self.save();
                self.message = "Compiling & running...".to_string();
                match compile_and_run(&path, &cc) {
                    Ok(result) => {
                        self.show_output = true;
                        self.build_result = Some(result);
                        if let Some(ref r) = self.build_result {
                            self.message = if r.success { "Finished".into() } else { "Failed".into() };
                        }
                    }
                    Err(e) => { self.message = format!("Error: {}", e); }
                }
            }
        }
    }

    pub fn scroll_offset(&self, view_height: usize) -> usize {
        if self.cursor_row >= view_height / 2 {
            self.cursor_row - view_height / 2
        } else {
            0
        }
    }
}
