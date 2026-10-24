use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::{Action, Bindings, LoadedConfig};
use crate::editor::Editor;
use crate::file_tree::FileTree;
use crate::highlight::Highlighter;
use crate::logging;
use crate::platform;
use crate::runner::{self, RunExit, RunPlan, Runner};
use crate::theme::Theme;

pub const EXTENSIONS: [&str; 13] = [
    "py", "js", "ts", "rs", "go", "c", "cpp", "sh", "json", "yaml", "md", "toml", "txt",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Editor,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Find,
    Goto,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    None,
    NewFileName {
        folder: PathBuf,
        input: String,
        error: Option<String>,
    },
    NewFolderName {
        parent: PathBuf,
        input: String,
        error: Option<String>,
    },
    ExtensionSelect {
        folder: PathBuf,
        base: String,
        original_name: String,
        selected: usize,
        error: Option<String>,
    },
    RuntimeSelect {
        target: PathBuf,
        selected: usize,
        error: Option<String>,
    },
    SettingsSelect {
        selected: usize,
        error: Option<String>,
    },
    DeleteConfirm {
        path: PathBuf,
        is_dir: bool,
    },
    RenamePath {
        path: PathBuf,
        is_dir: bool,
        input: String,
        error: Option<String>,
    },
    SaveConfirm {
        then_quit: bool,
        pending_focus_tree: bool,
    },
    QuitConfirm,
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

pub struct App {
    pub focus: Focus,
    pub cursor_mode: crate::editor::CursorMode,
    pub tree: FileTree,
    pub workdir: PathBuf,
    pub editor: Editor,
    pub highlighter: Highlighter,
    pub runner: Runner,
    pub theme: Theme,
    pub bindings: Bindings,
    pub run_rules: BTreeMap<String, String>,
    pub input_mode: InputMode,
    pub prompt: String,
    pub last_search: String,
    pub modal: Modal,
    pub notice: Option<Notice>,
    pub clipboard: String,
    pub should_quit: bool,
}

impl App {
    pub fn new(settings: LoadedConfig, root: PathBuf, initial_file: Option<PathBuf>) -> Self {
        let mut file_tree = FileTree::new(root);
        let mut editor = Editor::default();
        editor.cursor_mode = settings.cursor_mode;
        let mut notice = None;
        if let Some(file) = initial_file.as_ref() {
            match Editor::open(file, 4) {
                Ok(opened) => {
                    editor = opened;
                    editor.cursor_mode = settings.cursor_mode;
                    file_tree.reveal_file(file);
                }
                Err(error) => {
                    logging::error(&error);
                    notice = Some(Notice {
                        text: error,
                        error: true,
                        until: Instant::now() + Duration::from_secs(8),
                    });
                }
            }
        }
        let focus = if editor.has_file() {
            Focus::Editor
        } else {
            Focus::Tree
        };
        let workdir = settings
            .workdir
            .filter(|path| path.starts_with(&file_tree.root))
            .unwrap_or_else(|| file_tree.root.clone());
        Self {
            focus,
            cursor_mode: settings.cursor_mode,
            tree: file_tree,
            workdir,
            editor,
            highlighter: Highlighter::default(),
            runner: Runner::new(),
            theme: settings.theme,
            bindings: settings.bindings,
            run_rules: settings.run,
            input_mode: InputMode::Normal,
            prompt: String::new(),
            last_search: String::new(),
            modal: Modal::None,
            notice,
            clipboard: String::new(),
            should_quit: false,
        }
    }

    pub fn sync_highlights(&mut self) {
        self.highlighter.update(
            self.editor.language,
            self.editor.revision,
            &self.editor.lines,
        );
    }

    pub fn tick(&mut self) {
        self.runner.tick();
        if let Some(notice) = &self.notice {
            if Instant::now() >= notice.until {
                self.notice = None;
            }
        }
        if !self.runner.is_running() && self.focus == Focus::Output {
            self.focus = Focus::Editor;
        }
    }

    pub fn active_notice(&self) -> Option<&Notice> {
        self.notice
            .as_ref()
            .filter(|notice| Instant::now() < notice.until)
    }

    pub fn handle_paste(&mut self, text: &str) {
        if self.modal != Modal::None {
            return;
        }
        if self.input_mode != InputMode::Normal {
            for ch in text.chars() {
                if !ch.is_control() {
                    self.prompt.push(ch);
                }
            }
            return;
        }
        match self.focus {
            Focus::Editor => {
                if self.editor.has_file() {
                    self.editor.paste(text);
                }
            }
            Focus::Output => {
                let proxied = crate::editor::sanitize_text(text).replace('\n', "\r");
                if !proxied.is_empty() {
                    self.runner.send_input(proxied.as_bytes());
                }
            }
            Focus::Tree => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != crossterm::event::KeyEventKind::Press {
            return;
        }
        if self.bindings.matches(Action::Quit, &key) {
            if self.focus == Focus::Editor {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.return_to_tree();
                } else {
                    self.begin_quit();
                }
            } else {
                self.begin_quit();
            }
            return;
        }
        if self.modal != Modal::None {
            self.handle_modal_key(key);
            return;
        }
        if self.focus == Focus::Output {
            self.handle_output_key(key);
            return;
        }
        if key.code == KeyCode::PageUp {
            self.runner.output.page_up();
            return;
        }
        if key.code == KeyCode::PageDown {
            self.runner.output.page_down();
            return;
        }
        if self.input_mode != InputMode::Normal {
            self.handle_prompt_key(key);
            return;
        }
        if self.bindings.matches(Action::FocusOutput, &key) {
            self.toggle_output_focus();
            return;
        }
        if self.bindings.matches(Action::FocusTree, &key) {
            self.focus = if self.focus == Focus::Tree {
                Focus::Editor
            } else {
                Focus::Tree
            };
            return;
        }
        if self.bindings.matches(Action::Save, &key) {
            self.save_current();
            return;
        }
        if self.bindings.matches(Action::Run, &key) {
            self.run_selected();
            return;
        }
        if self.bindings.matches(Action::Find, &key) {
            self.begin_find();
            return;
        }
        if self.bindings.matches(Action::FindNext, &key) {
            self.find_next();
            return;
        }
        if self.bindings.matches(Action::Goto, &key) {
            self.input_mode = InputMode::Goto;
            self.prompt.clear();
            return;
        }
        if self.bindings.matches(Action::Copy, &key) {
            self.copy_selection();
            return;
        }
        if self.bindings.matches(Action::Paste, &key) {
            self.paste_clipboard();
            return;
        }
        if self.bindings.matches(Action::Undo, &key) {
            self.undo();
            return;
        }
        if self.bindings.matches(Action::Redo, &key) {
            self.redo();
            return;
        }
        if self.bindings.matches(Action::NewFile, &key) {
            self.begin_new_file();
            return;
        }
        if self.bindings.matches(Action::NewFolder, &key) {
            self.begin_new_folder();
            return;
        }
        if self.focus == Focus::Tree && self.bindings.matches(Action::Delete, &key) {
            self.begin_delete_selected();
            return;
        }
        if self.focus == Focus::Tree && self.bindings.matches(Action::Rename, &key) {
            self.begin_rename_selected();
            return;
        }
        if self.bindings.matches(Action::Workdir, &key) {
            self.make_workdir();
            return;
        }
        if self.bindings.matches(Action::Settings, &key) {
            self.begin_settings();
            return;
        }
        if self.bindings.matches(Action::Refresh, &key) {
            self.tree.refresh();
            self.set_notice("Дерево обновлено", false);
            return;
        }

        match self.focus {
            Focus::Tree => self.handle_tree_key(key),
            Focus::Editor => self.handle_editor_key(key),
            Focus::Output => {}
        }
    }

    fn handle_output_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
        {
            if self.bindings.matches(Action::FocusOutput, &key) {
                self.toggle_output_focus();
                return;
            }
            match key.code {
                KeyCode::Char('c') => {
                    self.runner.send_input(&[0x03]);
                    return;
                }
                KeyCode::Char('d') => {
                    self.runner.send_input(&[0x04]);
                    return;
                }
                // crossterm maps 0x1c to Char('4') + CONTROL
                KeyCode::Char('4') | KeyCode::Char('\\') => {
                    self.runner.send_input(&[0x1c]);
                    return;
                }
                _ => {}
            }
        }
        let sequence: Option<Vec<u8>> = match key.code {
            KeyCode::Enter => Some(vec![b'\r']),
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                Some(ch.to_string().into_bytes())
            }
            KeyCode::Backspace => Some(vec![0x7f]),
            KeyCode::Tab => Some(vec![b'\t']),
            KeyCode::Esc => Some(vec![0x1b]),
            KeyCode::Up => Some(b"\x1b[A".to_vec()),
            KeyCode::Down => Some(b"\x1b[B".to_vec()),
            KeyCode::Right => Some(b"\x1b[C".to_vec()),
            KeyCode::Left => Some(b"\x1b[D".to_vec()),
            KeyCode::Home => Some(b"\x1b[H".to_vec()),
            KeyCode::End => Some(b"\x1b[F".to_vec()),
            KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
            KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
            KeyCode::Insert => Some(b"\x1b[2~".to_vec()),
            KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
            _ => None,
        };
        if let Some(bytes) = sequence {
            self.runner.send_input(&bytes);
        }
    }

    fn toggle_output_focus(&mut self) {
        if self.runner.is_running() {
            self.focus = if self.focus == Focus::Output {
                Focus::Editor
            } else {
                Focus::Output
            };
        } else {
            self.focus = if self.focus == Focus::Output {
                Focus::Editor
            } else {
                Focus::Tree
            };
        }
        self.input_mode = InputMode::Normal;
        self.prompt.clear();
    }

    fn handle_tree_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.tree.move_up(),
            KeyCode::Down => self.tree.move_down(),
            KeyCode::Home => self.tree.selected = 0,
            KeyCode::End => {
                if !self.tree.entries.is_empty() {
                    self.tree.selected = self.tree.entries.len() - 1;
                }
            }
            KeyCode::Right => {
                if let Some(entry) = self.tree.selected_entry() {
                    if entry.is_dir {
                        self.tree.expand_selected();
                    } else {
                        let path = entry.path.clone();
                        self.open_file(&path);
                    }
                }
            }
            KeyCode::Enter => self.activate_tree_selection(),
            KeyCode::Left => self.tree.collapse_or_parent(),
            KeyCode::Char('q') if key.modifiers.is_empty() => self.begin_quit_confirm(),
            KeyCode::Char('w') if key.modifiers.is_empty() => self.make_workdir(),
            KeyCode::Tab => {
                if self.runner.is_running() {
                    self.toggle_output_focus();
                } else {
                    self.focus = Focus::Editor;
                }
            }
            _ => {}
        }
    }

    fn handle_editor_key(&mut self, key: KeyEvent) {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if self.bindings.matches(Action::AcceptGhost, &key) {
            if self.editor.has_file() && !self.editor.accept_ghost() {
                self.editor.insert_tab();
            }
            return;
        }
        if self.bindings.matches(Action::WrapToggle, &key) {
            self.editor.toggle_wrap();
            let state = if self.editor.wrap {
                "вкл"
            } else {
                "выкл"
            };
            self.set_notice(format!("Перенос строк: {state}"), false);
            return;
        }
        if key.code == KeyCode::Tab && key.modifiers.is_empty() && self.runner.is_running() {
            self.toggle_output_focus();
            return;
        }
        match key.code {
            KeyCode::Up => self.editor.move_up(shift),
            KeyCode::Down => self.editor.move_down(shift),
            KeyCode::Left => self.editor.move_left(shift),
            KeyCode::Right => self.editor.move_right(shift),
            KeyCode::Home => self.editor.move_home(shift),
            KeyCode::End => self.editor.move_end(shift),
            KeyCode::Enter => {
                if self.editor.has_file() {
                    self.editor.insert_newline();
                }
            }
            KeyCode::Backspace => {
                if self.editor.has_file() {
                    self.editor.backspace();
                }
            }
            KeyCode::Delete => {
                if self.editor.has_file() {
                    self.editor.delete_forward();
                }
            }
            KeyCode::Esc => {
                if self.editor.ghost().is_some() {
                    self.editor.dismiss_ghost();
                } else if self.editor.selection().is_some() {
                    self.editor.anchor = None;
                } else {
                    self.return_to_tree();
                }
            }
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.return_to_tree();
            }
            KeyCode::BackTab => self.focus = Focus::Tree,
            KeyCode::Char(ch)
                if self.editor.has_file()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.editor.insert_char(ch);
            }
            _ => {}
        }
    }

    fn return_to_tree(&mut self) {
        self.input_mode = InputMode::Normal;
        self.prompt.clear();
        if self.editor.has_file() && self.editor.modified {
            self.modal = Modal::SaveConfirm {
                then_quit: false,
                pending_focus_tree: true,
            };
            return;
        }
        self.focus = Focus::Tree;
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.prompt.clear();
            }
            KeyCode::Backspace => {
                self.prompt.pop();
            }
            KeyCode::Enter => {
                let kind = self.input_mode;
                self.input_mode = InputMode::Normal;
                let text = self.prompt.trim().to_string();
                self.prompt.clear();
                match kind {
                    InputMode::Find => self.commit_find(text),
                    InputMode::Goto => self.commit_goto(text),
                    InputMode::Normal => {}
                }
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.prompt.push(ch);
            }
            _ => {}
        }
    }

    fn commit_find(&mut self, query: String) {
        if query.is_empty() {
            self.set_notice("Поиск отменён", false);
            return;
        }
        self.editor.set_search(Some(query.clone()));
        match self.editor.find_next(&query, false) {
            Some((line, column)) => {
                self.last_search = query;
                self.focus = Focus::Editor;
                self.set_notice(
                    format!("Найдено: строка {}, колонка {}", line + 1, column + 1),
                    false,
                );
            }
            None => {
                self.last_search = query.clone();
                self.set_notice(format!("Не найдено: {query}"), true);
            }
        }
    }

    fn commit_goto(&mut self, text: String) {
        if text.is_empty() {
            self.set_notice("Переход отменён", false);
            return;
        }
        let Ok(line) = text.parse::<usize>() else {
            self.set_notice("Номер строки должен быть числом", true);
            return;
        };
        if line == 0 {
            self.set_notice("Строки нумеруются с единицы", true);
            return;
        }
        self.editor.goto_line(line);
        self.focus = Focus::Editor;
        self.set_notice(format!("Строка {line}"), false);
    }

    fn begin_find(&mut self) {
        self.input_mode = InputMode::Find;
        self.prompt.clear();
        self.focus = Focus::Editor;
    }

    fn find_next(&mut self) {
        if self.last_search.is_empty() {
            self.set_notice("Сначала выполните поиск через Ctrl+F", false);
            return;
        }
        let query = self.last_search.clone();
        if self.editor.find_next(&query, true).is_none() {
            self.set_notice(format!("Не найдено: {query}"), true);
        }
    }

    fn copy_selection(&mut self) {
        if self.focus != Focus::Editor {
            return;
        }
        if self.editor.copy_to_clipboard(&mut self.clipboard) {
            self.set_notice("Скопировано", false);
        } else {
            self.set_notice("Нечего копировать", false);
        }
    }

    fn paste_clipboard(&mut self) {
        if self.focus != Focus::Editor || !self.editor.has_file() {
            return;
        }
        if self.clipboard.is_empty() {
            self.set_notice("Буфер обмена пуст", false);
            return;
        }
        let clipboard = self.clipboard.clone();
        if self.editor.paste(&clipboard) {
            self.set_notice("Вставлено", false);
        }
    }

    fn undo(&mut self) {
        if self.editor.undo() {
            self.set_notice("Отменено", false);
        } else {
            self.set_notice("Отменять нечего", false);
        }
    }

    fn redo(&mut self) {
        if self.editor.redo() {
            self.set_notice("Повторено", false);
        } else {
            self.set_notice("Повторять нечего", false);
        }
    }

    fn save_current(&mut self) {
        if !self.editor.has_file() {
            self.set_notice("Нет открытого файла", true);
            return;
        }
        match self.editor.save() {
            Ok(()) => self.set_notice("Файл сохранён", false),
            Err(error) => {
                logging::error(&error);
                self.set_notice(error, true);
            }
        }
    }

    fn save_if_needed(&mut self) -> bool {
        if !self.editor.has_file() || !self.editor.modified {
            return true;
        }
        match self.editor.save() {
            Ok(()) => true,
            Err(error) => {
                logging::error(&error);
                self.set_notice(error, true);
                false
            }
        }
    }

    fn begin_quit(&mut self) {
        if self.editor.has_file() && self.editor.modified {
            self.modal = Modal::SaveConfirm {
                then_quit: true,
                pending_focus_tree: false,
            };
            return;
        }
        self.begin_quit_confirm();
    }

    fn begin_quit_confirm(&mut self) {
        self.modal = Modal::QuitConfirm;
    }

    fn open_file(&mut self, path: &Path) {
        if self.editor.path.as_deref() == Some(path) {
            self.focus = Focus::Editor;
            return;
        }
        if !self.save_if_needed() {
            return;
        }
        match Editor::open(path, 4) {
            Ok(editor) => {
                let cursor_mode = self.cursor_mode;
                let mut editor = editor;
                editor.cursor_mode = cursor_mode;
                self.editor = editor;
                self.highlighter = Highlighter::default();
                self.tree.reveal_file(path);
                self.focus = Focus::Editor;
                self.input_mode = InputMode::Normal;
                self.set_notice(
                    format!("Открыт {}", self.tree.relative_display(path)),
                    false,
                );
            }
            Err(error) => {
                logging::error(&error);
                self.set_notice(error, true);
            }
        }
    }

    fn activate_tree_selection(&mut self) {
        let Some(entry) = self.tree.selected_entry().cloned() else {
            return;
        };
        if entry.is_dir {
            self.tree.toggle_selected();
        } else {
            self.open_file(&entry.path);
        }
    }

    fn make_workdir(&mut self) {
        let workdir = self.tree.current_directory();
        self.workdir = workdir.clone();
        self.set_notice(
            format!(
                "Рабочая директория: {}",
                self.tree.relative_display(&workdir)
            ),
            false,
        );
    }

    fn run_selected(&mut self) {
        if self.runner.is_running() {
            self.set_notice("Выполнение уже идёт: Tab — фокус ввода/вывода", false);
            self.focus = Focus::Output;
            return;
        }
        let target = match self.focus {
            Focus::Editor => self.editor.path.clone().or_else(|| {
                self.tree
                    .selected_entry()
                    .filter(|entry| !entry.is_dir)
                    .map(|entry| entry.path.clone())
            }),
            Focus::Tree => match self.tree.selected_entry() {
                Some(entry) if !entry.is_dir => Some(entry.path.clone()),
                Some(entry) if entry.is_dir => {
                    self.set_notice("Для запуска выберите файл, а не папку", true);
                    None
                }
                _ => None,
            },
            Focus::Output => None,
        };
        let Some(target) = target else {
            self.set_notice("Нет файла для запуска", true);
            return;
        };
        if !target.is_file() {
            self.set_notice(format!("{} не является файлом", target.display()), true);
            return;
        }
        if !self.save_if_needed() {
            return;
        }
        let extension = target
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if extension.is_empty() {
            if let Some(command) = runner::shebang_command(&target) {
                let plan = RunPlan {
                    file: target.clone(),
                    cwd: self.workdir.clone(),
                    command,
                };
                self.start_embedded(plan);
                return;
            }
            self.modal = Modal::RuntimeSelect {
                target,
                selected: 0,
                error: None,
            };
            return;
        }
        let plan = match runner::build_plan(&target, &self.run_rules, &self.workdir) {
            Ok(plan) => plan,
            Err(error) => {
                logging::error(&error);
                self.runner.output.add_error(error.clone());
                self.set_notice(error, true);
                return;
            }
        };
        self.start_embedded(plan);
    }

    fn start_embedded(&mut self, plan: RunPlan) {
        match self.runner.start(&plan) {
            Ok(()) => {
                self.set_notice(
                    format!("Запущено {}. Tab — фокус на вывод", plan.file.display()),
                    false,
                );
                self.focus = Focus::Output;
            }
            Err(error) => {
                logging::error(&error);
                self.runner.output.add_error(error.clone());
                self.set_notice(error, true);
            }
        }
    }

    fn begin_new_file(&mut self) {
        let folder = self.workdir.clone();
        self.modal = Modal::NewFileName {
            folder,
            input: String::new(),
            error: None,
        };
    }

    fn begin_new_folder(&mut self) {
        let parent = self.workdir.clone();
        self.modal = Modal::NewFolderName {
            parent,
            input: String::new(),
            error: None,
        };
    }

    fn begin_settings(&mut self) {
        self.modal = Modal::SettingsSelect {
            selected: match self.editor.cursor_mode {
                crate::editor::CursorMode::Block => 0,
                crate::editor::CursorMode::Underline => 1,
                crate::editor::CursorMode::Letters => 2,
            },
            error: None,
        };
    }

    fn apply_cursor_mode(&mut self, index: usize) {
        let mode = match index {
            0 => crate::editor::CursorMode::Block,
            1 => crate::editor::CursorMode::Underline,
            _ => crate::editor::CursorMode::Letters,
        };
        self.cursor_mode = mode;
        self.editor.cursor_mode = mode;
        persist_cursor_setting(mode);
        self.modal = Modal::None;
        self.set_notice(format!("Курсор: {}", mode.label()), false);
    }

    fn begin_rename_selected(&mut self) {
        let Some(entry) = self.tree.selected_entry() else {
            return;
        };
        if entry.is_dir && entry.path == self.workdir {
            self.set_notice("Нельзя переименовать рабочую директорию", true);
            return;
        }
        let input = self.tree.relative_display(&entry.path);
        self.modal = Modal::RenamePath {
            path: entry.path.clone(),
            is_dir: entry.is_dir,
            input,
            error: None,
        };
    }

    fn begin_delete_selected(&mut self) {
        let Some(entry) = self.tree.selected_entry() else {
            return;
        };
        if entry.is_dir && entry.path == self.workdir {
            self.set_notice("Нельзя удалить рабочую директорию", true);
            return;
        }
        self.modal = Modal::DeleteConfirm {
            path: entry.path.clone(),
            is_dir: entry.is_dir,
        };
    }

    fn perform_delete(&mut self, path: &Path, is_dir: bool) {
        let result = if is_dir {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        self.modal = Modal::None;
        match result {
            Ok(()) => {
                let name = self.tree.relative_display(path);
                let editor_inside = self
                    .editor
                    .path
                    .as_ref()
                    .is_some_and(|current| current == path || current.starts_with(path));
                if editor_inside {
                    self.editor = Editor::default();
                    self.editor.cursor_mode = self.cursor_mode;
                }
                self.tree.refresh();
                self.set_notice(format!("Удалено: {name}"), false);
            }
            Err(error) => {
                let message = format!("Не удалось удалить: {error}");
                logging::error(&message);
                self.set_notice(message, true);
            }
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent) {
        let modal = std::mem::replace(&mut self.modal, Modal::None);
        match modal {
            Modal::None => {}
            Modal::NewFileName {
                folder,
                mut input,
                mut error,
            } => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Enter => match validate_new_name(&input) {
                        Ok(()) => {
                            let trimmed = input.trim().to_string();
                            let has_extension =
                                trimmed.rsplit_once('.').is_some_and(|(base, extension)| {
                                    !base.is_empty() && !extension.is_empty()
                                });
                            if has_extension {
                                self.create_file(&folder, &trimmed);
                                return;
                            }
                            let base = trimmed.trim_end_matches('.').to_string();
                            if base.is_empty() {
                                error = Some("Имя файла не может быть пустым".to_string());
                            } else {
                                self.modal = Modal::ExtensionSelect {
                                    folder,
                                    base,
                                    original_name: trimmed,
                                    selected: 0,
                                    error: None,
                                };
                                return;
                            }
                        }
                        Err(message) => error = Some(message),
                    },
                    KeyCode::Backspace => {
                        input.pop();
                        error = None;
                    }
                    KeyCode::Char(ch)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        input.push(ch);
                        error = None;
                    }
                    _ => {}
                }
                self.modal = Modal::NewFileName {
                    folder,
                    input,
                    error,
                };
            }
            Modal::NewFolderName {
                parent,
                mut input,
                mut error,
            } => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Enter => match validate_new_name(&input) {
                        Ok(()) => match fs::create_dir(parent.join(input.trim())) {
                            Ok(()) => {
                                let created = parent.join(input.trim());
                                self.tree.refresh_reveal(&created);
                                self.set_notice(format!("Папка создана: {}", input.trim()), false);
                                return;
                            }
                            Err(error_value) => {
                                error = Some(format!("Не удалось создать папку: {error_value}"));
                            }
                        },
                        Err(message) => error = Some(message),
                    },
                    KeyCode::Backspace => {
                        input.pop();
                        error = None;
                    }
                    KeyCode::Char(ch)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        input.push(ch);
                        error = None;
                    }
                    _ => {}
                }
                self.modal = Modal::NewFolderName {
                    parent,
                    input,
                    error,
                };
            }
            Modal::ExtensionSelect {
                folder,
                base,
                original_name,
                mut selected,
                error,
            } => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Up | KeyCode::Left => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Right => {
                        selected = (selected + 1).min(EXTENSIONS.len());
                    }
                    KeyCode::Enter => {
                        if selected == 0 {
                            let name = base.clone();
                            self.create_file(&folder, &name);
                            return;
                        }
                        let name = format!("{}.{}", base, EXTENSIONS[selected - 1]);
                        self.create_file(&folder, &name);
                        return;
                    }
                    _ => {}
                }
                self.modal = Modal::ExtensionSelect {
                    folder,
                    base,
                    original_name,
                    selected,
                    error,
                };
            }
            Modal::RuntimeSelect {
                target,
                mut selected,
                mut error,
            } => {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Up | KeyCode::Left => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Right => {
                        selected = (selected + 1).min(RUNTIME_HINTS.len() - 1);
                    }
                    KeyCode::Enter => {
                        let (extension, hint) = RUNTIME_HINTS[selected];
                        if let Some(required) = hint {
                            if platform::find_program(required).is_none() {
                                error = Some(format!(
                                    "не установлено: {required} (pkg install {required})"
                                ));
                                self.modal = Modal::RuntimeSelect {
                                    target,
                                    selected,
                                    error,
                                };
                                return;
                            }
                        }
                        match runner::plan_for_extension(
                            &target,
                            extension,
                            &self.run_rules,
                            &self.workdir,
                        ) {
                            Ok(plan) => {
                                self.start_embedded(plan);
                                return;
                            }
                            Err(message) => error = Some(message),
                        }
                    }
                    _ => {}
                }
                self.modal = Modal::RuntimeSelect {
                    target,
                    selected,
                    error,
                };
            }
            Modal::SettingsSelect {
                mut selected,
                error,
            } => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Up | KeyCode::Left => selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Right => selected = (selected + 1).min(2),
                    KeyCode::Enter => {
                        self.apply_cursor_mode(selected);
                        return;
                    }
                    _ => {}
                }
                self.modal = Modal::SettingsSelect {
                    selected,
                    error,
                };
            }
            Modal::DeleteConfirm { path, is_dir } => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.perform_delete(&path, is_dir);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') | KeyCode::Esc => {}
                _ => {}
            },
            Modal::RenamePath {
                path,
                is_dir,
                mut input,
                mut error,
            } => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Enter => match validate_rename_path(&input) {
                        Ok(()) => {
                            let target = self.workdir.join(input.trim());
                            if target == path {
                                self.set_notice("Путь не изменился", false);
                                return;
                            }
                            if is_dir && target.starts_with(&path) {
                                error = Some("Нельзя перенести папку внутрь её самой".to_string());
                                self.modal = Modal::RenamePath {
                                    path,
                                    is_dir,
                                    input,
                                    error,
                                };
                                return;
                            }
                            if target.exists() {
                                error = Some(format!(
                                    "уже существует: {}",
                                    self.tree.relative_display(&target)
                                ));
                                self.modal = Modal::RenamePath {
                                    path,
                                    is_dir,
                                    input,
                                    error,
                                };
                                return;
                            }
                            let display = self.tree.relative_display(&target);
                            match fs::rename(&path, &target) {
                                Ok(()) => {
                                    self.tree.refresh_reveal(&target);
                                    if let Some(current) = self.editor.path.as_mut() {
                                        if *current == path {
                                            *current = target.clone();
                                        } else if is_dir && current.starts_with(&path) {
                                            if let Ok(rest) = current.strip_prefix(&path) {
                                                *current = target.join(rest);
                                            }
                                        }
                                    }
                                    self.set_notice(format!("Перемещено: {display}"), false);
                                    return;
                                }
                                Err(err) => {
                                    error = Some(format!("Не удалось переместить: {err}"));
                                }
                            }
                        }
                        Err(message) => error = Some(message),
                    },
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        input.push(ch)
                    }
                    _ => {}
                }
                self.modal = Modal::RenamePath {
                    path,
                    is_dir,
                    input,
                    error,
                };
            }
            Modal::SaveConfirm {
                then_quit,
                pending_focus_tree,
            } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    let _ = pending_focus_tree;
                    match self.editor.save() {
                        Ok(()) => {
                            if then_quit {
                                self.should_quit = true;
                            } else {
                                self.focus = Focus::Tree;
                            }
                        }
                        Err(error) => {
                            logging::error(&error);
                            self.set_notice(error, true);
                            self.focus = Focus::Editor;
                        }
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.editor.modified = false;
                    if then_quit {
                        self.should_quit = true;
                    } else {
                        self.focus = Focus::Tree;
                    }
                }
                _ => {}
            },
            Modal::QuitConfirm => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.should_quit = true;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {}
                _ => {}
            },
        }
    }

    fn create_file(&mut self, folder: &Path, file_name: &str) {
        let path = folder.join(file_name);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) => {
                let message = format!("Не удалось создать {file_name}: {error}");
                self.modal = Modal::NewFileName {
                    folder: folder.to_path_buf(),
                    input: file_name.to_string(),
                    error: Some(message.clone()),
                };
                self.set_notice(message, true);
                return;
            }
        };
        if let Err(error) = file.flush() {
            self.set_notice(format!("Не удалось записать {file_name}: {error}"), true);
        }
        drop(file);
        if !self.save_if_needed() {
            return;
        }
        self.editor = Editor::default();
        self.editor.cursor_mode = self.cursor_mode;
        match Editor::open(&path, 4) {
            Ok(opened) => {
                let language = opened.language.title();
                let mut opened = opened;
                opened.cursor_mode = self.cursor_mode;
                self.editor = opened;
                self.highlighter = Highlighter::default();
                self.tree.refresh_reveal(&path);
                self.modal = Modal::None;
                self.focus = Focus::Editor;
                self.set_notice(format!("Создан {file_name}; язык: {language}"), false);
            }
            Err(error) => {
                logging::error(&error);
                self.tree.refresh_reveal(&path);
                self.modal = Modal::None;
                self.focus = Focus::Tree;
                self.set_notice(format!("Файл создан, но не открыт: {error}"), true);
            }
        }
    }

    pub fn set_notice(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
            until: Instant::now()
                + if error {
                    Duration::from_secs(8)
                } else {
                    Duration::from_secs(4)
                },
        });
    }

    pub fn exit(&self) -> Option<RunExit> {
        self.runner.exit
    }

    pub fn search_query(&self) -> Option<&str> {
        self.editor.search.as_deref()
    }
}

const RUNTIME_HINTS: [(&str, Option<&str>); 8] = [
    ("py", Some("python3")),
    ("js", Some("node")),
    ("ts", Some("tsx")),
    ("sh", Some("bash")),
    ("go", Some("go")),
    ("c", Some("gcc")),
    ("cpp", Some("g++")),
    ("rs", Some("cargo")),
];

fn persist_cursor_setting(mode: crate::editor::CursorMode) {
    let path = platform::config_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let updated = upsert_cursor_setting(&text, mode.label());
    let _ = fs::write(&path, updated);
}

fn upsert_cursor_setting(text: &str, value: &str) -> String {
    let line = format!("cursor = \"{value}\"");
    let mut result: Vec<String> = Vec::new();
    let mut in_editor = false;
    let mut replaced = false;
    for row in text.lines() {
        if row.trim_start() == "[editor]" {
            in_editor = true;
            result.push(row.to_string());
            continue;
        }
        if row.trim_start().starts_with('[') {
            if in_editor && !replaced {
                result.push(line.clone());
                replaced = true;
            }
            in_editor = false;
        }
        if in_editor && row.trim_start().starts_with("cursor") {
            result.push(line.clone());
            replaced = true;
            continue;
        }
        result.push(row.to_string());
    }
    if !replaced {
        if in_editor {
            result.push(line.clone());
        } else {
            while result.last().is_some_and(|last| last.trim().is_empty()) {
                result.pop();
            }
            result.push(String::new());
            result.push("[editor]".to_string());
            result.push(line.clone());
        }
    }
    let mut out = result.join("\n");
    out.push('\n');
    out
}

pub fn validate_rename_path(input: &str) -> Result<(), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Имя не может быть пустым".to_string());
    }
    if trimmed.contains('\\') {
        return Err("Только '/' между папками".to_string());
    }
    let invalid = trimmed
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..");
    if invalid {
        return Err("Недопустимый путь".to_string());
    }
    if trimmed.chars().any(|ch| ch == '\0') {
        return Err("Символ NUL недопустим".to_string());
    }
    Ok(())
}

pub fn validate_new_name(input: &str) -> Result<(), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Имя не может быть пустым".to_string());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("Недопустимое имя".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("Имя не должно содержать разделители пути".to_string());
    }
    if trimmed.chars().any(|ch| ch == '\0') {
        return Err("Имя содержит недопустимые символы".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workdir_starts_at_root() {
        let settings = LoadedConfig::test_stub();
        let root = std::env::temp_dir().join(format!("artef-app-{}", std::process::id()));
        fs::create_dir_all(&root).ok();
        let app = App::new(settings, root.clone(), None);
        assert_eq!(app.workdir, root);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn output_focus_only_while_running() {
        let mut app = App::new(LoadedConfig::test_stub(), std::env::temp_dir(), None);
        app.focus = Focus::Output;
        app.tick();
        assert_ne!(app.focus, Focus::Output);
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("artef-{}-{}", name, std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn delete_file_flow() {
        let root = temp_root("del");
        let file = root.join("f.txt");
        fs::write(&file, b"hi").unwrap();
        let mut app = App::new(LoadedConfig::test_stub(), root.clone(), None);
        let index = app
            .tree
            .entries
            .iter()
            .position(|e| !e.is_dir)
            .expect("file in tree");
        app.tree.selected = index;
        app.begin_delete_selected();
        assert!(matches!(app.modal, Modal::DeleteConfirm { .. }));
        app.handle_modal_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::None);
        assert!(!file.exists());
    }

    #[test]
    fn delete_folder_flow() {
        let root = temp_root("deldir");
        let sub = root.join("sub");
        fs::create_dir_all(sub.join("nested")).unwrap();
        fs::write(sub.join("nested").join("x.txt"), b"x").unwrap();
        let mut app = App::new(LoadedConfig::test_stub(), root.clone(), None);
        let index = app
            .tree
            .entries
            .iter()
            .filter_map(|e| {
                if e.is_dir && e.path != root {
                    Some(0)
                } else {
                    None
                }
            })
            .next()
            .map(|_| {
                app.tree
                    .entries
                    .iter()
                    .position(|e| e.is_dir && e.path != root)
                    .unwrap()
            })
            .unwrap();
        app.tree.selected = index;
        app.begin_delete_selected();
        assert!(matches!(
            app.modal,
            Modal::DeleteConfirm { is_dir: true, .. }
        ));
        app.handle_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::None);
        assert!(!sub.exists());
    }

    #[test]
    fn delete_cancel_esc_keeps_file() {
        let root = temp_root("delcancel");
        let file = root.join("keep.txt");
        fs::write(&file, b"hi").unwrap();
        let mut app = App::new(LoadedConfig::test_stub(), root.clone(), None);
        let index = app
            .tree
            .entries
            .iter()
            .position(|e| !e.is_dir)
            .expect("file in tree");
        app.tree.selected = index;
        app.begin_delete_selected();
        app.handle_modal_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::None);
        assert!(file.exists());
    }

    #[test]
    fn delete_refuses_workdir() {
        let root = temp_root("delroot");
        let mut app = App::new(LoadedConfig::test_stub(), root.clone(), None);
        if let Some(index) = app
            .tree
            .entries
            .iter()
            .position(|e| e.is_dir && e.path == root)
        {
            app.tree.selected = index;
        }
        app.begin_delete_selected();
        assert_eq!(app.modal, Modal::None);
    }

    #[test]
    fn rename_file_flow() {
        let root = temp_root("ren");
        let file = root.join("a.txt");
        fs::write(&file, b"x").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        let mut app = App::new(LoadedConfig::test_stub(), root.clone(), None);
        let index = app
            .tree
            .entries
            .iter()
            .position(|e| e.path == file)
            .expect("file in tree");
        app.tree.selected = index;
        app.begin_rename_selected();
        assert!(matches!(app.modal, Modal::RenamePath { .. }));
        app.handle_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        // same path -> just notice
        app.modal = Modal::None;
        app.begin_rename_selected();
        let target = root.join("sub").join("b.txt");
        if let Modal::RenamePath { path, is_dir, input, error, .. } = app.modal {
            let input = "sub/b.txt".to_string();
            app.modal = Modal::RenamePath { path, is_dir, input, error };
        }
        app.handle_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::None);
        assert!(!file.exists());
        assert!(target.exists());
    }

    #[test]
    fn rename_move_dir_updates_editor_path() {
        let root = temp_root("renmov");
        fs::create_dir_all(root.join("one").join("two")).unwrap();
        let mut app = App::new(LoadedConfig::test_stub(), root.clone(), None);
        app.editor.path = Some(root.join("one").join("rep.txt"));
        let _ = fs::write(root.join("one").join("rep.txt"), b"r");
        let index = app
            .tree
            .entries
            .iter()
            .position(|e| e.is_dir && e.path == root.join("one"))
            .expect("dir in tree");
        app.tree.selected = index;
        app.begin_rename_selected();
        if let Modal::RenamePath { path, is_dir, input, error, .. } = app.modal {
            let input = "renamed".to_string();
            app.modal = Modal::RenamePath { path, is_dir, input, error };
        }
        app.handle_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::None);
        assert!(!root.join("one").exists());
        assert!(root.join("renamed").exists());
        assert_eq!(
            app.editor.path,
            Some(root.join("renamed").join("rep.txt"))
        );
    }

    #[test]
    fn rename_inside_itself_rejected() {
        let root = temp_root("renself");
        fs::create_dir_all(root.join("dir").join("deep")).unwrap();
        let mut app = App::new(LoadedConfig::test_stub(), root.clone(), None);
        let index = app
            .tree
            .entries
            .iter()
            .position(|e| e.is_dir && e.path == root.join("dir"))
            .expect("dir in tree");
        app.tree.selected = index;
        app.begin_rename_selected();
        if let Modal::RenamePath { mut input, path, is_dir, error, .. } = app.modal {
            input = "dir/deep".to_string();
            app.modal = Modal::RenamePath { path, is_dir, input, error };
        }
        app.handle_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(app.modal, Modal::RenamePath { .. }));
    }

    #[test]
    fn settings_modal_changes_cursor_mode() {
        let mut app = App::new(LoadedConfig::test_stub(), std::env::temp_dir(), None);
        app.begin_settings();
        assert!(matches!(app.modal, Modal::SettingsSelect { .. }));
        app.handle_modal_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::None);
        assert_eq!(
            app.cursor_mode,
            crate::editor::CursorMode::Underline
        );
        assert_eq!(app.editor.cursor_mode, app.cursor_mode);
    }

    #[test]
    fn cursor_upsert_config() {
        let text = "workdir = \"\n\n[theme]\nbackground = \"\"\n\n[shortcuts]\nsave = \"ctrl+s\"\n";
        let updated = upsert_cursor_setting(text, "underline");
        assert!(updated.contains("[editor]\n"));
        assert!(updated.contains("cursor = \"underline\""));
        assert!(updated.contains("[shortcuts]"));
        let upper = upsert_cursor_setting("workdir = \"\n[editor]\ncursor = \"block\"\n", "letters");
        assert!(upper.matches("cursor").count() == 1);
        assert!(upper.contains("cursor = \"letters\""));
    }

    #[test]
    fn new_file_esc_closes_modal() {
        let root = temp_root("newfiiles");
        fs::create_dir_all(root.join("sub")).unwrap();
        let mut app = App::new(LoadedConfig::test_stub(), root.clone(), None);
        let index = app
            .tree
            .entries
            .iter()
            .position(|e| e.is_dir)
            .expect("dir in tree");
        app.tree.selected = index;
        app.begin_new_file();
        assert!(matches!(app.modal, Modal::NewFileName { .. }));
        app.handle_modal_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::None);
    }
}
