mod ai;
mod buffer;
mod builder;
mod editor;
mod ui;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use buffer::Buffer;
use builder::InteractiveRun;
use editor::Editor;

const WINDOWS_ACCESS_RETRY_ATTEMPTS: usize = 8;
const WINDOWS_ACCESS_RETRY_DELAY: Duration = Duration::from_millis(250);

#[cfg(windows)]
fn prepare_windows_console() {
    use winapi::um::consoleapi::{GetConsoleMode, SetConsoleMode};
    use winapi::um::processenv::GetStdHandle;
    use winapi::um::winbase::STD_OUTPUT_HANDLE;
    use winapi::um::wincon::{
        SetConsoleCP, SetConsoleOutputCP, ENABLE_PROCESSED_OUTPUT,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    };

    const UTF8_CODE_PAGE: u32 = 65001;

    unsafe {
        SetConsoleCP(UTF8_CODE_PAGE);
        SetConsoleOutputCP(UTF8_CODE_PAGE);

        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(out, &mut mode) != 0 {
            SetConsoleMode(
                out,
                mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            );
        }
    }
}

#[cfg(not(windows))]
fn prepare_windows_console() {}

#[cfg(windows)]
fn prepare_windows_interactive_console() {
    use winapi::um::consoleapi::{GetConsoleMode, SetConsoleMode};
    use winapi::um::processenv::GetStdHandle;
    use winapi::um::winbase::STD_INPUT_HANDLE;
    use winapi::um::wincon::{
        ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_MOUSE_INPUT, ENABLE_PROCESSED_INPUT,
        ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_WINDOW_INPUT,
    };

    prepare_windows_console();

    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(input, &mut mode) != 0 {
            let interactive_mode =
                (mode | ENABLE_PROCESSED_INPUT | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT)
                    & !(ENABLE_MOUSE_INPUT | ENABLE_WINDOW_INPUT | ENABLE_VIRTUAL_TERMINAL_INPUT);
            SetConsoleMode(input, interactive_mode);
        }
    }
}

#[cfg(not(windows))]
fn prepare_windows_interactive_console() {}

fn is_windows_access_denied(error: &io::Error) -> bool {
    cfg!(target_os = "windows")
        && (error.kind() == io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(5))
}

fn run_status_with_windows_access_retry(
    command: &mut Command,
    label: &str,
) -> io::Result<std::process::ExitStatus> {
    let program = std::path::Path::new(command.get_program()).to_path_buf();
    let mut last_error = None;

    for attempt in 1..=WINDOWS_ACCESS_RETRY_ATTEMPTS {
        match command.status() {
            Ok(status) => return Ok(status),
            Err(e) if is_windows_access_denied(&e) && attempt < WINDOWS_ACCESS_RETRY_ATTEMPTS => {
                last_error = Some(e);
                std::thread::sleep(WINDOWS_ACCESS_RETRY_DELAY);
            }
            Err(e) if is_windows_access_denied(&e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!(
                        "Windows denied access while starting {} ({}). TinyVim retried automatically. Original error: {}. Close any still-running program window, wait a moment, or allow the project/cache folder in Windows Security, then try again.",
                        label,
                        program.display(),
                        e
                    ),
                ));
            }
            Err(e) => return Err(e),
        }
    }

    let error = last_error.unwrap_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied));
    Err(io::Error::new(
        error.kind(),
        format!(
            "Windows denied access while starting {} ({}). TinyVim retried automatically. Original error: {}. Close any still-running program window, wait a moment, or allow the project/cache folder in Windows Security, then try again.",
            label,
            program.display(),
            error
        ),
    ))
}

fn main() -> io::Result<()> {
    prepare_windows_console();

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
    let _ = stdout.execute(EnableMouseCapture);
    let _ = stdout.execute(EnableBracketedPaste);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.show_cursor()?;

    let mut editor = Editor::new(buffer);
    let mut result = Ok(());

    while !editor.quit {
        editor.poll_active_task();
        terminal.draw(|f| ui::render(f, &editor))?;
        if let Some(run) = editor.pending_run.take() {
            match run_interactive(&mut terminal, run) {
                Ok(message) => editor.message = message,
                Err(e) => {
                    editor.message = format!("Run error: {}", e);
                    editor.build_result = Some(builder::BuildResult {
                        success: false,
                        output: editor.message.clone(),
                        command_line: String::new(),
                    });
                    editor.output_scroll = 0;
                    editor.show_output = true;
                }
            }
            continue;
        }
        let should_read_event =
            editor.active_task.is_none() || event::poll(Duration::from_millis(120))?;
        if should_read_event {
            if let Err(e) = handle_event(&mut editor) {
                if e.kind() == io::ErrorKind::Interrupted {
                    editor.quit = true;
                } else {
                    result = Err(e);
                    break;
                }
            }
        }
    }

    let _ = terminal.backend_mut().execute(DisableBracketedPaste);
    let _ = terminal.backend_mut().execute(DisableMouseCapture);
    let _ = disable_raw_mode();
    prepare_windows_interactive_console();
    let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    result
}

fn run_interactive(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    run: InteractiveRun,
) -> io::Result<String> {
    let _ = terminal.backend_mut().execute(DisableBracketedPaste);
    let _ = terminal.backend_mut().execute(DisableMouseCapture);
    disable_raw_mode()?;
    prepare_windows_interactive_console();
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    println!("TinyVim is running your program in the terminal.");
    println!("Interactive input is enabled for cin, scanf, and input().");
    println!();
    println!("$ cd {} && {}", run.cwd.display(), run.display);
    io::stdout().flush()?;
    let mut command = Command::new(&run.program);
    command
        .current_dir(&run.cwd)
        .args(&run.args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8");
    let status = run_status_with_windows_access_retry(&mut command, "your program");

    prepare_windows_interactive_console();
    println!();
    match &status {
        Ok(status) => println!("Process exited with status: {}", status),
        Err(e) => println!("Failed to run process: {}", e),
    }
    println!("Press Enter to return to TinyVim...");
    io::stdout().flush()?;
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);

    prepare_windows_console();
    enable_raw_mode()?;
    terminal.backend_mut().execute(EnterAlternateScreen)?;
    let _ = terminal.backend_mut().execute(EnableMouseCapture);
    let _ = terminal.backend_mut().execute(EnableBracketedPaste);
    terminal.clear()?;
    terminal.show_cursor()?;

    status.map(|status| {
        if status.success() {
            "Finished".to_string()
        } else {
            format!("Process exited with status: {}", status)
        }
    })
}

fn handle_event(editor: &mut Editor) -> io::Result<()> {
    if editor.file_dialog.is_some() {
        return handle_file_dialog(editor);
    }
    if editor.prompt.is_some() {
        return handle_prompt(editor);
    }
    let ev = event::read()?;
    match ev {
        Event::Key(key) => {
            if editor.should_ignore_key(&key) {
                return Ok(());
            }
            match key.code {
                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    handle_ctrl(editor, c);
                }
                KeyCode::Null if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    editor.trigger_completion();
                }
                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => {
                    handle_alt(editor, c);
                }
                KeyCode::Char(c) => {
                    editor.confirm_quit = false;
                    editor.insert_char(c);
                }
                KeyCode::Enter => {
                    editor.confirm_quit = false;
                    if !editor.accept_completion() {
                        editor.insert_char('\n');
                    }
                }
                KeyCode::Backspace => {
                    editor.confirm_quit = false;
                    editor.backspace();
                }
                KeyCode::Delete => {
                    editor.confirm_quit = false;
                    editor.delete();
                }
                KeyCode::Tab => {
                    editor.confirm_quit = false;
                    if !editor.accept_completion() {
                        editor.insert_char('\t');
                    }
                }
                KeyCode::Esc => {
                    editor.confirm_quit = false;
                    editor.selection = None;
                    editor.show_output = false;
                    editor.show_help = false;
                    editor.close_completion();
                }
                KeyCode::Up if !editor.select_previous_completion() => editor.move_up(),
                KeyCode::Down if !editor.select_next_completion() => editor.move_down(),
                KeyCode::Left => editor.move_left(),
                KeyCode::Right => editor.move_right(),
                KeyCode::Home => editor.home(),
                KeyCode::End => editor.end(),
                KeyCode::PageUp if editor.show_output && editor.build_result.is_some() => {
                    editor.scroll_output_up(10)
                }
                KeyCode::PageDown if editor.show_output && editor.build_result.is_some() => {
                    editor.scroll_output_down(10)
                }
                KeyCode::PageUp => editor.page_up(),
                KeyCode::PageDown => editor.page_down(),
                KeyCode::F(f) => handle_fkey(editor, f, key.modifiers),
                _ => {}
            }
        }
        Event::Paste(text) => {
            editor.confirm_quit = false;
            for ch in text.chars() {
                editor.insert_char(ch);
            }
        }
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                handle_mouse(editor, mouse.column, mouse.row);
            }
            MouseEventKind::ScrollUp => handle_mouse_scroll(editor, mouse.row, true),
            MouseEventKind::ScrollDown => handle_mouse_scroll(editor, mouse.row, false),
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn read_action_key(editor: &mut Editor) -> io::Result<Option<KeyEvent>> {
    let ev = event::read()?;
    if let Event::Key(key) = ev {
        if editor.should_ignore_key(&key) {
            return Ok(None);
        }
        return Ok(Some(key));
    }
    Ok(None)
}

fn handle_file_dialog(editor: &mut Editor) -> io::Result<()> {
    if let Some(key) = read_action_key(editor)? {
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
                        editor.clear_history();
                        editor.clear_ide_state();
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
                    if dlg.selected > 0 {
                        dlg.selected -= 1;
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(ref mut dlg) = editor.file_dialog {
                    if dlg.selected + 1 < dlg.entries.len() {
                        dlg.selected += 1;
                    }
                }
            }
            KeyCode::Enter => {
                let should_open = editor.file_dialog.as_ref().is_some_and(|dlg| {
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
                    editor.clear_history();
                    editor.clear_ide_state();
                    editor.message = format!("Opened {}", editor.buffer.filename());
                    editor.file_dialog = None;
                } else if let Some(ref mut dlg) = editor.file_dialog {
                    dlg.enter_dir();
                }
            }
            KeyCode::Esc => {
                editor.file_dialog = None;
            }
            KeyCode::Char('h') => {
                if let Some(ref mut dlg) = editor.file_dialog {
                    dlg.go_up();
                }
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
            KeyCode::Char('r') => {
                if let Some(ref dlg) = editor.file_dialog {
                    if !dlg.entries.is_empty() && dlg.selected < dlg.entries.len() {
                        let entry = &dlg.entries[dlg.selected];
                        if entry.name != ".." {
                            editor.pending_rename = Some((dlg.cwd.clone(), entry.name.clone()));
                            editor.prompt =
                                Some(editor::Prompt::new(&format!("Rename '{}' to:", entry.name)));
                            editor.file_dialog = None;
                        }
                    }
                }
            }
            KeyCode::Char(' ') => {
                if let Some(ref mut dlg) = editor.file_dialog {
                    if !dlg.entries.is_empty() && dlg.selected < dlg.entries.len() {
                        let entry = &dlg.entries[dlg.selected];
                        if entry.name != ".." {
                            let _ = dlg.preview_selected();
                        }
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(ref dlg) = editor.file_dialog {
                    if dlg.entries.is_empty() || dlg.selected >= dlg.entries.len() {
                        editor.message = "No file selected".to_string();
                        return Ok(());
                    }
                    let entry = &dlg.entries[dlg.selected];
                    if entry.name == ".." {
                        editor.message = "Cannot delete parent directory".to_string();
                        return Ok(());
                    }

                    editor.pending_delete = Some((dlg.cwd.clone(), entry.name.clone()));
                    editor.prompt = Some(editor::Prompt::new(&format!(
                        "Delete '{}'? Type y to confirm:",
                        entry.name
                    )));
                    editor.file_dialog = None;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_prompt(editor: &mut Editor) -> io::Result<()> {
    match event::read()? {
        Event::Key(key) => {
            if editor.should_ignore_key(&key) {
                return Ok(());
            }
            match key.code {
                KeyCode::PageUp if editor.show_output && editor.build_result.is_some() => {
                    editor.scroll_output_up(10);
                }
                KeyCode::PageDown if editor.show_output && editor.build_result.is_some() => {
                    editor.scroll_output_down(10);
                }
                KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    paste_system_clipboard_into_prompt(editor);
                }
                KeyCode::Char(c) => {
                    if editor.pending_ai_apply {
                        handle_ai_apply_prompt(editor, c.to_string());
                        return Ok(());
                    }
                    if let Some(ref mut p) = editor.prompt {
                        p.input.push(c);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(ref mut p) = editor.prompt {
                        p.input.pop();
                    }
                }
                KeyCode::Esc => {
                    editor.prompt = None;
                    editor.pending_search = false;
                    editor.pending_ai_key = false;
                    editor.pending_ai_base_url = false;
                    editor.pending_ai_model = false;
                    editor.pending_ai_setup_key = None;
                    editor.pending_ai_setup_base_url = None;
                    editor.pending_ai_setup_model = None;
                    editor.pending_ai_setup_chat = false;
                    editor.pending_ai_setup_only = false;
                    editor.pending_ai_edit = false;
                    editor.pending_ai_chat = false;
                    editor.pending_ai_apply = false;
                    editor.pending_ai_edit_content = None;
                    if let Some(dir) = editor.pending_mkfile.take() {
                        editor.file_dialog = Some(editor::FileDialog::new(dir));
                    }
                    if let Some(dir) = editor.pending_mkdir.take() {
                        editor.file_dialog = Some(editor::FileDialog::new(dir));
                    }
                    if let Some((dir, _)) = editor.pending_rename.take() {
                        editor.file_dialog = Some(editor::FileDialog::new(dir));
                    }
                    if let Some((dir, _)) = editor.pending_delete.take() {
                        editor.file_dialog = Some(editor::FileDialog::new(dir));
                    }
                }
                KeyCode::Enter => {
                    let name = editor
                        .prompt
                        .take()
                        .map(|p| p.input.trim().to_string())
                        .unwrap_or_default();
                    if editor.pending_search {
                        editor.pending_search = false;
                        editor.set_search(name);
                        return Ok(());
                    }
                    if editor.pending_ai_key {
                        editor.pending_ai_key = false;
                        handle_ai_key_prompt(editor, name);
                        return Ok(());
                    }
                    if editor.pending_ai_base_url {
                        editor.pending_ai_base_url = false;
                        handle_ai_base_url_prompt(editor, name);
                        return Ok(());
                    }
                    if editor.pending_ai_model {
                        editor.pending_ai_model = false;
                        handle_ai_model_prompt(editor, name);
                        return Ok(());
                    }
                    if editor.pending_ai_edit {
                        editor.pending_ai_edit = false;
                        handle_ai_edit_prompt(editor, name);
                        return Ok(());
                    }
                    if editor.pending_ai_chat {
                        editor.pending_ai_chat = false;
                        handle_ai_chat_prompt(editor, name);
                        return Ok(());
                    }
                    if editor.pending_ai_apply {
                        editor.pending_ai_apply = false;
                        handle_ai_apply_prompt(editor, name);
                        return Ok(());
                    }

                    if name.is_empty() {
                        editor.pending_mkfile = None;
                        editor.pending_mkdir = None;
                        if let Some((dir, _)) = editor.pending_rename.take() {
                            editor.file_dialog = Some(editor::FileDialog::new(dir));
                        }
                        if let Some((dir, _)) = editor.pending_delete.take() {
                            editor.message = "Delete cancelled".to_string();
                            editor.file_dialog = Some(editor::FileDialog::new(dir));
                        }
                        return Ok(());
                    }

                    if let Some(dir) = editor.pending_mkfile.take() {
                        let mut dlg = editor::FileDialog::new(dir);
                        match dlg.create_file(&name) {
                            Ok(()) => editor.message = format!("Created {}", name),
                            Err(e) => editor.message = e,
                        }
                        editor.file_dialog = Some(dlg);
                        return Ok(());
                    }
                    if let Some(dir) = editor.pending_mkdir.take() {
                        let mut dlg = editor::FileDialog::new(dir);
                        match dlg.create_directory(&name) {
                            Ok(()) => editor.message = format!("Created directory {}", name),
                            Err(e) => editor.message = e,
                        }
                        editor.file_dialog = Some(dlg);
                        return Ok(());
                    }
                    if let Some((dir, old_name)) = editor.pending_rename.take() {
                        let mut dlg = editor::FileDialog::new(dir);
                        match dlg.rename_file(&old_name, &name) {
                            Ok(()) => {
                                editor.message = format!("Renamed '{}' to '{}'", old_name, name);
                            }
                            Err(e) => {
                                editor.message = format!("Rename failed: {}", e);
                            }
                        }
                        editor.file_dialog = Some(dlg);
                        return Ok(());
                    }
                    if let Some((dir, delete_name)) = editor.pending_delete.take() {
                        let mut dlg = editor::FileDialog::new(dir);
                        if name.eq_ignore_ascii_case("y") {
                            match dlg.delete_entry(&delete_name) {
                                Ok(()) => {
                                    editor.message = format!("Deleted '{}'", delete_name);
                                }
                                Err(e) => {
                                    editor.message = format!("Delete failed: {}", e);
                                }
                            }
                        } else {
                            editor.message = "Delete cancelled".to_string();
                        }
                        editor.file_dialog = Some(dlg);
                        return Ok(());
                    }

                    // Normal save prompt
                    editor.buffer.filepath = Some(name);
                    editor.save();
                    editor.selection = None;
                }
                _ => {}
            }
        }
        Event::Paste(text) => {
            if let Some(ref mut p) = editor.prompt {
                p.input.push_str(&prompt_paste_text(&text));
            }
        }
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => handle_mouse_scroll(editor, mouse.row, true),
            MouseEventKind::ScrollDown => handle_mouse_scroll(editor, mouse.row, false),
            MouseEventKind::Down(MouseButton::Left) => {
                handle_mouse(editor, mouse.column, mouse.row);
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn paste_system_clipboard_into_prompt(editor: &mut Editor) {
    match read_system_clipboard() {
        Ok(text) if !text.is_empty() => {
            if let Some(ref mut prompt) = editor.prompt {
                prompt.input.push_str(&prompt_paste_text(&text));
                editor.message = "Pasted from system clipboard".to_string();
            }
        }
        Ok(_) => {
            editor.message = "System clipboard is empty".to_string();
        }
        Err(e) => {
            editor.message = format!("Paste failed: {}", e);
        }
    }
}

fn prompt_paste_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace(['\r', '\n'], " ")
}

fn read_system_clipboard() -> Result<String, String> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "windows") {
        &[
            (
                "powershell.exe",
                &[
                    "-NoProfile",
                    "-Command",
                    "[Console]::OutputEncoding=[Text.UTF8Encoding]::UTF8; Get-Clipboard -Raw",
                ],
            ),
            (
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "[Console]::OutputEncoding=[Text.UTF8Encoding]::UTF8; Get-Clipboard -Raw",
                ],
            ),
            (
                "pwsh",
                &[
                    "-NoProfile",
                    "-Command",
                    "[Console]::OutputEncoding=[Text.UTF8Encoding]::UTF8; Get-Clipboard -Raw",
                ],
            ),
        ]
    } else if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ]
    };

    let mut last_error = "no clipboard command available".to_string();
    for (program, args) in candidates {
        match Command::new(program).args(*args).output() {
            Ok(output) if output.status.success() => {
                return Ok(String::from_utf8_lossy(&output.stdout).to_string());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                last_error = format!("{} exited with {}: {}", program, output.status, stderr);
            }
            Err(e) => {
                last_error = format!("{}: {}", program, e);
            }
        }
    }
    Err(last_error)
}

fn handle_ctrl(editor: &mut Editor, c: char) {
    if c != 'q' {
        editor.confirm_quit = false;
    }
    match c {
        's' => {
            editor.save();
        }
        'f' => {
            editor.pending_search = true;
            editor.prompt = Some(editor::Prompt::new("Search:"));
        }
        'e' => {
            start_ai_flow(editor, false);
        }
        'r' => {
            start_ai_flow(editor, true);
        }
        ' ' => editor.trigger_completion(),
        'q' => {
            if editor.buffer.modified && !editor.confirm_quit {
                editor.confirm_quit = true;
                editor.message =
                    "Unsaved changes. Press Ctrl+S to save, or Ctrl+Q again to force quit."
                        .to_string();
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
            editor.clear_history();
            editor.clear_ide_state();
            editor.message = "New file. Press Ctrl+S to save.".to_string();
        }
        'z' => editor.undo(),
        'x' => editor.cut_line(),
        'c' => editor.copy_line(),
        'v' => editor.paste(),
        'a' if editor.buffer.num_lines() > 0 => {
            let last = editor.buffer.num_lines() - 1;
            let last_len = editor.buffer.line_len(last);
            editor.selection = Some((0, 0, last, last_len));
        }
        _ => {}
    }
}

fn start_ai_flow(editor: &mut Editor, chat: bool) {
    if ai::has_config() {
        start_configured_ai_flow(editor, chat);
    } else {
        editor.pending_ai_setup_chat = chat;
        editor.pending_ai_setup_only = false;
        editor.pending_ai_base_url = true;
        editor.prompt = Some(editor::Prompt::with_input(
            "AI Base URL:",
            ai::default_base_url(),
        ));
        editor.message = "Edit base URL or press Enter to keep DeepSeek default.".to_string();
    }
}

fn start_ai_config_flow(editor: &mut Editor) {
    editor.pending_ai_setup_chat = false;
    editor.pending_ai_setup_only = true;
    editor.pending_ai_edit = false;
    editor.pending_ai_chat = false;
    editor.pending_ai_apply = false;
    editor.pending_ai_edit_content = None;
    editor.pending_ai_base_url = true;
    editor.prompt = Some(editor::Prompt::with_input(
        "AI Base URL:",
        ai::default_base_url(),
    ));
    editor.message = if ai::env_overrides_ai_config() {
        "AI is using environment variables. F2 can save config, but env vars still win; update or clear TINYVIM_AI_API_KEY/OPENAI_API_KEY.".to_string()
    } else {
        "Reconfigure AI. Edit base URL or press Enter to keep DeepSeek default.".to_string()
    };
}

fn start_configured_ai_flow(editor: &mut Editor, chat: bool) {
    if chat {
        editor.pending_ai_chat = true;
        editor.prompt = Some(editor::Prompt::new("AI Chat:"));
        editor.message = "Ask AI a question. It will not edit the file.".to_string();
    } else {
        editor.pending_ai_edit = true;
        editor.prompt = Some(editor::Prompt::new("AI Edit:"));
        editor.message = "Describe the code change for the current file.".to_string();
    }
}

fn handle_ai_key_prompt(editor: &mut Editor, api_key: String) {
    if api_key.trim().is_empty() {
        editor.message = "AI API key cannot be empty".to_string();
        return;
    }
    let base_url = editor
        .pending_ai_setup_base_url
        .take()
        .unwrap_or_else(|| ai::default_base_url().to_string());
    let model = editor
        .pending_ai_setup_model
        .take()
        .unwrap_or_else(|| ai::default_model().to_string());
    match ai::save_config(&api_key, &base_url, &model) {
        Ok(()) => {
            let chat = editor.pending_ai_setup_chat;
            editor.pending_ai_setup_chat = false;
            let setup_only = editor.pending_ai_setup_only;
            editor.pending_ai_setup_only = false;

            match ai::test_current_config() {
                Ok(test_message) => {
                    if setup_only {
                        if ai::env_overrides_ai_config() {
                            show_ai_panel(
                                editor,
                                format!(
                                    "{test_message}\n\nSaved config exists, but environment variables still override it.\nUpdate or clear TINYVIM_AI_API_KEY / OPENAI_API_KEY if this is not the key you just pasted."
                                ),
                                "AI config test",
                                "AI config saved, but environment variables still win.",
                            );
                        } else {
                            show_ai_panel(
                                editor,
                                test_message,
                                "AI config test",
                                "AI config saved and tested. Press Ctrl+R to chat or Ctrl+E to edit.",
                            );
                        }
                    } else {
                        start_configured_ai_flow(editor, chat);
                        let message = if chat {
                            "AI config saved and tested. Ask AI a question."
                        } else {
                            "AI config saved and tested. Describe the code change."
                        };
                        show_ai_panel(editor, test_message, "AI config test", message);
                    }
                }
                Err(e) => {
                    show_ai_panel_with_status(
                        editor,
                        false,
                        ai::auth_error_detail(&e),
                        "AI config test",
                        "AI config saved, but test failed. Press F2 and paste the full key again.",
                    );
                }
            }
        }
        Err(e) => {
            show_ai_error(editor, e, Some(false));
        }
    }
}

fn handle_ai_base_url_prompt(editor: &mut Editor, base_url: String) {
    let base_url = if base_url.trim().is_empty() {
        ai::default_base_url().to_string()
    } else {
        base_url
    };
    editor.pending_ai_setup_base_url = Some(base_url);
    editor.pending_ai_model = true;
    editor.prompt = Some(editor::Prompt::with_input("AI Model:", ai::default_model()));
    editor.message = "Edit model or press Enter to keep DeepSeek default.".to_string();
}

fn handle_ai_model_prompt(editor: &mut Editor, model: String) {
    let model = if model.trim().is_empty() {
        ai::default_model().to_string()
    } else {
        model
    };
    editor.pending_ai_setup_model = Some(model);
    editor.pending_ai_key = true;
    editor.prompt = Some(editor::Prompt::secret("AI API Key:"));
    editor.message = "Paste API key. Input is hidden; Ctrl+V uses system clipboard.".to_string();
}

fn handle_ai_edit_prompt(editor: &mut Editor, instruction: String) {
    if instruction.trim().is_empty() {
        editor.message = "AI edit cancelled".to_string();
        return;
    }
    let instruction = instruction.trim();

    let filename = editor.buffer.filename().to_string();
    let language = editor.buffer.extension().to_string();
    let content = editor.buffer.lines.join("\n");

    editor.message = "AI is editing current file...".to_string();
    match ai::edit_current_file(ai::AiEditRequest {
        instruction,
        filename: &filename,
        language: &language,
        content: &content,
        history: &editor.ai_edit_history,
    }) {
        Ok(new_content) => {
            editor.ai_edit_history.push(ai::AiTurn {
                role: ai::AiRole::User,
                content: instruction.to_string(),
            });
            editor.ai_edit_history.push(ai::AiTurn {
                role: ai::AiRole::Assistant,
                content: format!(
                    "Proposed updated file:\n```{}\n{}\n```",
                    language, new_content
                ),
            });
            editor.pending_ai_edit_content = Some(new_content.clone());
            editor.pending_ai_apply = true;
            editor.prompt = Some(editor::Prompt::new("Apply AI edit? y/n:"));
            show_ai_panel(
                editor,
                format!("AI generated this candidate file:\n\n{}", new_content),
                "AI edit preview",
                "Review preview below. Press y to apply, n to discard.",
            );
        }
        Err(e) => editor.message = e,
    }
}

fn handle_ai_chat_prompt(editor: &mut Editor, question: String) {
    if question.trim().is_empty() {
        editor.message = "AI chat cancelled".to_string();
        return;
    }
    let question = question.trim();

    if asks_current_ai_config(question) {
        match ai::config_summary() {
            Ok(summary) => {
                show_ai_output(editor, summary);
                reopen_ai_chat(editor);
            }
            Err(e) => editor.message = e,
        }
        return;
    }

    let filename = editor.buffer.filename().to_string();
    let language = editor.buffer.extension().to_string();
    let content = editor.buffer.lines.join("\n");
    editor.message = "AI is answering...".to_string();

    match ai::chat(ai::AiChatRequest {
        question,
        filename: &filename,
        language: &language,
        content: &content,
        history: &editor.ai_chat_history,
    }) {
        Ok(answer) => {
            editor.ai_chat_history.push(ai::AiTurn {
                role: ai::AiRole::User,
                content: question.to_string(),
            });
            editor.ai_chat_history.push(ai::AiTurn {
                role: ai::AiRole::Assistant,
                content: answer.clone(),
            });
            show_ai_output(editor, answer);
            reopen_ai_chat(editor);
        }
        Err(e) => {
            show_ai_error(editor, e, Some(true));
        }
    }
}

fn show_ai_error(editor: &mut Editor, error: String, reopen_chat: Option<bool>) {
    if ai::is_auth_error_message(&error) {
        show_ai_panel(
            editor,
            ai::auth_error_detail(&error),
            "AI auth error",
            "AI key was rejected. Reconfigure below, or press Esc and fix environment variables.",
        );
        if ai::env_overrides_ai_config() {
            if reopen_chat == Some(true) {
                reopen_ai_chat(editor);
            } else if reopen_chat == Some(false) {
                reopen_ai_edit(editor);
            }
        } else {
            let resume_chat = reopen_chat.unwrap_or(false);
            editor.pending_ai_setup_chat = resume_chat;
            editor.pending_ai_setup_only = reopen_chat.is_none();
            editor.pending_ai_edit = false;
            editor.pending_ai_chat = false;
            editor.pending_ai_apply = false;
            editor.pending_ai_edit_content = None;
            editor.pending_ai_base_url = true;
            editor.prompt = Some(editor::Prompt::with_input(
                "AI Base URL:",
                ai::default_base_url(),
            ));
        }
    } else {
        editor.message = error;
        if reopen_chat == Some(true) {
            reopen_ai_chat(editor);
        } else if reopen_chat == Some(false) {
            reopen_ai_edit(editor);
        }
    }
}

fn show_ai_output(editor: &mut Editor, output: String) {
    show_ai_panel(
        editor,
        output,
        "AI chat",
        "AI answer shown below. Esc closes AI chat input.",
    );
}

fn show_ai_panel(editor: &mut Editor, output: String, command_line: &str, message: &str) {
    show_ai_panel_with_status(editor, true, output, command_line, message);
}

fn show_ai_panel_with_status(
    editor: &mut Editor,
    success: bool,
    output: String,
    command_line: &str,
    message: &str,
) {
    editor.build_result = Some(builder::BuildResult {
        success,
        output,
        command_line: command_line.to_string(),
    });
    editor.output_scroll = 0;
    editor.show_output = true;
    editor.show_help = false;
    editor.message = message.to_string();
}

fn reopen_ai_chat(editor: &mut Editor) {
    editor.pending_ai_chat = true;
    editor.prompt = Some(editor::Prompt::new("AI Chat:"));
}

fn reopen_ai_edit(editor: &mut Editor) {
    editor.pending_ai_edit = true;
    editor.prompt = Some(editor::Prompt::new("AI Edit:"));
}

fn handle_ai_apply_prompt(editor: &mut Editor, answer: String) {
    let answer = answer.trim().to_ascii_lowercase();
    editor.pending_ai_apply = false;
    if answer == "y" || answer == "yes" {
        if let Some(content) = editor.pending_ai_edit_content.take() {
            editor.apply_ai_edit(content);
            editor.pending_ai_edit = true;
            editor.prompt = Some(editor::Prompt::new("AI Edit:"));
        } else {
            editor.message = "No AI edit to apply".to_string();
            editor.pending_ai_edit = true;
            editor.prompt = Some(editor::Prompt::new("AI Edit:"));
        }
    } else if answer == "n" || answer == "no" {
        editor.pending_ai_edit_content = None;
        editor.pending_ai_edit = true;
        editor.prompt = Some(editor::Prompt::new("AI Edit:"));
        editor.message =
            "AI edit discarded. Describe another change or press Esc to exit.".to_string();
    } else {
        editor.pending_ai_apply = true;
        editor.prompt = Some(editor::Prompt::new("Apply AI edit? y/n:"));
        editor.message = "Press y to apply or n to discard.".to_string();
    }
}

fn asks_current_ai_config(input: &str) -> bool {
    let text = input.to_ascii_lowercase();
    if text.contains("why")
        || text.contains("how")
        || input.contains("为什么")
        || input.contains("为啥")
        || input.contains("怎么")
        || input.contains("如何")
    {
        return false;
    }

    let asks_current = text.contains("current")
        || text.contains("show")
        || text.contains("which")
        || text.contains("what")
        || input.contains("当前")
        || input.contains("现在")
        || input.contains("显示")
        || input.contains("查看")
        || input.contains("用的是")
        || input.contains("用的什么")
        || input.contains("是什么")
        || input.contains("是啥");
    let mentions_config = text.contains("model")
        || text.contains("base url")
        || text.contains("api key")
        || text.contains("apikey")
        || input.contains("模型")
        || input.contains("接口地址")
        || input.contains("配置");

    asks_current && mentions_config
}

fn handle_alt(editor: &mut Editor, c: char) {
    editor.confirm_quit = false;
    match c {
        'h' => editor.move_word_left(),
        'l' => editor.move_word_right(),
        _ => {}
    }
}

fn handle_fkey(editor: &mut Editor, f: u8, modifiers: KeyModifiers) {
    editor.confirm_quit = false;
    match f {
        1 => {
            editor.show_help = !editor.show_help;
            if editor.show_help {
                editor.show_output = false;
            }
        }
        2 => start_ai_config_flow(editor),
        3 if modifiers.contains(KeyModifiers::SHIFT) => editor.previous_search(),
        3 => editor.next_search(),
        5 => editor.do_compile(),
        6 => editor.do_compile_run(),
        8 if modifiers.contains(KeyModifiers::SHIFT) => editor.previous_diagnostic(),
        8 => editor.next_diagnostic(),
        _ => {}
    }
}

fn handle_mouse(editor: &mut Editor, _x: u16, y: u16) {
    if let Some(idx) = diagnostic_index_at_row(editor, y) {
        editor.goto_diagnostic(idx);
    }
}

fn handle_mouse_scroll(editor: &mut Editor, y: u16, up: bool) {
    if mouse_row_is_output(editor, y) {
        if up {
            editor.scroll_output_up(3);
        } else {
            editor.scroll_output_down(3);
        }
    } else if up {
        for _ in 0..3 {
            editor.move_up();
        }
    } else {
        for _ in 0..3 {
            editor.move_down();
        }
    }
}

fn mouse_row_is_output(editor: &Editor, y: u16) -> bool {
    if !editor.show_output || editor.build_result.is_none() || editor.show_help {
        return false;
    }
    let Ok((_width, height)) = crossterm::terminal::size() else {
        return false;
    };
    let main_height = (height as f64 * 0.6) as u16;
    y > main_height
}

fn diagnostic_index_at_row(editor: &Editor, y: u16) -> Option<usize> {
    if !editor.show_output || editor.build_result.is_none() || editor.diagnostics.is_empty() {
        return None;
    }
    let (_width, height) = crossterm::terminal::size().ok()?;
    let main_height = (height as f64 * 0.6) as u16;
    let first_problem_row = main_height + 3;
    let idx = y.checked_sub(first_problem_row)? as usize;
    (idx < editor.diagnostics.len().min(6)).then_some(idx)
}
