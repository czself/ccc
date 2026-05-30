mod buffer;
mod builder;
mod editor;
mod ui;

use std::fs;
use std::io;
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use buffer::Buffer;
use editor::Editor;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let buffer = if args.len() > 1 {
        Buffer::load(&args[1]).unwrap_or_else(|_| {
            let mut b = Buffer::new();
            b.filepath = Some(args[1].clone());
            b
        })
    } else {
        Buffer::new()
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.show_cursor()?;

    let mut editor = Editor::new(buffer);
    let mut result = Ok(());

    while !editor.quit {
        terminal.draw(|f| ui::render(f, &editor))?;
        if let Err(e) = handle_event(&mut editor) {
            if e.kind() == io::ErrorKind::Interrupted {
                editor.quit = true;
            } else {
                result = Err(e);
                break;
            }
        }
    }

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn handle_event(editor: &mut Editor) -> io::Result<()> {
    if editor.file_dialog.is_some() {
        return handle_file_dialog(editor);
    }
    if editor.prompt.is_some() {
        return handle_prompt(editor);
    }
    let ev = event::read()?;
    if let Event::Key(key) = ev {
        match key.code {
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                handle_ctrl(editor, c);
            }
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => {
                handle_alt(editor, c);
            }
            KeyCode::Char(c) => {
                editor.confirm_quit = false;
                editor.insert_char(c);
            }
            KeyCode::Enter => { editor.confirm_quit = false; editor.insert_char('\n'); }
            KeyCode::Backspace => { editor.confirm_quit = false; editor.backspace(); }
            KeyCode::Delete => { editor.confirm_quit = false; editor.delete(); }
            KeyCode::Tab => { editor.confirm_quit = false; editor.insert_char('\t'); }
            KeyCode::Esc => {
                editor.confirm_quit = false;
                editor.selection = None;
                editor.show_output = false;
            }
            KeyCode::Up => editor.move_up(),
            KeyCode::Down => editor.move_down(),
            KeyCode::Left => editor.move_left(),
            KeyCode::Right => editor.move_right(),
            KeyCode::Home => editor.home(),
            KeyCode::End => editor.end(),
            KeyCode::PageUp => editor.page_up(),
            KeyCode::PageDown => editor.page_down(),
            KeyCode::F(f) => handle_fkey(editor, f),
            _ => {}
        }
    }
    Ok(())
}

fn handle_file_dialog(editor: &mut Editor) -> io::Result<()> {
    let ev = event::read()?;
    if let Event::Key(key) = ev {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if let KeyCode::Char(c) = key.code {
                match c {
                    'n' => {
                        editor.file_dialog = None;
                        editor.buffer = Buffer::new();
                        editor.cursor_row = 0;
                        editor.cursor_col = 0;
                        editor.selection = None;
                        editor.show_output = false;
                        editor.message = "New file. Press Ctrl+S to save.".to_string();
                    }
                    'o' => {
                        editor.file_dialog = None;
                        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                        editor.file_dialog = Some(editor::FileDialog::new(cwd));
                    }
                    _ => {}
                }
                return Ok(());
            }
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(ref mut dlg) = editor.file_dialog {
                    if dlg.selected > 0 { dlg.selected -= 1; }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(ref mut dlg) = editor.file_dialog {
                    if dlg.selected + 1 < dlg.entries.len() { dlg.selected += 1; }
                }
            }
            KeyCode::Enter => {
                let should_open = editor.file_dialog.as_ref().map_or(false, |dlg| {
                    !dlg.entries.is_empty() && !dlg.entries[dlg.selected].is_dir
                });
                if should_open {
                    let path = editor.file_dialog.as_ref().unwrap().selected_path();
                    let path_str = path.to_string_lossy().to_string();
                    let new_buffer = Buffer::load(&path).unwrap_or_else(|_| {
                        let mut b = Buffer::new();
                        b.filepath = Some(path_str.clone());
                        b
                    });
                    editor.buffer = new_buffer;
                    editor.cursor_row = 0;
                    editor.cursor_col = 0;
                    editor.selection = None;
                    editor.show_output = false;
                    editor.message = format!("Opened {}", editor.buffer.filename());
                    editor.file_dialog = None;
                } else if let Some(ref mut dlg) = editor.file_dialog {
                    dlg.enter_dir();
                }
            }
            KeyCode::Esc => { editor.file_dialog = None; }
            KeyCode::Char('h') => {
                if let Some(ref mut dlg) = editor.file_dialog { dlg.go_up(); }
            }
            KeyCode::Char('l') => {
                if let Some(ref mut dlg) = editor.file_dialog {
                    if dlg.selected < dlg.entries.len() && dlg.entries[dlg.selected].is_dir {
                        dlg.enter_dir();
                    }
                }
            }
            KeyCode::Char('n') => {
                if let Some(ref dlg) = editor.file_dialog {
                    editor.pending_mkfile = Some(dlg.cwd.clone());
                    editor.prompt = Some(editor::Prompt::new("New file:"));
                    editor.file_dialog = None;
                }
            }
            KeyCode::Char('m') => {
                if let Some(ref dlg) = editor.file_dialog {
                    editor.pending_mkdir = Some(dlg.cwd.clone());
                    editor.prompt = Some(editor::Prompt::new("New directory:"));
                    editor.file_dialog = None;
                }
            }
            KeyCode::Char('d') => {
                let (path, name) = {
                    let dlg = editor.file_dialog.as_ref().unwrap();
                    let entry = &dlg.entries[dlg.selected];
                    if entry.name == ".." { return Ok(()); }
                    (dlg.selected_path(), entry.name.clone())
                };
                if path.is_dir() {
                    fs::remove_dir_all(&path).ok();
                } else {
                    fs::remove_file(&path).ok();
                }
                editor.message = format!("Deleted {}", name);
                if let Some(ref mut dlg) = editor.file_dialog { dlg.refresh(); }
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_prompt(editor: &mut Editor) -> io::Result<()> {
    let ev = event::read()?;
    if let Event::Key(key) = ev {
        match key.code {
            KeyCode::Char(c) => {
                if let Some(ref mut p) = editor.prompt { p.input.push(c); }
            }
            KeyCode::Backspace => {
                if let Some(ref mut p) = editor.prompt { p.input.pop(); }
            }
            KeyCode::Esc => {
                editor.prompt = None;
                if let Some(dir) = editor.pending_mkfile.take() {
                    editor.file_dialog = Some(editor::FileDialog::new(dir));
                }
                if let Some(dir) = editor.pending_mkdir.take() {
                    editor.file_dialog = Some(editor::FileDialog::new(dir));
                }
            }
            KeyCode::Enter => {
                let name = editor.prompt.take().map(|p| p.input.trim().to_string()).unwrap_or_default();
                if name.is_empty() {
                    editor.pending_mkfile = None;
                    editor.pending_mkdir = None;
                    return Ok(());
                }

                if let Some(dir) = editor.pending_mkfile.take() {
                    let full_path = dir.join(&name);
                    std::fs::write(&full_path, b"")?;
                    editor.message = format!("Created {}", name);
                    editor.file_dialog = Some(editor::FileDialog::new(dir));
                    return Ok(());
                }
                if let Some(dir) = editor.pending_mkdir.take() {
                    let full_path = dir.join(&name);
                    std::fs::create_dir_all(&full_path)?;
                    editor.message = format!("Created directory {}", name);
                    editor.file_dialog = Some(editor::FileDialog::new(dir));
                    return Ok(());
                }

                // Normal save prompt
                editor.buffer.filepath = Some(name);
                editor.buffer.save().ok();
                editor.message = format!("Saved {}", editor.buffer.filename());
                editor.selection = None;
                editor.show_output = false;
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_ctrl(editor: &mut Editor, c: char) {
    if c != 'q' { editor.confirm_quit = false; }
    match c {
        's' => { editor.save(); }
        'q' => {
            if editor.buffer.modified && !editor.confirm_quit {
                editor.confirm_quit = true;
                editor.message = "Unsaved changes. Press Ctrl+S to save, or Ctrl+Q again to force quit.";
            } else {
                editor.quit = true;
            }
        }
        'o' => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            editor.file_dialog = Some(editor::FileDialog::new(cwd));
        }
        'n' => {
            editor.buffer = Buffer::new();
            editor.cursor_row = 0;
            editor.cursor_col = 0;
            editor.selection = None;
            editor.show_output = false;
            editor.message = "New file. Press Ctrl+S to save.".to_string();
        }
        'z' => { editor.message = "Undo not implemented yet".to_string(); }
        'x' => editor.cut_line(),
        'c' => editor.copy_line(),
        'v' => editor.paste(),
        'a' => {
            if editor.buffer.num_lines() > 0 {
                let last = editor.buffer.num_lines() - 1;
                let last_len = editor.buffer.line_len(last);
                editor.selection = Some((0, 0, last, last_len));
            }
        }
        _ => {}
    }
}

fn handle_alt(editor: &mut Editor, c: char) {
    editor.confirm_quit = false;
    match c {
        'h' => editor.move_word_left(),
        'l' => editor.move_word_right(),
        _ => {}
    }
}

fn handle_fkey(editor: &mut Editor, f: u8) {
    editor.confirm_quit = false;
    match f {
        5 => editor.do_compile(),
        6 => editor.do_compile_run(),
        _ => {}
    }
}
