use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::{Action, Bindings, LoadedConfig, ModePreference};
use crate::editor::Editor;
use crate::file_tree::FileTree;
use crate::highlight::Highlighter;
use crate::language::Language;
use crate::logging;
use crate::platform;
use crate::runner::{self, RunExit, RunPlan, Runner};
use crate::theme::Theme;

pub const EXTENSIONS: [&str; 13] = [
    "py", "js", "ts", "rs", "go", "c", "cpp", "sh", "json", "yaml", "md", "toml", "txt",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Explorer,
    Ide,
}

impl Mode {
    pub fn tag(self) -> &'static str {
        match self {
            Mode::Explorer => "EXP",
            Mode::Ide => "IDE",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Explorer => "Explorer",
            Mode::Ide => "IDE",
        }
    }

    fn other(self) -> Mode {
        match self {
            Mode::Explorer => Mode::Ide,
            Mode::Ide => Mode::Explorer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Editor,
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
    ModeSelect {
        selected: usize,
        path_input: String,
        editing_path: bool,
        error: Option<String>,
    },
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
    GuiConfirm {
        plan: RunPlan,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

pub struct App {
    pub mode: Mode,
    pub focus: Focus,
    pub tree: FileTree,
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
    pub fn new(
        settings: LoadedConfig,
        root: PathBuf,
        cli_mode: Option<Mode>,
        initial_file: Option<PathBuf>,
    ) -> Self {
        let mut file_tree = FileTree::new(root);
        let default_mode = match settings.default_mode {
            ModePreference::Ide => Some(Mode::Ide),
            ModePreference::Explorer => Some(Mode::Explorer),
            ModePreference::Ask => None,
        };
        let show_chooser = cli_mode.is_none() && default_mode.is_none();
        let mode = cli_mode.or(default_mode).unwrap_or(Mode::Explorer);
        let mut editor = Editor::default();
        let mut notice = None;
        if let Some(file) = initial_file.as_ref() {
            match Editor::open(file, 4) {
                Ok(opened) => {
                    editor = opened;
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
        let focus = if mode == Mode::Ide && editor.has_file() {
            Focus::Editor
        } else {
            Focus::Tree
        };
        let modal = if show_chooser {
            Modal::ModeSelect {
                selected: if mode == Mode::Ide { 1 } else { 0 },
                path_input: file_tree.root.display().to_string(),
                editing_path: false,
                error: None,
            }
        } else {
            Modal::None
        };
        let mut app = Self {
            mode,
            focus,
            tree: file_tree,
            editor,
            highlighter: Highlighter::default(),
            runner: Runner::new(),
            theme: settings.theme,
            bindings: settings.bindings,
            run_rules: settings.run,
            input_mode: InputMode::Normal,
            prompt: String::new(),
            last_search: String::new(),
            modal,
            notice,
            clipboard: String::new(),
            should_quit: false,
        };
        if app.modal != Modal::None {
            app.focus = Focus::Tree;
        }
        app
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
    }

    pub fn active_notice(&self) -> Option<&Notice> {
        self.notice
            .as_ref()
            .filter(|notice| Instant::now() < notice.until)
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != crossterm::event::KeyEventKind::Press {
            return;
        }
        if self.bindings.matches(Action::Quit, &key) {
            self.request_quit();
            return;
        }
        if self.modal != Modal::None {
            self.handle_modal_key(key);
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
        if self.bindings.matches(Action::ToggleMode, &key) {
            self.toggle_mode();
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
        if self.bindings.matches(Action::Refresh, &key) {
            self.tree.refresh();
            self.set_notice("Дерево обновлено", false);
            return;
        }

        match self.focus {
            Focus::Tree => self.handle_tree_key(key),
            Focus::Editor => self.handle_editor_key(key),
        }
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
            KeyCode::Tab => self.focus = Focus::Editor,
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
                } else {
                    self.editor.anchor = None;
                }
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

    fn request_quit(&mut self) {
        if self.save_if_needed() {
            self.should_quit = true;
        }
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

    fn toggle_mode(&mut self) {
        self.mode = self.mode.other();
        self.focus = match self.mode {
            Mode::Explorer => Focus::Tree,
            Mode::Ide => {
                if self.editor.has_file() {
                    Focus::Editor
                } else {
                    Focus::Tree
                }
            }
        };
        self.input_mode = InputMode::Normal;
        self.prompt.clear();
        self.set_notice(format!("Режим {}", self.mode.name()), false);
    }

    fn run_selected(&mut self) {
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
        };
        let Some(target) = target else {
            self.set_notice("Нет файла для запуска", true);
            return;
        };
        if !target.is_file() {
            self.set_notice(format!("{} не является файлом", target.display()), true);
            return;
        }
        if self.editor.path.as_deref() == Some(target.as_path()) && !self.save_if_needed() {
            return;
        }
        let plan = match runner::build_plan(&target, &self.run_rules, &self.tree.root) {
            Ok(plan) => plan,
            Err(error) => {
                logging::error(&error);
                self.runner.output.add_error(error.clone());
                self.set_notice(error, true);
                return;
            }
        };
        let language = Language::from_path(&target);
        let content = if self.editor.path.as_deref() == Some(target.as_path()) {
            self.editor.display_text()
        } else {
            fs::read_to_string(&target).unwrap_or_default()
        };
        if let Some(warning) = runner::gui_warning(&target, language, &content) {
            self.modal = Modal::GuiConfirm {
                plan,
                reason: warning.reason,
            };
            return;
        }
        self.start_embedded(plan);
    }

    fn start_embedded(&mut self, plan: RunPlan) {
        if let Err(error) = self.runner.start(&plan) {
            logging::error(&error);
            self.runner.output.add_error(error.clone());
            self.set_notice(error, true);
            return;
        }
        self.set_notice("Программа запущена", false);
    }

    fn begin_new_file(&mut self) {
        if self.mode != Mode::Ide {
            self.set_notice("Создание файлов доступно в режиме IDE (F2)", true);
            return;
        }
        let folder = self.tree.current_directory();
        self.modal = Modal::NewFileName {
            folder,
            input: String::new(),
            error: None,
        };
    }

    fn begin_new_folder(&mut self) {
        if self.mode != Mode::Ide {
            self.set_notice("Создание папок доступно в режиме IDE (F2)", true);
            return;
        }
        let parent = self.tree.current_directory();
        self.modal = Modal::NewFolderName {
            parent,
            input: String::new(),
            error: None,
        };
    }

    fn handle_modal_key(&mut self, key: KeyEvent) {
        let modal = std::mem::replace(&mut self.modal, Modal::None);
        match modal {
            Modal::None => {}
            Modal::ModeSelect {
                mut selected,
                mut path_input,
                mut editing_path,
                mut error,
            } => {
                let mut finished = false;
                if editing_path {
                    match key.code {
                        KeyCode::Esc => {
                            editing_path = false;
                            path_input = self.tree.root.display().to_string();
                            error = None;
                        }
                        KeyCode::Enter => match validate_directory(&path_input) {
                            Ok(path) => {
                                path_input = path.display().to_string();
                                editing_path = false;
                                error = None;
                            }
                            Err(message) => error = Some(message),
                        },
                        KeyCode::Backspace => {
                            path_input.pop();
                            error = None;
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            path_input.clear();
                            error = None;
                        }
                        KeyCode::Char(ch)
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
                                && !key.modifiers.contains(KeyModifiers::ALT) =>
                        {
                            path_input.push(ch);
                            error = None;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => {
                            self.should_quit = true;
                            return;
                        }
                        KeyCode::Up | KeyCode::Left => {
                            selected = selected.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Right => {
                            selected = (selected + 1).min(1);
                        }
                        KeyCode::Tab => {
                            editing_path = true;
                            error = None;
                        }
                        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            editing_path = true;
                            error = None;
                        }
                        KeyCode::Enter => {
                            match self.apply_project_and_mode(&path_input, selected) {
                                Ok(()) => {
                                    finished = true;
                                }
                                Err(message) => error = Some(message),
                            }
                        }
                        _ => {}
                    }
                }
                if !finished && !self.should_quit {
                    self.modal = Modal::ModeSelect {
                        selected,
                        path_input,
                        editing_path,
                        error,
                    };
                }
            }
            Modal::NewFileName {
                folder,
                mut input,
                mut error,
            } => {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => match validate_new_name(&input) {
                        Ok(()) => {
                            let (base, extension_index) = split_known_extension(input.trim());
                            if base.is_empty() {
                                error = Some("Имя файла не может быть пустым".to_string());
                            } else {
                                self.modal = Modal::ExtensionSelect {
                                    folder,
                                    base,
                                    original_name: input.trim().to_string(),
                                    selected: extension_index,
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
                    KeyCode::Esc => {}
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
                mut error,
            } => {
                match key.code {
                    KeyCode::Esc => {
                        self.modal = Modal::NewFileName {
                            folder,
                            input: original_name,
                            error: None,
                        };
                        return;
                    }
                    KeyCode::Up | KeyCode::Left => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Right => {
                        selected = (selected + 1).min(EXTENSIONS.len() - 1);
                    }
                    KeyCode::Enter => {
                        if let Err(message) = self.create_file(&folder, &base, EXTENSIONS[selected])
                        {
                            error = Some(message);
                        } else {
                            return;
                        }
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
            Modal::GuiConfirm { plan, reason } => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.launch_external(plan);
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    let _ = reason;
                    self.runner.output.add_system("Запуск отменён.");
                }
                _ => {
                    self.modal = Modal::GuiConfirm { plan, reason };
                }
            },
        }
    }

    fn apply_project_and_mode(&mut self, path_input: &str, selected: usize) -> Result<(), String> {
        let path = validate_directory(path_input)?;
        let changed = path != self.tree.root;
        if changed {
            self.runner.shutdown();
            self.runner.output.clear();
            self.tree.set_root(path.clone());
            self.editor = Editor::default();
            self.highlighter = Highlighter::default();
            self.clipboard.clear();
            self.runner.output.clear();
            self.runner.exit = None;
        }
        self.mode = if selected == 1 {
            Mode::Ide
        } else {
            Mode::Explorer
        };
        self.modal = Modal::None;
        self.focus = if self.mode == Mode::Ide && self.editor.has_file() {
            Focus::Editor
        } else {
            Focus::Tree
        };
        if changed {
            self.set_notice(format!("Открыт проект {}", path.display()), false);
        } else {
            self.set_notice(format!("Режим {}", self.mode.name()), false);
        }
        Ok(())
    }

    fn create_file(&mut self, folder: &Path, base: &str, extension: &str) -> Result<(), String> {
        if !self.save_if_needed() {
            return Err("Сначала сохраните текущий файл".to_string());
        }
        let file_name = format!("{base}.{extension}");
        let path = folder.join(&file_name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("Не удалось создать {file_name}: {error}"))?;
        file.flush()
            .map_err(|error| format!("Не удалось записать {file_name}: {error}"))?;
        drop(file);
        let opened = Editor::open(&path, 4)
            .map_err(|error| format!("Файл создан, но не открыт: {error}"))?;
        self.editor = opened;
        self.highlighter = Highlighter::default();
        self.tree.refresh_reveal(&path);
        self.modal = Modal::None;
        self.focus = Focus::Editor;
        self.set_notice(
            format!("Создан {file_name}; язык: {}", self.editor.language.title()),
            false,
        );
        Ok(())
    }

    fn launch_external(&mut self, plan: RunPlan) {
        match platform::launch_external(&plan.command, &plan.cwd) {
            Ok(terminal) => {
                self.runner.output.clear();
                self.runner.output.add_system(format!("$ {}", plan.command));
                self.runner
                    .output
                    .add_system(format!("Запущено во внешнем терминале: {terminal}"));
                self.runner.exit = None;
                self.set_notice("Программа запущена во внешнем терминале", false);
            }
            Err(error) => {
                logging::error(&error);
                self.runner.output.add_error(error.clone());
                self.set_notice(error, true);
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

    pub fn status_path(&self) -> String {
        if let Some(path) = &self.editor.path {
            self.tree.relative_display(path)
        } else {
            self.tree.relative_display(&self.tree.root)
        }
    }

    pub fn search_query(&self) -> Option<&str> {
        self.editor.search.as_deref()
    }
}

pub fn validate_directory(input: &str) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Путь к проекту пуст".to_string());
    }
    let expanded = if trimmed == "~" {
        platform::home_dir()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        platform::home_dir().join(rest)
    } else {
        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|error| format!("Не удалось определить текущий каталог: {error}"))?
                .join(path)
        }
    };
    let canonical =
        fs::canonicalize(&expanded).map_err(|error| format!("Каталог недоступен: {error}"))?;
    if !canonical.is_dir() {
        return Err(format!("{} не является каталогом", canonical.display()));
    }
    Ok(canonical)
}

fn validate_new_name(input: &str) -> Result<(), String> {
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
    if trimmed
        .chars()
        .any(|ch| ch == '\0' || "<>:\"|?*".contains(ch))
    {
        return Err("Имя содержит недопустимые символы".to_string());
    }
    Ok(())
}

fn split_known_extension(input: &str) -> (String, usize) {
    let lower = input.to_ascii_lowercase();
    for (index, extension) in EXTENSIONS.iter().enumerate() {
        let suffix = format!(".{extension}");
        if lower.ends_with(&suffix) && input.len() > suffix.len() {
            return (input[..input.len() - suffix.len()].to_string(), index);
        }
    }
    (input.to_string(), 0)
}
