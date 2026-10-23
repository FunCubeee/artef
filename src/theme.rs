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

#[derive(Debug, Clone)]
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

impl Default for ThemeFile {
    fn default() -> Self {
        Self {
            background: "#1d2021".to_string(),
            panel: "#282828".to_string(),
            border: "#3c3836".to_string(),
            active_bg: "#32302f".to_string(),
            accent: "#b8bb26".to_string(),
            folder: "#fabd2f".to_string(),
            file: "#ebdbb2".to_string(),
            python: "#458588".to_string(),
            rust: "#d65d0e".to_string(),
            javascript: "#b8bb26".to_string(),
            go: "#83a598".to_string(),
            ghost: "#504945".to_string(),
            status_bg: "#3c3836".to_string(),
            status_fg: "#ebdbb2".to_string(),
            key: "#83a598".to_string(),
            error: "#fb4934".to_string(),
            output: "#d5c4a1".to_string(),
            line_number: "#665c54".to_string(),
            selection: "#504945".to_string(),
            search: "#665c54".to_string(),
            muted: "#928374".to_string(),
            keyword: "#fb4934".to_string(),
            type_color: "#fabd2f".to_string(),
            string: "#b8bb26".to_string(),
            comment: "#928374".to_string(),
            number: "#d3869b".to_string(),
            function: "#83a598".to_string(),
            macro_color: "#fabd2f".to_string(),
            operator: "#a89984".to_string(),
            property: "#83a598".to_string(),
            heading: "#b8bb26".to_string(),
            link: "#83a598".to_string(),
            code: "#fabd2f".to_string(),
            tag: "#8ec07c".to_string(),
            key_color: "#83a598".to_string(),
        }
    }
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
        let fallback = Theme::fallback();
        Self {
            background: parse_color("background", &file.background, fallback.background),
            panel: parse_color("panel", &file.panel, fallback.panel),
            border: parse_color("border", &file.border, fallback.border),
            active_bg: parse_color("active_bg", &file.active_bg, fallback.active_bg),
            accent: parse_color("accent", &file.accent, fallback.accent),
            folder: parse_color("folder", &file.folder, fallback.folder),
            file: parse_color("file", &file.file, fallback.file),
            python: parse_color("python", &file.python, fallback.python),
            rust: parse_color("rust", &file.rust, fallback.rust),
            javascript: parse_color("javascript", &file.javascript, fallback.javascript),
            go: parse_color("go", &file.go, fallback.go),
            ghost: parse_color("ghost", &file.ghost, fallback.ghost),
            status_bg: parse_color("status_bg", &file.status_bg, fallback.status_bg),
            status_fg: parse_color("status_fg", &file.status_fg, fallback.status_fg),
            key: parse_color("key", &file.key, fallback.key),
            error: parse_color("error", &file.error, fallback.error),
            output: parse_color("output", &file.output, fallback.output),
            line_number: parse_color("line_number", &file.line_number, fallback.line_number),
            selection: parse_color("selection", &file.selection, fallback.selection),
            search: parse_color("search", &file.search, fallback.search),
            muted: parse_color("muted", &file.muted, fallback.muted),
            keyword: parse_color("keyword", &file.keyword, fallback.keyword),
            type_color: parse_color("type", &file.type_color, fallback.type_color),
            string: parse_color("string", &file.string, fallback.string),
            comment: parse_color("comment", &file.comment, fallback.comment),
            number: parse_color("number", &file.number, fallback.number),
            function: parse_color("function", &file.function, fallback.function),
            macro_color: parse_color("macro", &file.macro_color, fallback.macro_color),
            operator: parse_color("operator", &file.operator, fallback.operator),
            property: parse_color("property", &file.property, fallback.property),
            heading: parse_color("heading", &file.heading, fallback.heading),
            link: parse_color("link", &file.link, fallback.link),
            code: parse_color("code", &file.code, fallback.code),
            tag: parse_color("tag", &file.tag, fallback.tag),
            key_color: parse_color("key_color", &file.key_color, fallback.key_color),
        }
    }

    pub fn fallback() -> Self {
        Self {
            background: Color::Rgb(0x1d, 0x20, 0x21),
            panel: Color::Rgb(0x28, 0x28, 0x28),
            border: Color::Rgb(0x3c, 0x38, 0x36),
            active_bg: Color::Rgb(0x32, 0x30, 0x2f),
            accent: Color::Rgb(0xb8, 0xbb, 0x26),
            folder: Color::Rgb(0xfa, 0xbd, 0x2f),
            file: Color::Rgb(0xeb, 0xdb, 0xb2),
            python: Color::Rgb(0x45, 0x85, 0x88),
            rust: Color::Rgb(0xd6, 0x5d, 0x0e),
            javascript: Color::Rgb(0xb8, 0xbb, 0x26),
            go: Color::Rgb(0x83, 0xa5, 0x98),
            ghost: Color::Rgb(0x50, 0x49, 0x45),
            status_bg: Color::Rgb(0x3c, 0x38, 0x36),
            status_fg: Color::Rgb(0xeb, 0xdb, 0xb2),
            key: Color::Rgb(0x83, 0xa5, 0x98),
            error: Color::Rgb(0xfb, 0x49, 0x34),
            output: Color::Rgb(0xd5, 0xc4, 0xa1),
            line_number: Color::Rgb(0x66, 0x5c, 0x54),
            selection: Color::Rgb(0x50, 0x49, 0x45),
            search: Color::Rgb(0x66, 0x5c, 0x54),
            muted: Color::Rgb(0x92, 0x83, 0x74),
            keyword: Color::Rgb(0xfb, 0x49, 0x34),
            type_color: Color::Rgb(0xfa, 0xbd, 0x2f),
            string: Color::Rgb(0xb8, 0xbb, 0x26),
            comment: Color::Rgb(0x92, 0x83, 0x74),
            number: Color::Rgb(0xd3, 0x86, 0x9b),
            function: Color::Rgb(0x83, 0xa5, 0x98),
            macro_color: Color::Rgb(0xfa, 0xbd, 0x2f),
            operator: Color::Rgb(0xa8, 0x99, 0x84),
            property: Color::Rgb(0x83, 0xa5, 0x98),
            heading: Color::Rgb(0xb8, 0xbb, 0x26),
            link: Color::Rgb(0x83, 0xa5, 0x98),
            code: Color::Rgb(0xfa, 0xbd, 0x2f),
            tag: Color::Rgb(0x8e, 0xc0, 0x7c),
            key_color: Color::Rgb(0x83, 0xa5, 0x98),
        }
    }
}
