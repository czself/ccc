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
        let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        if content.ends_with('\n') {
            lines.push(String::new());
        }
        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
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
        if row < self.lines.len() {
            self.lines[row].len()
        } else {
            0
        }
    }

    pub fn line_char_count(&self, row: usize, col: usize) -> usize {
        self.line_as_str(row)
            .get(..col.min(self.line_len(row)))
            .unwrap_or("")
            .chars()
            .count()
    }

    pub fn num_lines(&self) -> usize {
        self.lines.len()
    }

    pub fn line_as_str(&self, row: usize) -> &str {
        if row < self.lines.len() {
            &self.lines[row]
        } else {
            ""
        }
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
            if !self.lines[row].is_char_boundary(col) {
                return false;
            }
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
            let Some(prev_col) = prev_char_boundary(&self.lines[row], col) else {
                return false;
            };
            self.lines[row].drain(prev_col..col);
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

fn prev_char_boundary(line: &str, col: usize) -> Option<usize> {
    if col == 0 || col > line.len() || !line.is_char_boundary(col) {
        return None;
    }
    line[..col].char_indices().last().map(|(idx, _)| idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tinyvim-buffer-{}-{}-{}",
            std::process::id(),
            stamp,
            name
        ))
    }

    #[test]
    fn load_and_save_preserves_trailing_newline() {
        let path = temp_file("newline.txt");
        std::fs::write(&path, "hello\n").unwrap();

        let mut buffer = Buffer::load(&path).unwrap();
        buffer.save().unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");
        let _ = std::fs::remove_file(path);
    }
}
