use std::fs;
use std::io;
use std::path::Path;

pub struct Buffer {
    pub lines: Vec<String>,
    pub filepath: Option<String>,
    pub modified: bool,
}

impl Buffer {
    pub fn new() -> Self {
        Buffer {
            lines: vec![String::new()],
            filepath: None,
            modified: false,
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let content = fs::read_to_string(&path)?;
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let lines = if lines.is_empty() { vec![String::new()] } else { lines };
        Ok(Buffer {
            lines,
            filepath: Some(path.as_ref().to_string_lossy().to_string()),
            modified: false,
        })
    }

    pub fn save(&mut self) -> io::Result<()> {
        if let Some(ref path) = self.filepath.clone() {
            fs::write(path, self.lines.join("\n"))?;
            self.modified = false;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn save_as<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        fs::write(&path, self.lines.join("\n"))?;
        self.filepath = Some(path.as_ref().to_string_lossy().to_string());
        self.modified = false;
        Ok(())
    }

    pub fn line_len(&self, row: usize) -> usize {
        if row < self.lines.len() { self.lines[row].len() } else { 0 }
    }

    pub fn num_lines(&self) -> usize {
        self.lines.len()
    }

    pub fn line_as_str(&self, row: usize) -> &str {
        if row < self.lines.len() { &self.lines[row] } else { "" }
    }

    pub fn insert_char(&mut self, row: usize, col: usize, c: char) {
        if c == '\n' {
            let rest = self.lines[row].split_off(col);
            self.lines.insert(row + 1, rest);
        } else {
            self.lines[row].insert(col, c);
        }
        self.modified = true;
    }

    pub fn delete_char(&mut self, row: usize, col: usize) -> bool {
        if col < self.lines[row].len() {
            self.lines[row].remove(col);
            self.modified = true;
            true
        } else if row + 1 < self.lines.len() {
            let next = self.lines.remove(row + 1);
            self.lines[row].push_str(&next);
            self.modified = true;
            true
        } else {
            false
        }
    }

    pub fn backspace(&mut self, row: usize, col: usize) -> bool {
        if col > 0 {
            self.lines[row].remove(col - 1);
            self.modified = true;
            true
        } else if row > 0 {
            let cur = self.lines.remove(row);
            self.lines[row - 1].push_str(&cur);
            self.modified = true;
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub fn insert_line(&mut self, row: usize) {
        self.lines.insert(row, String::new());
        self.modified = true;
    }

    pub fn remove_line(&mut self, row: usize) -> String {
        if self.lines.len() <= 1 {
            let old = std::mem::take(&mut self.lines[0]);
            self.modified = true;
            return old;
        }
        let line = self.lines.remove(row);
        self.modified = true;
        line
    }

    pub fn filename(&self) -> &str {
        self.filepath
            .as_ref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("[No Name]")
    }

    pub fn extension(&self) -> &str {
        self.filepath
            .as_ref()
            .and_then(|p| std::path::Path::new(p).extension())
            .and_then(|e| e.to_str())
            .unwrap_or("")
    }
}
