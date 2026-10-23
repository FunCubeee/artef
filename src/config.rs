use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

use crate::logging;
use crate::platform;
use crate::theme::{Theme, ThemeFile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModePreference {
    Ask,
    Ide,
    Explorer,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub theme: Theme,
    pub bindings: Bindings,
    pub run: BTreeMap<String, String>,
    pub default_mode: ModePreference,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub default_mode: String,
    pub theme: ThemeConfig,
    pub shortcuts: ShortcutConfig,
    pub run: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_mode: "ask".to_string(),
            theme: ThemeConfig::default(),
            shortcuts: ShortcutConfig::default(),
            run: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct ThemeConfig {
    pub background: String,
    pub panel: String,
    pub border: String,
    pub active_bg: String,
    pub accent: String,
    pub folder: String,
    pub file: String,
    pub python: String,
    pub rust: String,
    pub javascript: String,
    pub go: String,
    pub ghost: String,
    pub status_bg: String,
    pub status_fg: String,
    pub key: String,
    pub error: String,
    pub output: String,
    pub line_number: String,
    pub selection: String,
    pub search: String,
    pub muted: String,
    pub keyword: String,
    pub type_color: String,
    pub string: String,
    pub comment: String,
    pub number: String,
    pub function: String,
    pub macro_color: String,
    pub operator: String,
    pub property: String,
    pub heading: String,
    pub link: String,
    pub code: String,
    pub tag: String,
    pub key_color: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        let file = ThemeFile::default();
        Self {
            background: file.background,
            panel: file.panel,
            border: file.border,
            active_bg: file.active_bg,
            accent: file.accent,
            folder: file.folder,
            file: file.file,
            python: file.python,
            rust: file.rust,
            javascript: file.javascript,
            go: file.go,
            ghost: file.ghost,
            status_bg: file.status_bg,
            status_fg: file.status_fg,
            key: file.key,
            error: file.error,
            output: file.output,
            line_number: file.line_number,
            selection: file.selection,
            search: file.search,
            muted: file.muted,
            keyword: file.keyword,
            type_color: file.type_color,
            string: file.string,
            comment: file.comment,
            number: file.number,
            function: file.function,
            macro_color: file.macro_color,
            operator: file.operator,
            property: file.property,
            heading: file.heading,
            link: file.link,
            code: file.code,
            tag: file.tag,
            key_color: file.key_color,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct ShortcutConfig {
    pub save: String,
    pub run: String,
    pub find: String,
    pub find_next: String,
    pub goto: String,
    pub copy: String,
    pub paste: String,
    pub undo: String,
    pub redo: String,
    pub quit: String,
    pub new_file: String,
    pub new_folder: String,
    pub new_folder_alt: String,
    pub toggle_mode: String,
    pub focus_tree: String,
    pub refresh: String,
    pub accept_ghost: String,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            save: "ctrl+s".to_string(),
            run: "ctrl+r".to_string(),
            find: "ctrl+f".to_string(),
            find_next: "f3".to_string(),
            goto: "ctrl+g".to_string(),
            copy: "ctrl+c".to_string(),
            paste: "ctrl+v".to_string(),
            undo: "ctrl+z".to_string(),
            redo: "ctrl+y".to_string(),
            quit: "ctrl+q".to_string(),
            new_file: "ctrl+n".to_string(),
            new_folder: "f7".to_string(),
            new_folder_alt: "ctrl+shift+n".to_string(),
            toggle_mode: "f2".to_string(),
            focus_tree: "ctrl+b".to_string(),
            refresh: "f5".to_string(),
            accept_ghost: "tab".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Save,
    Run,
    Find,
    FindNext,
    Goto,
    Copy,
    Paste,
    Undo,
    Redo,
    Quit,
    NewFile,
    NewFolder,
    ToggleMode,
    FocusTree,
    Refresh,
    AcceptGhost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyBinding {
    pub fn matches(&self, event: &KeyEvent) -> bool {
        if event.kind != crossterm::event::KeyEventKind::Press {
            return false;
        }
        let normalized = match event.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };
        self.code == normalized && self.modifiers == event.modifiers
    }

    pub fn short_label(&self) -> String {
        match self.code {
            KeyCode::Char(c) if self.modifiers.contains(KeyModifiers::CONTROL) => {
                let mut label = String::new();
                if self.modifiers.contains(KeyModifiers::ALT) {
                    label.push_str("M-");
                }
                if self.modifiers.contains(KeyModifiers::SHIFT) {
                    label.push_str("S-");
                }
                label.push(c.to_ascii_uppercase());
                label
            }
            KeyCode::Char(c) => c.to_ascii_uppercase().to_string(),
            KeyCode::F(number) => format!("F{number}"),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => "S-Tab".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PgUp".to_string(),
            KeyCode::PageDown => "PgDn".to_string(),
            KeyCode::Backspace => "BS".to_string(),
            KeyCode::Delete => "Del".to_string(),
            _ => "?".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Bindings {
    entries: Vec<(Action, KeyBinding)>,
}

impl Bindings {
    pub fn from_config(config: &ShortcutConfig) -> Self {
        let defaults = ShortcutConfig::default();
        let entries = vec![
            (
                Action::Save,
                parse_binding(&config.save, &defaults.save, "save"),
            ),
            (
                Action::Run,
                parse_binding(&config.run, &defaults.run, "run"),
            ),
            (
                Action::Find,
                parse_binding(&config.find, &defaults.find, "find"),
            ),
            (
                Action::FindNext,
                parse_binding(&config.find_next, &defaults.find_next, "find_next"),
            ),
            (
                Action::Goto,
                parse_binding(&config.goto, &defaults.goto, "goto"),
            ),
            (
                Action::Copy,
                parse_binding(&config.copy, &defaults.copy, "copy"),
            ),
            (
                Action::Paste,
                parse_binding(&config.paste, &defaults.paste, "paste"),
            ),
            (
                Action::Undo,
                parse_binding(&config.undo, &defaults.undo, "undo"),
            ),
            (
                Action::Redo,
                parse_binding(&config.redo, &defaults.redo, "redo"),
            ),
            (
                Action::Quit,
                parse_binding(&config.quit, &defaults.quit, "quit"),
            ),
            (
                Action::NewFile,
                parse_binding(&config.new_file, &defaults.new_file, "new_file"),
            ),
            (
                Action::NewFolder,
                parse_binding(&config.new_folder, &defaults.new_folder, "new_folder"),
            ),
            (
                Action::NewFolder,
                parse_binding(
                    &config.new_folder_alt,
                    &defaults.new_folder_alt,
                    "new_folder_alt",
                ),
            ),
            (
                Action::ToggleMode,
                parse_binding(&config.toggle_mode, &defaults.toggle_mode, "toggle_mode"),
            ),
            (
                Action::FocusTree,
                parse_binding(&config.focus_tree, &defaults.focus_tree, "focus_tree"),
            ),
            (
                Action::Refresh,
                parse_binding(&config.refresh, &defaults.refresh, "refresh"),
            ),
            (
                Action::AcceptGhost,
                parse_binding(&config.accept_ghost, &defaults.accept_ghost, "accept_ghost"),
            ),
        ];
        Self { entries }
    }

    pub fn matches(&self, action: Action, event: &KeyEvent) -> bool {
        self.entries
            .iter()
            .any(|(candidate, binding)| *candidate == action && binding.matches(event))
    }

    pub fn binding(&self, action: Action) -> Option<KeyBinding> {
        self.entries
            .iter()
            .find(|(candidate, _)| *candidate == action)
            .map(|(_, binding)| *binding)
    }

    pub fn label(&self, action: Action) -> String {
        self.binding(action)
            .map(|binding| binding.short_label())
            .unwrap_or_else(|| "?".to_string())
    }
}

pub fn parse_binding(value: &str, fallback: &str, name: &str) -> KeyBinding {
    parse_binding_value(value).unwrap_or_else(|| {
        logging::warn(&format!(
            "некорректный шорткат {name}='{value}', используется '{fallback}'"
        ));
        parse_binding_value(fallback).unwrap_or(KeyBinding {
            code: KeyCode::Null,
            modifiers: KeyModifiers::NONE,
        })
    })
}

fn parse_binding_value(value: &str) -> Option<KeyBinding> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    if text == "+" {
        return Some(KeyBinding {
            code: KeyCode::Char('+'),
            modifiers: KeyModifiers::NONE,
        });
    }
    if text.eq_ignore_ascii_case("space") {
        return Some(KeyBinding {
            code: KeyCode::Char(' '),
            modifiers: KeyModifiers::NONE,
        });
    }
    let raw_parts: Vec<&str> = text.split('+').collect();
    if raw_parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let mut modifiers = KeyModifiers::NONE;
    let mut key_part: Option<&str> = None;
    for part in raw_parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.insert(KeyModifiers::CONTROL),
            "shift" => modifiers.insert(KeyModifiers::SHIFT),
            "alt" | "meta" => modifiers.insert(KeyModifiers::ALT),
            "super" | "cmd" => modifiers.insert(KeyModifiers::SUPER),
            _ => {
                if key_part.is_some() {
                    return None;
                }
                key_part = Some(part);
            }
        }
    }
    let raw_key = key_part?;
    let lower = raw_key.to_ascii_lowercase();
    let code = match lower.as_str() {
        "esc" | "escape" => KeyCode::Esc,
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "null" => KeyCode::Null,
        _ => {
            if let Some(number) = lower.strip_prefix('f') {
                if let Ok(number) = number.parse::<u8>() {
                    if (1..=24).contains(&number) {
                        KeyCode::F(number)
                    } else {
                        return None;
                    }
                } else {
                    return single_char_code(raw_key).map(|code| KeyBinding { code, modifiers });
                }
            } else {
                return single_char_code(raw_key).map(|code| KeyBinding { code, modifiers });
            }
        }
    };
    Some(KeyBinding { code, modifiers })
}

fn single_char_code(value: &str) -> Option<KeyCode> {
    let mut chars = value.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(KeyCode::Char(first.to_ascii_lowercase()))
}

pub fn load() -> LoadedConfig {
    let path = platform::config_path();
    let config = load_config(&path);
    let theme_file = ThemeFile::from(&config.theme);
    let theme = Theme::from_file(&theme_file);
    let bindings = Bindings::from_config(&config.shortcuts);
    let default_mode = match config.default_mode.to_ascii_lowercase().as_str() {
        "ide" => ModePreference::Ide,
        "explorer" | "exp" => ModePreference::Explorer,
        "ask" => ModePreference::Ask,
        other => {
            logging::warn(&format!(
                "неизвестный default_mode='{other}', используется ask"
            ));
            ModePreference::Ask
        }
    };
    LoadedConfig {
        theme,
        bindings,
        run: config.run,
        default_mode,
    }
}

fn load_config(path: &PathBuf) -> Config {
    if let Some(parent) = path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            logging::error(&format!(
                "не удалось создать каталог конфига {}: {error}",
                parent.display()
            ));
        }
    }
    if !path.exists() {
        if let Err(error) = fs::write(path, include_str!("../config.toml")) {
            logging::error(&format!(
                "не удалось записать конфиг {}: {error}",
                path.display()
            ));
        }
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            logging::error(&format!(
                "не удалось прочитать конфиг {}: {error}",
                path.display()
            ));
            return Config::default();
        }
    };
    match toml::from_str::<Config>(&text) {
        Ok(config) => config,
        Err(error) => {
            logging::error(&format!(
                "ошибка конфига {}: {error}; используются настройки по умолчанию",
                path.display()
            ));
            Config::default()
        }
    }
}
