use std::fs;
use std::path::{Path, PathBuf};

use crate::dictionary::Dictionary;
use crate::language::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    anchor: Option<Position>,
    modified: bool,
}

#[derive(Debug, Clone)]
pub struct GhostSuggestion {
    pub completion: String,
}

pub struct Editor {
    pub path: Option<PathBuf>,
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub anchor: Option<Position>,
    pub scroll: usize,
    pub x_scroll: usize,
    pub wrap: bool,
    pub modified: bool,
    pub revision: u64,
    pub language: Language,
    pub search: Option<String>,
    pub tab_width: usize,
    dictionary: Dictionary,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    dismissed_ghost: Option<(usize, usize, String)>,
    visual_cache: Option<(u64, usize, Vec<usize>)>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            path: None,
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
            anchor: None,
            scroll: 0,
            x_scroll: 0,
            wrap: false,
            modified: false,
            revision: 1,
            language: Language::Plain,
            search: None,
            tab_width: 4,
            dictionary: Dictionary::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            dismissed_ghost: None,
            visual_cache: None,
        }
    }
}

impl Editor {
    pub fn open(path: &Path, tab_width: usize) -> Result<Self, String> {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("не удалось открыть {}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("{} не является файлом", path.display()));
        }
        let content = fs::read_to_string(path)
            .map_err(|error| format!("не удалось прочитать {}: {error}", path.display()))?;
        let content = content.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<String> = content.split('\n').map(|line| line.to_string()).collect();
        let language = Language::from_path(path);
        let dictionary = Dictionary::load(language);
        Ok(Self {
            path: Some(path.to_path_buf()),
            lines: if lines.is_empty() {
                vec![String::new()]
            } else {
                lines
            },
            cursor_line: 0,
            cursor_col: 0,
            anchor: None,
            scroll: 0,
            x_scroll: 0,
            wrap: false,
            modified: false,
            revision: 1,
            language,
            search: None,
            tab_width: tab_width.max(1),
            dictionary,
            undo: Vec::new(),
            redo: Vec::new(),
            dismissed_ghost: None,
            visual_cache: None,
        })
    }

    pub fn has_file(&self) -> bool {
        self.path.is_some()
    }

    pub fn file_name(&self) -> Option<String> {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string())
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn current_line(&self) -> &str {
        self.lines
            .get(self.cursor_line)
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn cursor(&self) -> Position {
        Position {
            line: self.cursor_line,
            col: self.cursor_col,
        }
    }

    pub fn selection(&self) -> Option<(Position, Position)> {
        let anchor = self.anchor?;
        let start = Position {
            line: self.cursor_line,
            col: self.cursor_col,
        };
        if anchor == start {
            return None;
        }
        if (anchor.line, anchor.col) < (start.line, start.col) {
            Some((anchor, start))
        } else {
            Some((start, anchor))
        }
    }

    pub fn save(&mut self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err("файл не открыт".to_string());
        };
        let content = self.lines.join("\n");
        fs::write(path, content)
            .map_err(|error| format!("не удалось сохранить {}: {error}", path.display()))?;
        self.modified = false;
        Ok(())
    }

    pub fn set_search(&mut self, query: Option<String>) {
        self.search = query.filter(|value| !value.is_empty());
    }

    pub fn find_next(&mut self, query: &str, advance: bool) -> Option<(usize, usize)> {
        if query.is_empty() {
            return None;
        }
        let needle: Vec<char> = query.chars().collect();
        let mut matches = Vec::new();
        for (line_index, line) in self.lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            if needle.is_empty() || chars.len() < needle.len() {
                continue;
            }
            for start in 0..=chars.len() - needle.len() {
                if chars[start..start + needle.len()] == needle[..] {
                    matches.push((line_index, start));
                }
            }
        }
        if matches.is_empty() {
            return None;
        }
        let current = (self.cursor_line, self.cursor_col);
        let next = matches
            .iter()
            .copied()
            .find(|position| {
                if advance {
                    *position > current
                } else {
                    *position >= current
                }
            })
            .or_else(|| matches.first().copied());
        if let Some((line, col)) = next {
            self.cursor_line = line;
            self.cursor_col = col;
            self.anchor = None;
            self.dismissed_ghost = None;
            Some((line, col))
        } else {
            None
        }
    }

    pub fn goto_line(&mut self, one_based: usize) {
        let line = one_based
            .saturating_sub(1)
            .min(self.lines.len().saturating_sub(1));
        self.cursor_line = line;
        self.cursor_col = 0;
        self.anchor = None;
        self.dismissed_ghost = None;
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        if start.line == end.line {
            let chars: Vec<char> = self.lines[start.line].chars().collect();
            let begin = start.col.min(chars.len());
            let finish = end.col.min(chars.len());
            return Some(chars[begin..finish].iter().collect());
        }
        let mut result = String::new();
        let first: Vec<char> = self.lines[start.line].chars().collect();
        result.extend(first.iter().skip(start.col));
        for line in &self.lines[start.line + 1..end.line] {
            result.push('\n');
            result.push_str(line);
        }
        result.push('\n');
        let last: Vec<char> = self.lines[end.line].chars().collect();
        result.extend(last.iter().take(end.col.min(last.len())));
        Some(result)
    }

    pub fn copy_to_clipboard(&self, clipboard: &mut String) -> bool {
        if let Some(selection) = self.selected_text() {
            *clipboard = selection;
            true
        } else {
            if self.current_line().is_empty() {
                return false;
            }
            *clipboard = self.current_line().to_string();
            true
        }
    }

    pub fn paste(&mut self, clipboard: &str) -> bool {
        let normalized = sanitize_text(clipboard);
        if normalized.is_empty() {
            return false;
        }
        self.record_undo();
        self.delete_selection_raw();
        self.insert_text_raw(&normalized);
        self.after_edit();
        true
    }

    pub fn insert_char(&mut self, ch: char) {
        self.record_undo();
        self.delete_selection_raw();
        let line = self.cursor_line;
        let byte = char_to_byte(&self.lines[line], self.cursor_col);
        self.lines[line].insert(byte, ch);
        self.cursor_col += 1;
        self.after_edit();
    }

    pub fn insert_tab(&mut self) {
        self.record_undo();
        self.delete_selection_raw();
        let spaces = self.tab_width - (self.cursor_col % self.tab_width);
        let text = " ".repeat(spaces);
        self.insert_text_raw(&text);
        self.after_edit();
    }

    pub fn insert_newline(&mut self) {
        self.record_undo();
        self.delete_selection_raw();
        let line = self.cursor_line;
        let chars: Vec<char> = self.lines[line].chars().collect();
        let indent: String = chars
            .iter()
            .take(chars.len().min(self.cursor_col))
            .take_while(|ch| ch.is_whitespace())
            .collect();
        let text = format!("\n{indent}");
        self.insert_text_raw(&text);
        self.after_edit();
    }

    pub fn backspace(&mut self) {
        if self.selection().is_some() {
            self.record_undo();
            self.delete_selection_raw();
            self.after_edit();
            return;
        }
        if self.cursor_col > 0 {
            self.record_undo();
            let line = self.cursor_line;
            let start = char_to_byte(&self.lines[line], self.cursor_col - 1);
            let end = char_to_byte(&self.lines[line], self.cursor_col);
            self.lines[line].replace_range(start..end, "");
            self.cursor_col -= 1;
            self.after_edit();
            return;
        }
        if self.cursor_line > 0 {
            self.record_undo();
            let current = self.lines.remove(self.cursor_line);
            let previous_line = self.cursor_line - 1;
            let previous_len = self.lines[previous_line].chars().count();
            self.lines[previous_line].push_str(&current);
            self.cursor_line = previous_line;
            self.cursor_col = previous_len;
            self.anchor = None;
            self.after_edit();
        }
    }

    pub fn delete_forward(&mut self) {
        if self.selection().is_some() {
            self.record_undo();
            self.delete_selection_raw();
            self.after_edit();
            return;
        }
        let line = self.cursor_line;
        let length = self.lines[line].chars().count();
        if self.cursor_col < length {
            self.record_undo();
            let start = char_to_byte(&self.lines[line], self.cursor_col);
            let end = char_to_byte(&self.lines[line], self.cursor_col + 1);
            self.lines[line].replace_range(start..end, "");
            self.after_edit();
            return;
        }
        if self.cursor_line + 1 < self.lines.len() {
            self.record_undo();
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[line].push_str(&next);
            self.after_edit();
        }
    }

    pub fn move_left(&mut self, extend: bool) {
        self.begin_move(extend);
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].chars().count();
        }
        self.dismissed_ghost = None;
    }

    pub fn move_right(&mut self, extend: bool) {
        self.begin_move(extend);
        let length = self.lines[self.cursor_line].chars().count();
        if self.cursor_col < length {
            self.cursor_col += 1;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
        self.dismissed_ghost = None;
    }

    pub fn move_up(&mut self, extend: bool) {
        self.begin_move(extend);
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            let length = self.lines[self.cursor_line].chars().count();
            self.cursor_col = self.cursor_col.min(length);
        }
        self.dismissed_ghost = None;
    }

    pub fn move_down(&mut self, extend: bool) {
        self.begin_move(extend);
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            let length = self.lines[self.cursor_line].chars().count();
            self.cursor_col = self.cursor_col.min(length);
        }
        self.dismissed_ghost = None;
    }

    pub fn move_home(&mut self, extend: bool) {
        self.begin_move(extend);
        self.cursor_col = 0;
        self.dismissed_ghost = None;
    }

    pub fn move_end(&mut self, extend: bool) {
        self.begin_move(extend);
        self.cursor_col = self.lines[self.cursor_line].chars().count();
        self.dismissed_ghost = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.redo.push(current);
        self.restore(snapshot);
        self.dismissed_ghost = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.undo.push(current);
        self.restore(snapshot);
        self.dismissed_ghost = None;
        true
    }

    pub fn ghost(&self) -> Option<GhostSuggestion> {
        if self.selection().is_some() {
            return None;
        }
        let (line, start, prefix) = self.word_prefix()?;
        if let Some((dismissed_line, dismissed_col, dismissed_prefix)) = &self.dismissed_ghost {
            if *dismissed_line == line
                && *dismissed_col == self.cursor_col
                && *dismissed_prefix == prefix
            {
                return None;
            }
        }
        let completion = self.dictionary.suggestion(&prefix)?;
        if start > self.cursor_col {
            return None;
        }
        Some(GhostSuggestion {
            completion: completion.to_string(),
        })
    }

    pub fn dismiss_ghost(&mut self) {
        if let Some((line, _, prefix)) = self.word_prefix() {
            self.dismissed_ghost = Some((line, self.cursor_col, prefix));
        }
    }

    pub fn accept_ghost(&mut self) -> bool {
        let Some(suggestion) = self.ghost() else {
            return false;
        };
        if suggestion.completion.is_empty() {
            return false;
        }
        self.record_undo();
        self.insert_text_raw(&suggestion.completion);
        self.after_edit();
        self.dismissed_ghost = None;
        true
    }

    pub fn matches_in_line(&self, line_index: usize, needle: &str) -> Vec<usize> {
        if needle.is_empty() {
            return Vec::new();
        }
        let chars: Vec<char> = self
            .lines
            .get(line_index)
            .map(|line| line.chars().collect())
            .unwrap_or_default();
        let needle: Vec<char> = needle.chars().collect();
        if chars.len() < needle.len() || needle.is_empty() {
            return Vec::new();
        }
        let mut result = Vec::new();
        for start in 0..=chars.len() - needle.len() {
            if chars[start..start + needle.len()] == needle[..] {
                result.push(start);
            }
        }
        result
    }

    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        if !self.wrap {
            self.x_scroll = 0;
        }
    }

    pub fn cursor_row_number(&self) -> usize {
        self.cursor_line + 1
    }

    pub fn column_number(&self) -> usize {
        self.cursor_col + 1
    }

    /// Visual-row start per logical line, cached per (revision, width).
    pub fn visual_starts(&mut self, code_width: usize) -> &[usize] {
        let width = code_width.max(1);
        if let Some((cached_revision, cached_width)) = self
            .visual_cache
            .as_ref()
            .map(|(revision, width, _)| (*revision, *width))
        {
            if cached_revision == self.revision && cached_width == width {
                return match self.visual_cache.as_ref() {
                    Some((_, _, cached)) => cached,
                    None => unreachable!(),
                };
            }
        }
        let mut starts = Vec::with_capacity(self.lines.len() + 1);
        let mut total = 0usize;
        for line in &self.lines {
            starts.push(total);
            total += wrap_ranges(line, width, self.tab_width).len();
        }
        starts.push(total);
        self.visual_cache = Some((self.revision, width, starts));
        match self.visual_cache.as_ref() {
            Some((_, _, cached)) => cached,
            None => unreachable!(),
        }
    }

    pub fn cursor_row_offset(&self) -> usize {
        let Some(text) = self.lines.get(self.cursor_line) else {
            return 0;
        };
        let ranges = wrap_ranges(text, usize::MAX, self.tab_width);
        let mut offset = 0usize;
        for (start, end) in ranges {
            if self.cursor_col >= start && self.cursor_col <= end {
                return offset;
            }
            offset += 1;
        }
        offset
    }

    fn word_prefix(&self) -> Option<(usize, usize, String)> {
        let line = self.lines.get(self.cursor_line)?;
        let chars: Vec<char> = line.chars().collect();
        if self.cursor_col > chars.len() {
            return None;
        }
        if let Some(next) = chars.get(self.cursor_col) {
            if is_word_char(*next) {
                return None;
            }
        }
        let mut start = self.cursor_col;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        if start == self.cursor_col {
            return None;
        }
        let prefix: String = chars[start..self.cursor_col].iter().collect();
        Some((self.cursor_line, start, prefix))
    }

    fn begin_move(&mut self, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor());
            }
        } else {
            self.anchor = None;
        }
    }

    fn after_edit(&mut self) {
        self.modified = true;
        self.revision = self.revision.wrapping_add(1);
        self.anchor = None;
        self.dismissed_ghost = None;
    }

    fn record_undo(&mut self) {
        let snapshot = self.snapshot();
        self.undo.push(snapshot);
        if self.undo.len() > 256 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
            anchor: self.anchor,
            modified: self.modified,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.lines = snapshot.lines;
        self.cursor_line = snapshot.cursor_line.min(self.lines.len().saturating_sub(1));
        let length = self.lines[self.cursor_line].chars().count();
        self.cursor_col = snapshot.cursor_col.min(length);
        self.anchor = snapshot.anchor;
        self.modified = snapshot.modified;
        self.revision = self.revision.wrapping_add(1);
    }

    fn delete_selection_raw(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        if start.line == end.line {
            let line = start.line;
            let begin = char_to_byte(&self.lines[line], start.col);
            let finish = char_to_byte(&self.lines[line], end.col);
            self.lines[line].replace_range(begin..finish, "");
        } else {
            let first_chars: Vec<char> = self.lines[start.line].chars().collect();
            let last_chars: Vec<char> = self.lines[end.line].chars().collect();
            let mut joined: String = first_chars.iter().take(start.col).collect();
            joined.push_str(&last_chars.iter().skip(end.col).collect::<String>());
            self.lines[start.line] = joined;
            self.lines.drain(start.line + 1..=end.line);
        }
        self.cursor_line = start.line;
        self.cursor_col = start.col;
        self.anchor = None;
        true
    }

    fn insert_text_raw(&mut self, text: &str) {
        let parts: Vec<&str> = text.split('\n').collect();
        let line = self.cursor_line;
        let chars: Vec<char> = self.lines[line].chars().collect();
        let before: String = chars.iter().take(self.cursor_col).collect();
        let after: String = chars.iter().skip(self.cursor_col).collect();
        if parts.len() == 1 {
            let updated = format!("{before}{}{after}", parts[0]);
            self.lines[line] = updated;
            self.cursor_col += parts[0].chars().count();
            return;
        }
        self.lines[line] = format!("{before}{}", parts[0]);
        for (offset, part) in parts[1..parts.len() - 1].iter().enumerate() {
            self.lines.insert(line + 1 + offset, (*part).to_string());
        }
        let last = parts[parts.len() - 1];
        self.lines
            .insert(line + parts.len() - 1, format!("{last}{after}"));
        self.cursor_line = line + parts.len() - 1;
        self.cursor_col = last.chars().count();
    }
}

/// Strip C0 control chars (keep `\n`, `\t`), normalise `\r` to `\n`.
pub fn sanitize_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            result.push('\n');
            continue;
        }
        if (ch as u32) < 0x20 && ch != '\n' && ch != '\t' {
            continue;
        }
        if ch == '\u{7f}' {
            continue;
        }
        result.push(ch);
    }
    result
}

/// Word-aware greedy wrap ranges of a line in char-index space, tab atomic.
pub fn wrap_ranges(text: &str, width: usize, tab_width: usize) -> Vec<(usize, usize)> {
    let chars: Vec<(char, usize)> = text
        .chars()
        .map(|ch| {
            let w = if ch == '\t' {
                0
            } else {
                unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0).max(1)
            };
            (ch, w)
        })
        .collect();
    if width == 0 {
        return vec![(0, chars.len())];
    }
    let tab_width = tab_width.max(1);
    let mut ranges = Vec::new();
    let mut row_start = 0usize;
    let mut x = 0usize;
    for index in 0..chars.len() {
        let (ch, mut w) = chars[index];
        if ch == '\t' {
            w = tab_width - (x % tab_width);
        }
        if x + w > width && index > row_start {
            let mut break_at = index;
            let mut scan = index;
            while scan > row_start && is_breakable(chars[scan - 1].0) {
                scan -= 1;
            }
            if scan > row_start {
                break_at = scan;
            }
            ranges.push((row_start, break_at));
            row_start = break_at;
            x = 0;
            if ch == '\t' {
                w = tab_width;
            }
        }
        x += w;
    }
    ranges.push((row_start, chars.len()));
    ranges
}

fn is_breakable(ch: char) -> bool {
    ch == ' ' || ch == '\t'
}

fn char_to_byte(text: &str, column: usize) -> usize {
    if column == 0 {
        return 0;
    }
    text.char_indices()
        .nth(column)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_normalises_newlines_and_strips_controls() {
        let text = "a\r\nb\rcd\u{1}\u{1b}[f\tg";
        assert_eq!(sanitize_text(text), "a\nb\ncd[f\tg");
    }

    #[test]
    fn paste_replaces_buffer_atomically() {
        let mut editor = Editor::default();
        editor.paste("one\ntwo\nthree\n");
        assert_eq!(editor.line_count(), 4);
        assert_eq!(editor.lines[0], "one");
        assert_eq!(editor.lines[2], "three");
        assert_eq!(editor.cursor_line, 3);
        assert_eq!(editor.cursor_col, 0);
        assert!(editor.modified);
        assert_eq!(editor.revision, 2);
        assert_eq!(editor.undo.len(), 1);
        assert!(editor.undo());
        assert_eq!(editor.line_count(), 1);
    }

    #[test]
    fn wrap_ranges_break_long_lines() {
        let ranges = wrap_ranges("abcdefgh", 3, 4);
        assert_eq!(
            ranges,
            vec![(0, 3), (3, 6), (6, 8)]
        );
    }

    #[test]
    fn wrap_ranges_empty_line_is_single_row() {
        assert_eq!(wrap_ranges("", 10, 4), vec![(0, 0)]);
    }

    #[test]
    fn wrap_ranges_expands_tabs_atomically() {
        let ranges = wrap_ranges("\t\tx", 5, 4);
        assert_eq!(ranges, vec![(0, 1), (1, 3)]);
    }

    #[test]
    fn visual_starts_counts_rows_per_line() {
        let mut editor = Editor::default();
        editor.lines = vec!["x".repeat(10), String::new(), "y".repeat(3)];
        let starts = editor.visual_starts(4).to_vec();
        assert_eq!(starts, vec![0, 3, 4, 5]);
        editor.revision = 77;
        editor.visual_starts(4);
        editor.cursor_line = 0;
        assert_eq!(editor.cursor_row_offset(), 0);
    }

    #[test]
    fn cursor_double_undo_past_paste() {
        let mut editor = Editor::default();
        editor.paste("alpha\nbeta\n");
        assert!(editor.undo());
        assert_eq!(editor.lines.len(), 1);
        assert!(editor.redo());
        assert_eq!(editor.lines.len(), 3);
    }
}
