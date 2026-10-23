use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, EXTENSIONS, Focus, InputMode, Modal};
use crate::config::Action;
use crate::editor::wrap_ranges;
use crate::highlight::SyntaxKind;
use crate::platform;
use crate::runner::{OutLine, OutSource, OutStyle};
use crate::theme::Theme;

struct Glyphs {
    selected: &'static str,
    branch: &'static str,
    branch_last: &'static str,
    vertical: &'static str,
    closed: &'static str,
    open: &'static str,
    blank: &'static str,
    modified: &'static str,
}

impl Glyphs {
    fn new(unicode: bool) -> Self {
        if unicode {
            Self {
                selected: "▎ ",
                branch: "├─ ",
                branch_last: "└─ ",
                vertical: "│  ",
                closed: "▸ ",
                open: "▾ ",
                blank: "   ",
                modified: "●",
            }
        } else {
            Self {
                selected: "> ",
                branch: "|- ",
                branch_last: "`- ",
                vertical: "|  ",
                closed: "> ",
                open: "v ",
                blank: "   ",
                modified: "*",
            }
        }
    }
}

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = app.theme;
    frame.render_widget(
        Block::new().style(Style::default().bg(theme.background)),
        area,
    );
    let status_height = if platform::is_termux() && area.height >= 8 {
        2
    } else {
        1
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(status_height)])
        .split(area);
    draw_body(frame, app, rows[0]);
    draw_status(frame, app, rows[1], status_height == 2);
    draw_modal(frame, app);
}

fn draw_body(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let unicode = use_unicode(area.width);
    if area.width >= 74 && area.height >= 10 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);
        draw_tree(frame, app, columns[0], unicode);
        let right = columns[1];
        let output_height = ((right.height as u32 * 30 / 100) as u16)
            .clamp(3, right.height.saturating_sub(3).max(3));
        let panels = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(output_height)])
            .split(right);
        draw_editor(frame, app, panels[0], unicode);
        draw_output(frame, app, panels[1]);
    } else if area.height >= 12 {
        let output_height = ((area.height as u32 / 3) as u16).clamp(3, 12);
        let panels = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(output_height)])
            .split(area);
        match app.focus {
            Focus::Tree => draw_tree(frame, app, panels[0], unicode),
            _ => draw_editor(frame, app, panels[0], unicode),
        }
        draw_output(frame, app, panels[1]);
    } else {
        match app.focus {
            Focus::Tree => draw_tree(frame, app, area, unicode),
            _ => draw_editor(frame, app, area, unicode),
        }
    }
}

fn panel_block(theme: Theme, title: Line<'static>, active: bool, focused_output: bool) -> Block<'static> {
    let border = if focused_output || active {
        theme.accent
    } else {
        theme.border
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(border))
        .title(title)
        .style(Style::default().bg(theme.panel).fg(theme.file))
}

fn draw_tree(frame: &mut Frame<'_>, app: &mut App, area: Rect, unicode: bool) {
    let theme = app.theme;
    let glyphs = Glyphs::new(unicode);
    let active = app.focus == Focus::Tree;
    let project = app
        .tree
        .root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| app.tree.root.display().to_string());
    let title = Line::from(vec![
        Span::styled(" FILES ", Style::default().fg(theme.folder)),
        Span::styled(
            format!("· {} (workdir) ", app.tree.relative_display(&app.workdir)),
            Style::default().fg(theme.muted),
        ),
        Span::styled(format!("· {project} "), Style::default().fg(theme.muted)),
    ]);
    let block = panel_block(theme, title, active, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let height = inner.height as usize;
    app.tree.ensure_visible(height);
    let mut lines = Vec::with_capacity(height);
    if app.tree.entries.is_empty() {
        lines.push(Line::from(Span::styled(
            " пусто",
            Style::default().fg(theme.muted),
        )));
    } else {
        let start = app.tree.scroll.min(app.tree.entries.len());
        let end = (start + height).min(app.tree.entries.len());
        for index in start..end {
            let entry = &app.tree.entries[index];
            let selected = index == app.tree.selected;
            let row_bg = if selected {
                theme.active_bg
            } else {
                theme.panel
            };
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut used = 0usize;
            let mut push = |text: &str, color: Color, used: &mut usize| {
                push_clipped(
                    &mut spans,
                    text,
                    Style::default().fg(color).bg(row_bg),
                    used,
                    width,
                );
            };
            if selected {
                push(glyphs.selected, theme.accent, &mut used);
            } else {
                push("  ", theme.panel, &mut used);
            }
            for more in &entry.ancestors_more {
                push(
                    if *more { glyphs.vertical } else { glyphs.blank },
                    theme.border,
                    &mut used,
                );
            }
            if entry.depth() > 0 {
                push(
                    if entry.is_last {
                        glyphs.branch_last
                    } else {
                        glyphs.branch
                    },
                    theme.border,
                    &mut used,
                );
            }
            if entry.is_dir {
                push(
                    if entry.expanded {
                        glyphs.open
                    } else {
                        glyphs.closed
                    },
                    theme.folder,
                    &mut used,
                );
            } else {
                push("  ", theme.panel, &mut used);
            }
            let color = if entry.is_dir {
                theme.folder
            } else {
                file_color(theme, &entry.name)
            };
            push(&entry.name, color, &mut used);
            if entry.is_dir {
                push("/", theme.folder, &mut used);
            }
            if selected && used < width {
                spans.push(Span::styled(
                    " ".repeat(width - used),
                    Style::default().bg(theme.active_bg),
                ));
            }
            lines.push(Line::from(spans));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

fn file_color(theme: Theme, name: &str) -> Color {
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "py" | "pyw" => theme.python,
        "rs" => theme.rust,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => theme.javascript,
        "go" => theme.go,
        _ => theme.file,
    }
}

fn push_clipped(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    style: Style,
    used: &mut usize,
    limit: usize,
) {
    if *used >= limit {
        return;
    }
    let mut clipped = String::new();
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if *used + width > limit {
            break;
        }
        clipped.push(ch);
        *used += width;
    }
    if !clipped.is_empty() {
        spans.push(Span::styled(clipped, style));
    }
}

fn use_unicode(width: u16) -> bool {
    if width < 44 {
        return false;
    }
    match std::env::var("ARTEF_ASCII") {
        Ok(value) => {
            let value = value.to_ascii_lowercase();
            !matches!(value.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => true,
    }
}

fn draw_editor(frame: &mut Frame<'_>, app: &mut App, area: Rect, unicode: bool) {
    let theme = app.theme;
    let focused = app.focus == Focus::Editor;
    let title = if app.editor.has_file() {
        let name = app
            .editor
            .file_name()
            .unwrap_or_else(|| "untitled".to_string());
        let marker = if app.editor.modified {
            format!(" {} ", Glyphs::new(unicode).modified)
        } else {
            String::new()
        };
        let wrap_mode = if app.editor.wrap { "wrap" } else { "lines" };
        Line::from(vec![
            Span::styled(format!(" {name}"), Style::default().fg(theme.file)),
            Span::styled(marker, Style::default().fg(theme.error)),
            Span::styled(
                format!("[{}] ", app.editor.language.title()),
                Style::default().fg(theme.key),
            ),
            Span::styled(
                format!("[{wrap_mode}] · L{}:{} ", app.editor.cursor_row_number(), app.editor.column_number()),
                Style::default().fg(theme.line_number),
            ),
        ])
    } else {
        Line::from(Span::styled(" EDITOR ", Style::default().fg(theme.muted)))
    };
    let block = panel_block(theme, title, focused, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if !app.editor.has_file() {
        draw_welcome(frame, app, inner);
        return;
    }

    let height = inner.height as usize;
    let width = inner.width as usize;
    let digits = app.editor.line_count().max(1).to_string().len();
    let gutter_width = if width > digits + 5 { digits + 2 } else { 0 };
    let code_width = width.saturating_sub(gutter_width);

    let wrapped = app.editor.wrap && code_width > 0;
    let cursor_active = app.focus == Focus::Editor
        && app.modal == Modal::None
        && app.input_mode == InputMode::Normal;

    let starts_snapshot: Vec<usize> = if wrapped {
        app.editor.visual_starts(code_width).to_vec()
    } else {
        let total = app.editor.line_count();
        (0..=total).collect()
    };
    let cursor_within = if wrapped {
        app.editor.cursor_row_offset()
    } else {
        0
    };
    let cursor_visual = starts_snapshot
        .get(app.editor.cursor_line)
        .copied()
        .unwrap_or(0)
        + cursor_within;
    if cursor_visual < app.editor.scroll {
        app.editor.scroll = cursor_visual;
    } else if cursor_visual >= app.editor.scroll + height {
        app.editor.scroll = cursor_visual + 1 - height;
    }
    if !wrapped {
        let cursor_x = app
            .editor
            .lines
            .get(app.editor.cursor_line)
            .map(|line| display_prefix_width(line, app.editor.cursor_col, app.editor.tab_width))
            .unwrap_or(0);
        if code_width > 0 {
            if cursor_x < app.editor.x_scroll {
                app.editor.x_scroll = cursor_x;
            } else if cursor_x >= app.editor.x_scroll + code_width {
                app.editor.x_scroll = cursor_x + 1 - code_width;
            }
        }
    }

    let total_rows = *starts_snapshot.last().unwrap_or(&0);
    let scroll = app.editor.scroll.min(total_rows.saturating_sub(1));
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(height);
    for row_index in 0..height {
        let visual = scroll + row_index;
        if visual >= total_rows {
            if gutter_width > 0 {
                lines.push(Line::from(Span::styled(
                    format!("{:>digits$} ~", "", digits = cursor_number_width(gutter_width)),
                    Style::default().fg(theme.muted).bg(theme.panel),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "~",
                    Style::default().fg(theme.muted).bg(theme.panel),
                )));
            }
            continue;
        }
        let line_index = partition_line(&starts_snapshot, visual);
        let row_in_line = visual - starts_snapshot[line_index];
        lines.push(build_editor_row(
            app,
            line_index,
            row_in_line,
            gutter_width,
            digits,
            code_width,
            wrapped,
            cursor_active,
        ));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

fn cursor_number_width(gutter_width: usize) -> usize {
    gutter_width.saturating_sub(1)
}

fn partition_line(starts: &[usize], visual: usize) -> usize {
    let limit = starts.len().saturating_sub(2);
    let mut position = starts.len().saturating_sub(1);
    for (index, value) in starts.iter().take(limit + 1).enumerate() {
        if *value > visual {
            position = index.saturating_sub(1);
            break;
        }
    }
    position.min(limit)
}

fn draw_welcome(frame: &mut Frame<'_>, app: &App, inner: Rect) {
    let theme = app.theme;
    let lines = vec![
        Line::from(Span::styled(
            "artef",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "терминальный редактор и запуск программ",
            Style::default().fg(theme.muted),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("Проект: {}", app.tree.root.display()),
            Style::default().fg(theme.file),
        )),
        Line::from(Span::styled(
            format!("Рабочая директория: {}", app.workdir.display()),
            Style::default().fg(theme.key),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Выберите файл в дереве и нажмите Enter.",
            Style::default().fg(theme.file),
        )),
        Line::from(Span::styled(
            "Ctrl+N файл · Alt+N папка · D удалить · Ctrl+W рабочая папка",
            Style::default().fg(theme.muted),
        )),
        Line::from(Span::styled(
            "Ctrl+S сохранить · Ctrl+R запустить · Ctrl+Shift+Q выход",
            Style::default().fg(theme.muted),
        )),
    ];
    let content_height = (lines.len() as u16).min(inner.height);
    let top = inner.y + inner.height.saturating_sub(content_height) / 2;
    let target = Rect::new(inner.x, top, inner.width, content_height);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(Style::default().bg(theme.panel)),
        target,
    );
}

struct DisplayCell {
    text: String,
    original: usize,
    x: usize,
    width: usize,
}

#[allow(clippy::too_many_arguments)]
fn build_editor_row(
    app: &App,
    line_index: usize,
    row_in_line: usize,
    gutter_width: usize,
    digits: usize,
    code_width: usize,
    wrapped: bool,
    cursor_active: bool,
) -> Line<'static> {
    let theme = app.theme;
    let editor = &app.editor;
    let text = editor
        .lines
        .get(line_index)
        .map(String::as_str)
        .unwrap_or("");
    let tab_width = editor.tab_width.max(1);
    let ranges = wrap_ranges(text, if wrapped { code_width } else { usize::MAX }, tab_width);
    let Some(&(row_start, row_end)) = ranges.get(row_in_line) else {
        return Line::from(Span::styled(
            " ".repeat(code_width + gutter_width),
            Style::default().bg(theme.panel),
        ));
    };

    let chars: Vec<char> = text.chars().collect();
    let mut cells: Vec<DisplayCell> = Vec::new();
    let mut x = 0usize;
    for (index, ch) in chars.iter().copied().enumerate() {
        if index < row_start {
            if ch == '\t' {
                x += tab_width - (x % tab_width);
            } else {
                x += UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
            }
            continue;
        }
        if index >= row_end {
            break;
        }
        if ch == '\t' {
            let spaces = tab_width - (x % tab_width);
            spaces_to_cells(&mut cells, index, spaces, &mut x);
            continue;
        }
        let mut width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if ch.is_control() {
            width = 1;
        }
        cells.push(DisplayCell {
            text: if ch.is_control() {
                " ".to_string()
            } else {
                ch.to_string()
            },
            original: index,
            x,
            width,
        });
        x += width;
    }

    let cursor_col = editor.cursor_col.min(chars.len());
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(code_width.min(160) + 2);
    let mut code_used = 0usize;
    let show_gutter = gutter_width > 0 && (!wrapped || row_in_line == 0);
    if show_gutter {
        let number_style = if line_index == editor.cursor_line {
            Style::default()
                .fg(theme.accent)
                .bg(theme.panel)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.line_number).bg(theme.panel)
        };
        let number = format!("{:>digits$}", line_index + 1);
        spans.push(Span::styled(number, number_style));
        let padding = gutter_width.saturating_sub(digits);
        if padding > 0 {
            spans.push(Span::styled(
                " ".repeat(padding),
                Style::default().bg(theme.panel),
            ));
        }
    } else if gutter_width > 0 {
        spans.push(Span::styled(
            " ".repeat(gutter_width),
            Style::default().bg(theme.panel),
        ));
    }

    let selection = editor.selection();
    let search = app.search_query();
    let search_len = search.map(|value| value.chars().count()).unwrap_or(0);
    let mut search_flags = vec![false; chars.len()];
    if let Some(query) = search {
        for start in editor.matches_in_line(line_index, query) {
            let end = (start + search_len).min(chars.len());
            for flag in search_flags.iter_mut().take(end).skip(start) {
                *flag = true;
            }
        }
    }
    let ghost = if cursor_active
        && line_index == editor.cursor_line
        && (!wrapped || row_end == chars.len())
    {
        editor.ghost().map(|suggestion| {
            suggestion
                .completion
                .chars()
                .map(|ch| {
                    if ch == '\n' || ch == '\r' || ch == '\t' || ch.is_control() {
                        ' '
                    } else {
                        ch
                    }
                })
                .collect::<String>()
        })
    } else {
        None
    };
    let cursor_on_row = !wrapped
        || (editor.cursor_col >= row_start && editor.cursor_col <= row_end);
    let cursor_original = if !cursor_on_row {
        None
    } else if ghost.is_some() && cursor_col > 0 {
        cursor_col.checked_sub(1)
    } else {
        Some(cursor_col).filter(|col| !chars.is_empty() || *col == 0)
    };
    let mut ghost_inserted = false;

    let x_scroll = if wrapped { 0 } else { editor.x_scroll };
    for cell in &cells {
        if let Some(ghost_text) = ghost.as_deref() {
            if !ghost_inserted && cell.original >= cursor_col {
                append_ghost(
                    &mut spans,
                    ghost_text,
                    cell.x,
                    code_width,
                    x_scroll,
                    theme,
                    &mut code_used,
                );
                ghost_inserted = true;
            }
        }
        if cell.x < x_scroll {
            continue;
        }
        if cell.x + cell.width > x_scroll + code_width {
            break;
        }
        let mut style = syntax_style(theme, app.highlighter.kind(line_index, cell.original));
        if let Some((start, end)) = selection {
            let selected = if line_index < start.line || line_index > end.line {
                false
            } else if start.line == end.line {
                cell.original >= start.col && cell.original < end.col
            } else if line_index == start.line {
                cell.original >= start.col
            } else if line_index == end.line {
                cell.original < end.col
            } else {
                true
            };
            if selected {
                style = style.bg(theme.selection);
            }
        }
        if search_flags.get(cell.original).copied().unwrap_or(false) {
            style = style.bg(theme.search).fg(theme.file);
        }
        if cursor_active
            && cursor_on_row
            && cursor_original == Some(cell.original)
        {
            style = Style::default()
                .fg(theme.background)
                .bg(theme.file)
                .add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(cell.text.clone(), style));
        code_used += cell.width;
    }

    if let Some(ghost_text) = ghost.as_deref() {
        if !ghost_inserted {
            append_ghost(
                &mut spans,
                ghost_text,
                x,
                code_width,
                x_scroll,
                theme,
                &mut code_used,
            );
        }
    } else if cursor_active
        && cursor_on_row
        && cursor_col >= chars.len()
        && x >= x_scroll
        && x < x_scroll + code_width
        && code_used < code_width
    {
        spans.push(Span::styled(
            " ",
            Style::default()
                .fg(theme.background)
                .bg(theme.file)
                .add_modifier(Modifier::BOLD),
        ));
    }

    Line::from(spans).style(Style::default().bg(theme.panel))
}

fn spaces_to_cells(cells: &mut Vec<DisplayCell>, original: usize, spaces: usize, x: &mut usize) {
    for _ in 0..spaces {
        cells.push(DisplayCell {
            text: " ".to_string(),
            original,
            x: *x,
            width: 1,
        });
        *x += 1;
    }
}

fn append_ghost(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    x: usize,
    code_width: usize,
    x_scroll: usize,
    theme: Theme,
    used: &mut usize,
) {
    if x < x_scroll || x >= x_scroll + code_width {
        return;
    }
    let mut visible = String::new();
    let mut column = x;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if column + width > x_scroll + code_width || *used + width > code_width {
            break;
        }
        visible.push(ch);
        column += width;
        *used += width;
    }
    if !visible.is_empty() {
        spans.push(Span::styled(
            visible,
            Style::default().fg(theme.ghost).bg(theme.panel),
        ));
    }
}

fn display_prefix_width(text: &str, columns: usize, tab_width: usize) -> usize {
    let tab_width = tab_width.max(1);
    let mut x = 0usize;
    for ch in text.chars().take(columns) {
        if ch == '\t' {
            x += tab_width - (x % tab_width);
        } else {
            x += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
    }
    x
}

fn syntax_style(theme: Theme, kind: SyntaxKind) -> Style {
    let color = match kind {
        SyntaxKind::Default => theme.file,
        SyntaxKind::Keyword => theme.keyword,
        SyntaxKind::Type => theme.type_color,
        SyntaxKind::String => theme.string,
        SyntaxKind::Comment => theme.comment,
        SyntaxKind::Number => theme.number,
        SyntaxKind::Function => theme.function,
        SyntaxKind::Macro => theme.macro_color,
        SyntaxKind::Constant => theme.type_color,
        SyntaxKind::Operator => theme.operator,
        SyntaxKind::Property => theme.property,
        SyntaxKind::Heading => theme.heading,
        SyntaxKind::Link => theme.link,
        SyntaxKind::Code => theme.code,
        SyntaxKind::Tag => theme.tag,
    };
    Style::default().fg(color).bg(theme.panel)
}

fn draw_output(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let theme = app.theme;
    let focused_output = app.focus == Focus::Output;
    let (state_text, state_color) = if app.runner.is_running() {
        ("running".to_string(), theme.accent)
    } else if let Some(exit) = app.exit() {
        let text = match exit.code {
            Some(code) => format!("exit {code}"),
            None => "terminated".to_string(),
        };
        (
            text,
            if exit.success {
                theme.accent
            } else {
                theme.error
            },
        )
    } else {
        ("idle".to_string(), theme.muted)
    };
    let title = Line::from(vec![
        Span::styled(" OUTPUT ", Style::default().fg(theme.key)),
        Span::styled(format!("· {state_text} "), Style::default().fg(state_color)),
        Span::styled(
            "· ввод: клавиатура ".to_string(),
            Style::default().fg(theme.muted),
        ),
    ]);
    let block = panel_block(theme, title, false, focused_output);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    app.runner.set_winsize(inner.width, inner.height);
    let width = inner.width as usize;
    let height = inner.height as usize;
    let mut rows = Vec::new();
    for line in app.runner.output.completed() {
        rows.extend(wrap_out_line(line, width, theme));
    }
    for line in app.runner.output.partial_lines() {
        rows.extend(wrap_out_line(&line, width, theme));
    }
    if rows.is_empty() {
        rows.push(Line::from(Span::styled(
            " Ctrl+R — запустить выбранный файл",
            Style::default().fg(theme.muted).bg(theme.panel),
        )));
        rows.push(Line::from(Span::styled(
            " PageUp/PageDown — прокрутка вывода",
            Style::default().fg(theme.muted).bg(theme.panel),
        )));
    }
    let max_offset = rows.len().saturating_sub(height);
    app.runner.output.note_view(height, max_offset);
    let start = app.runner.output.offset.min(max_offset);
    let visible: Vec<Line<'static>> = rows.into_iter().skip(start).take(height).collect();
    frame.render_widget(
        Paragraph::new(visible).style(Style::default().bg(theme.panel)),
        inner,
    );
}

struct StyledRun {
    text: String,
    style: Style,
}

fn wrap_out_line(line: &OutLine, width: usize, theme: Theme) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let base_color = match line.source {
        OutSource::Stdout => theme.output,
        OutSource::System => theme.accent,
        OutSource::Error => theme.error,
    };
    let mut rows = Vec::new();
    let mut runs: Vec<StyledRun> = Vec::new();
    let mut column = 0usize;
    for span in &line.spans {
        let style = output_style(span.style, base_color, theme);
        for ch in span.text.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                flush_output_row(&mut runs, &mut rows);
                column = 0;
                continue;
            }
            if ch == '\t' {
                let spaces = 4 - (column % 4);
                for _ in 0..spaces {
                    emit_output_char(&mut runs, &mut rows, &mut column, width, ' ', style);
                }
                continue;
            }
            emit_output_char(&mut runs, &mut rows, &mut column, width, ch, style);
        }
    }
    flush_output_row(&mut runs, &mut rows);
    rows
}

fn emit_output_char(
    runs: &mut Vec<StyledRun>,
    rows: &mut Vec<Line<'static>>,
    column: &mut usize,
    width: usize,
    ch: char,
    style: Style,
) {
    let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
    if char_width == 0 {
        if let Some(last) = runs.last_mut() {
            last.text.push(ch);
        }
        return;
    }
    if *column + char_width > width && *column > 0 {
        flush_output_row(runs, rows);
        *column = 0;
    }
    if char_width > width {
        return;
    }
    if let Some(last) = runs.last_mut() {
        if last.style == style {
            last.text.push(ch);
            *column += char_width;
            return;
        }
    }
    runs.push(StyledRun {
        text: ch.to_string(),
        style,
    });
    *column += char_width;
}

fn flush_output_row(runs: &mut Vec<StyledRun>, rows: &mut Vec<Line<'static>>) {
    let spans = runs
        .drain(..)
        .map(|run| Span::styled(run.text, run.style))
        .collect::<Vec<_>>();
    rows.push(Line::from(spans));
}

fn output_style(style: OutStyle, base_color: Color, theme: Theme) -> Style {
    let mut result = Style::default()
        .fg(style.fg.unwrap_or(base_color))
        .bg(style.bg.unwrap_or(theme.panel))
        .add_modifier(style.modifier);
    if style.fg.is_none() && style.bg.is_some() {
        result = result.fg(theme.background);
    }
    result
}

fn draw_status(frame: &mut Frame<'_>, app: &App, area: Rect, two_rows: bool) {
    let theme = app.theme;
    frame.render_widget(
        Block::new().style(Style::default().bg(theme.status_bg)),
        area,
    );
    if area.width == 0 || area.height == 0 {
        return;
    }
    let status_rect = Rect::new(area.x, area.y, area.width, 1);
    let right_width = area.width.min(20);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(status_rect);
    let workdir = fit_tail(&app.tree.relative_display(&app.workdir), 28);
    let mut left = vec![
        Span::styled(
            " artef ",
            Style::default()
                .fg(theme.accent)
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", Style::default().fg(theme.muted).bg(theme.status_bg)),
        Span::styled(
            workdir,
            Style::default().fg(theme.status_fg).bg(theme.status_bg),
        ),
    ];
    if app.editor.modified {
        left.push(Span::styled(
            format!(" {}", Glyphs::new(use_unicode(area.width)).modified),
            Style::default().fg(theme.error).bg(theme.status_bg),
        ));
    }
    if app.runner.is_running() {
        left.push(Span::styled(
            if app.focus == Focus::Output {
                " │ [OUTPUT FOCUS] "
            } else {
                " │ [TUI FOCUS] "
            },
            Style::default()
                .fg(theme.accent)
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if !two_rows {
        if app.input_mode != InputMode::Normal {
            left.push(Span::styled(
                " │ ",
                Style::default().fg(theme.muted).bg(theme.status_bg),
            ));
            left.extend(prompt_spans(app));
        } else if let Some(notice) = app.active_notice() {
            left.push(Span::styled(
                " │ ",
                Style::default().fg(theme.muted).bg(theme.status_bg),
            ));
            left.push(Span::styled(
                notice.text.clone(),
                Style::default()
                    .fg(if notice.error {
                        theme.error
                    } else {
                        theme.status_fg
                    })
                    .bg(theme.status_bg),
            ));
        } else {
            left.push(Span::styled(
                " │ ",
                Style::default().fg(theme.muted).bg(theme.status_bg),
            ));
            left.extend(hint_spans(app, theme));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(left)).style(Style::default().bg(theme.status_bg)),
        columns[0],
    );

    let status = if app.runner.is_running() {
        Span::styled(
            " running ",
            Style::default()
                .fg(theme.accent)
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(exit) = app.exit() {
        let label = match exit.code {
            Some(code) => format!(" exit:{code} "),
            None => " exited ".to_string(),
        };
        Span::styled(
            label,
            Style::default()
                .fg(if exit.success {
                    theme.accent
                } else {
                    theme.error
                })
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " ready ",
            Style::default()
                .fg(theme.muted)
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![status]))
            .alignment(Alignment::Right)
            .style(Style::default().bg(theme.status_bg)),
        columns[1],
    );

    if two_rows && area.height >= 2 {
        let second = Rect::new(area.x, area.y + 1, area.width, 1);
        let spans = if app.input_mode != InputMode::Normal {
            prompt_spans(app)
        } else if let Some(notice) = app.active_notice() {
            vec![Span::styled(
                format!(" {}", notice.text),
                Style::default()
                    .fg(if notice.error {
                        theme.error
                    } else {
                        theme.status_fg
                    })
                    .bg(theme.status_bg),
            )]
        } else {
            hint_spans(app, theme)
        };
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.status_bg)),
            second,
        );
    }
}

fn prompt_spans(app: &App) -> Vec<Span<'static>> {
    let theme = app.theme;
    let (label, color) = match app.input_mode {
        InputMode::Find => ("FIND", theme.key),
        InputMode::Goto => ("GOTO", theme.key),
        InputMode::Normal => ("", theme.key),
    };
    let cursor = if use_unicode(80) { "▌" } else { "_" };
    vec![
        Span::styled(
            format!(" {label}: "),
            Style::default()
                .fg(color)
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            app.prompt.clone(),
            Style::default().fg(theme.status_fg).bg(theme.status_bg),
        ),
        Span::styled(
            cursor,
            Style::default().fg(theme.accent).bg(theme.status_bg),
        ),
        Span::styled(
            "  Esc отмена · Enter применить",
            Style::default().fg(theme.muted).bg(theme.status_bg),
        ),
    ]
}

fn hint_spans(app: &App, theme: Theme) -> Vec<Span<'static>> {
    let items: Vec<(Action, String)> = vec![
        (Action::Save, "Save".to_string()),
        (Action::Run, "Run".to_string()),
        (Action::NewFile, "New".to_string()),
        (Action::Workdir, "Workdir".to_string()),
        (Action::Find, "Find".to_string()),
        (Action::FocusOutput, "Focus".to_string()),
        (Action::Quit, "Quit".to_string()),
    ];
    let mut spans = Vec::new();
    for (index, (action, label)) in items.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" ", Style::default().bg(theme.status_bg)));
        }
        spans.push(Span::styled(
            "[",
            Style::default().fg(theme.muted).bg(theme.status_bg),
        ));
        spans.push(Span::styled(
            app.bindings.label(*action),
            Style::default()
                .fg(theme.key)
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            "]",
            Style::default().fg(theme.muted).bg(theme.status_bg),
        ));
        spans.push(Span::styled(
            label.clone(),
            Style::default()
                .fg(theme.status_fg)
                .bg(theme.status_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

fn fit_tail(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let mut result = String::new();
    let mut used = 1usize;
    for ch in text.chars().rev() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + char_width > width {
            break;
        }
        result.insert(0, ch);
        used += char_width;
    }
    result.insert(0, '…');
    result
}

fn draw_modal(frame: &mut Frame<'_>, app: &App) {
    if app.modal == Modal::None {
        return;
    }
    let theme = app.theme;
    let area = frame.area();
    let (width, height, title) = match &app.modal {
        Modal::NewFileName { .. } => (68, 11, "новый файл"),
        Modal::NewFolderName { .. } => (68, 10, "новая папка"),
        Modal::ExtensionSelect { .. } => (76, 13, "язык для файла"),
        Modal::RuntimeSelect { .. } => (70, 13, "выбор рантайма"),
        Modal::DeleteConfirm { .. } => (64, 8, "удалить?"),
        Modal::SaveConfirm { .. } => (64, 8, "сохранить?"),
        Modal::QuitConfirm => (64, 8, "выход из artef"),
        Modal::None => return,
    };
    let rect = modal_area(area, width, height);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Line::from(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let unicode = use_unicode(inner.width);
    let glyphs = Glyphs::new(unicode);
    let lines = match &app.modal {
        Modal::NewFileName {
            folder,
            input,
            error,
        } => {
            new_name_lines(app, "Файл", folder, input, error.as_deref(), width)
        }
        Modal::NewFolderName {
            parent,
            input,
            error,
        } => new_name_lines(app, "Папка", parent, input, error.as_deref(), width),
        Modal::ExtensionSelect {
            base,
            selected,
            error,
            ..
        } => extension_lines(app, base, *selected, error.as_deref(), width),
        Modal::RuntimeSelect {
            selected, error, ..
        } => runtime_lines(app, *selected, error.as_deref(), width),
        Modal::DeleteConfirm { path, is_dir } => {
            let raw = app.tree.relative_display(path);
            let name = fit_tail(&raw, width.saturating_sub(14));
            let kind = if *is_dir { "папку" } else { "файл" };
            confirm_lines(
                app,
                &format!(" Удалить {kind} «{name}» безвозвратно?"),
                " Enter/Y удалить · Esc/N отменить",
                width,
            )
        }
        Modal::SaveConfirm { then_quit, .. } => {
            let (question, hints) = if *then_quit {
                (
                    " Есть несохранённые изменения. Сохранить перед выходом?",
                    " Enter/Y сохранить и выйти · N/Esc не сохранять и выйти",
                )
            } else {
                (
                    " Есть несохранённые изменения. Сохранить перед возвратом к дереву?",
                    " Enter/Y сохранить · N/Esc не сохранять · вернуться к дереву",
                )
            };
            confirm_lines(app, question, hints, width)
        }
        Modal::QuitConfirm => confirm_lines(
            app,
            " Выйти из artef целиком?",
            " Enter/Y выйти · N/Esc остаться · Ctrl+C в выводе идёт в процесс",
            width,
        ),
        Modal::None => Vec::new(),
    };
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
    let _ = glyphs;
}

fn modal_area(area: Rect, requested_width: u16, requested_height: u16) -> Rect {
    let width = requested_width.min(area.width.saturating_sub(2)).max(1);
    let height = requested_height.min(area.height.saturating_sub(1)).max(1);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn new_name_lines(
    app: &App,
    kind: &str,
    folder: &std::path::Path,
    input: &str,
    error: Option<&str>,
    width: usize,
) -> Vec<Line<'static>> {
    let theme = app.theme;
    let cursor = if use_unicode(width as u16) {
        "▌"
    } else {
        "_"
    };
    let folder = fit_tail(&app.tree.relative_display(folder), width.saturating_sub(10));
    let input_text = if input.is_empty() {
        cursor.to_string()
    } else {
        format!("{input}{cursor}")
    };
    let mut lines = vec![
        Line::from(Span::styled(
            format!(" {kind}"),
            Style::default().fg(theme.file).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" Папка: ", Style::default().fg(theme.key)),
            Span::styled(folder, Style::default().fg(theme.muted)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Имя: ", Style::default().fg(theme.key)),
            Span::styled(input_text, Style::default().fg(theme.file)),
        ]),
    ];
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            format!(" {error}"),
            Style::default().fg(theme.error),
        )));
    } else {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        " Расширение опционально: есть — язык авто, нет — выбор языка",
        Style::default().fg(theme.muted),
    )));
    lines.push(Line::from(Span::styled(
        " Enter далее · Esc отмена",
        Style::default().fg(theme.muted),
    )));
    lines
}

fn extension_lines(
    app: &App,
    base: &str,
    selected: usize,
    error: Option<&str>,
    width: usize,
) -> Vec<Line<'static>> {
    let theme = app.theme;
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" Файл: ", Style::default().fg(theme.key)),
            Span::styled(base.to_string(), Style::default().fg(theme.file)),
        ]),
        Line::from(Span::styled(
            " Выберите язык (или создайте без расширения)",
            Style::default().fg(theme.file).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    let total = EXTENSIONS.len() + 1;
    let columns = if width >= 60 {
        4
    } else if width >= 28 {
        2
    } else {
        1
    };
    let cell_width = (width / columns).max(1);
    for chunk_start in (0..total).step_by(columns) {
        let mut spans = Vec::new();
        for offset in 0..columns {
            let index = chunk_start + offset;
            if index >= total {
                spans.push(Span::styled(
                    " ".repeat(cell_width),
                    Style::default().bg(theme.panel),
                ));
                continue;
            }
            let label = if index == 0 {
                "(без расшир.)".to_string()
            } else {
                format!(".{}", EXTENSIONS[index - 1])
            };
            let text = format!(" {:<width$}", label, width = cell_width.saturating_sub(2));
            let text = format!("{:<cw$}", text, cw = cell_width);
            let style = if index == selected {
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.active_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.file).bg(theme.panel)
            };
            spans.push(Span::styled(text, style));
        }
        lines.push(Line::from(spans));
    }
    let selected_label = if selected == 0 {
        "(без расширения)".to_string()
    } else {
        format!(".{}", EXTENSIONS[selected - 1])
    };
    lines.push(Line::from(Span::styled(
        format!(" Итоговое имя: {base}{selected_label}"),
        Style::default().fg(theme.key),
    )));
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            format!(" {error}"),
            Style::default().fg(theme.error),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!(
            " {} / {} выбор · Enter создать · Esc назад",
            if use_unicode(width as u16) {
                "↑↓"
            } else {
                "Up/Down"
            },
            if use_unicode(width as u16) {
                "←→"
            } else {
                "Left/Right"
            }
        ),
        Style::default().fg(theme.muted),
    )));
    lines
}

fn runtime_lines(
    app: &App,
    selected: usize,
    error: Option<&str>,
    width: usize,
) -> Vec<Line<'static>> {
    let theme = app.theme;
    let mut lines = vec![Line::from(Span::styled(
        " У файла нет расширения — выберите рантайм",
        Style::default().fg(theme.file).add_modifier(Modifier::BOLD),
    ))];
    let us = use_unicode(width as u16);
    for (index, (extension, hint)) in RUNTIME_ENTRIES.iter().enumerate() {
        let marker = if index == selected {
            if us {
                "▸ "
            } else {
                "> "
            }
        } else {
            "  "
        };
        let hint_text = match hint {
            Some(program) => format!(" pkg: {program}"),
            None => String::new(),
        };
        let text = format!(" {marker}.{extension:<8}{hint_text}");
        let style = if index == selected {
            Style::default()
                .fg(theme.accent)
                .bg(theme.active_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.file).bg(theme.panel)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            format!(" {error}"),
            Style::default().fg(theme.error),
        )));
    } else {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        format!(
            " {} / {} выбор · Enter запустить · Esc отмена",
            if us { "↑↓" } else { "Up/Down" },
            if us { "↓↑" } else { "Down/Up" }
        ),
        Style::default().fg(theme.muted),
    )));
    lines
}

const RUNTIME_ENTRIES: [(&str, Option<&str>); 8] = [
    ("py", Some("python3")),
    ("js", Some("node")),
    ("ts", Some("tsx")),
    ("sh", Some("bash")),
    ("go", Some("go")),
    ("c", Some("gcc")),
    ("cpp", Some("g++")),
    ("rs", Some("cargo")),
];

fn confirm_lines(app: &App, question: &str, hints: &str, width: usize) -> Vec<Line<'static>> {
    let theme = app.theme;
    vec![
        Line::from(""),
        Line::from(Span::styled(
            chunk_text_line(question, width),
            Style::default()
                .fg(theme.file)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            chunk_text_line(hints, width),
            Style::default().fg(theme.key),
        )),
    ]
}

fn chunk_text_line(text: &str, width: usize) -> String {
    if width == 0 {
        return text.to_string();
    }
    let mut result = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(1);
        if used + char_width > width {
            result.push('…');
            return result;
        }
        result.push(ch);
        used += char_width;
    }
    result
}
