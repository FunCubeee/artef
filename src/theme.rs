use ratatui::style::Color;

use crate::config::ThemeConfig;
use crate::logging;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub background: Color,
    pub panel: Color,
    pub border: Color,
    pub active_bg: Color,
    pub accent: Color,
    pub folder: Color,
    pub file: Color,
    pub python: Color,
    pub rust: Color,
    pub javascript: Color,
    pub go: Color,
    pub ghost: Color,
    pub status_bg: Color,
    pub status_fg: Color,
    pub key: Color,
    pub error: Color,
    pub output: Color,
    pub line_number: Color,
    pub selection: Color,
    pub search: Color,
    pub muted: Color,
    pub keyword: Color,
    pub type_color: Color,
    pub string: Color,
    pub comment: Color,
    pub number: Color,
    pub function: Color,
    pub macro_color: Color,
    pub operator: Color,
    pub property: Color,
    pub heading: Color,
    pub link: Color,
    pub code: Color,
    pub tag: Color,
    pub key_color: Color,
}

#[derive(Debug, Clone, Default)]
pub struct ThemeFile {
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

impl From<&ThemeConfig> for ThemeFile {
    fn from(config: &ThemeConfig) -> Self {
        Self {
            background: config.background.clone(),
            panel: config.panel.clone(),
            border: config.border.clone(),
            active_bg: config.active_bg.clone(),
            accent: config.accent.clone(),
            folder: config.folder.clone(),
            file: config.file.clone(),
            python: config.python.clone(),
            rust: config.rust.clone(),
            javascript: config.javascript.clone(),
            go: config.go.clone(),
            ghost: config.ghost.clone(),
            status_bg: config.status_bg.clone(),
            status_fg: config.status_fg.clone(),
            key: config.key.clone(),
            error: config.error.clone(),
            output: config.output.clone(),
            line_number: config.line_number.clone(),
            selection: config.selection.clone(),
            search: config.search.clone(),
            muted: config.muted.clone(),
            keyword: config.keyword.clone(),
            type_color: config.type_color.clone(),
            string: config.string.clone(),
            comment: config.comment.clone(),
            number: config.number.clone(),
            function: config.function.clone(),
            macro_color: config.macro_color.clone(),
            operator: config.operator.clone(),
            property: config.property.clone(),
            heading: config.heading.clone(),
            link: config.link.clone(),
            code: config.code.clone(),
            tag: config.tag.clone(),
            key_color: config.key_color.clone(),
        }
    }
}

fn pick_color(name: &str, value: &str, fallback: Color) -> Color {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("default")
        || trimmed.eq_ignore_ascii_case("reset")
    {
        return fallback;
    }
    parse_color(name, trimmed, fallback)
}

fn parse_color(name: &str, value: &str, fallback: Color) -> Color {
    let Some(hex) = value.strip_prefix('#') else {
        logging::warn(&format!("цвет {name}='{value}' не в формате #rrggbb"));
        return fallback;
    };
    if hex.len() != 6 {
        logging::warn(&format!("цвет {name}='{value}' не в формате #rrggbb"));
        return fallback;
    }
    let parsed = u32::from_str_radix(hex, 16);
    let Ok(value) = parsed else {
        logging::warn(&format!("цвет {name}='{value}' не разобран"));
        return fallback;
    };
    Color::Rgb(
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

impl Theme {
    pub fn from_file(file: &ThemeFile) -> Self {
        let fallback = Theme::terminal();
        Self {
            background: pick_color("background", &file.background, fallback.background),
            panel: pick_color("panel", &file.panel, fallback.panel),
            border: pick_color("border", &file.border, fallback.border),
            active_bg: pick_color("active_bg", &file.active_bg, fallback.active_bg),
            accent: pick_color("accent", &file.accent, fallback.accent),
            folder: pick_color("folder", &file.folder, fallback.folder),
            file: pick_color("file", &file.file, fallback.file),
            python: pick_color("python", &file.python, fallback.python),
            rust: pick_color("rust", &file.rust, fallback.rust),
            javascript: pick_color("javascript", &file.javascript, fallback.javascript),
            go: pick_color("go", &file.go, fallback.go),
            ghost: pick_color("ghost", &file.ghost, fallback.ghost),
            status_bg: pick_color("status_bg", &file.status_bg, fallback.status_bg),
            status_fg: pick_color("status_fg", &file.status_fg, fallback.status_fg),
            key: pick_color("key", &file.key, fallback.key),
            error: pick_color("error", &file.error, fallback.error),
            output: pick_color("output", &file.output, fallback.output),
            line_number: pick_color("line_number", &file.line_number, fallback.line_number),
            selection: pick_color("selection", &file.selection, fallback.selection),
            search: pick_color("search", &file.search, fallback.search),
            muted: pick_color("muted", &file.muted, fallback.muted),
            keyword: pick_color("keyword", &file.keyword, fallback.keyword),
            type_color: pick_color("type", &file.type_color, fallback.type_color),
            string: pick_color("string", &file.string, fallback.string),
            comment: pick_color("comment", &file.comment, fallback.comment),
            number: pick_color("number", &file.number, fallback.number),
            function: pick_color("function", &file.function, fallback.function),
            macro_color: pick_color("macro", &file.macro_color, fallback.macro_color),
            operator: pick_color("operator", &file.operator, fallback.operator),
            property: pick_color("property", &file.property, fallback.property),
            heading: pick_color("heading", &file.heading, fallback.heading),
            link: pick_color("link", &file.link, fallback.link),
            code: pick_color("code", &file.code, fallback.code),
            tag: pick_color("tag", &file.tag, fallback.tag),
            key_color: pick_color("key_color", &file.key_color, fallback.key_color),
        }
    }

    #[cfg(test)]
    pub fn fallback() -> Self {
        Theme::terminal()
    }

    fn terminal() -> Self {
        Self {
            background: Color::Reset,
            panel: Color::Reset,
            border: Color::DarkGray,
            active_bg: Color::DarkGray,
            accent: Color::Green,
            folder: Color::Yellow,
            file: Color::Reset,
            python: Color::Cyan,
            rust: Color::Red,
            javascript: Color::Green,
            go: Color::Cyan,
            ghost: Color::DarkGray,
            status_bg: Color::Reset,
            status_fg: Color::Reset,
            key: Color::Cyan,
            error: Color::Red,
            output: Color::Reset,
            line_number: Color::DarkGray,
            selection: Color::DarkGray,
            search: Color::DarkGray,
            muted: Color::Gray,
            keyword: Color::Red,
            type_color: Color::Yellow,
            string: Color::Green,
            comment: Color::DarkGray,
            number: Color::Magenta,
            function: Color::Cyan,
            macro_color: Color::Yellow,
            operator: Color::Gray,
            property: Color::Cyan,
            heading: Color::Green,
            link: Color::Cyan,
            code: Color::Yellow,
            tag: Color::Green,
            key_color: Color::Cyan,
        }
    }
}
