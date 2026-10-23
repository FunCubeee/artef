use crate::language::{self, Language};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxKind {
    Default,
    Keyword,
    Type,
    String,
    Comment,
    Number,
    Function,
    Macro,
    Constant,
    Operator,
    Property,
    Heading,
    Link,
    Code,
    Tag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LexState {
    Normal,
    BlockComment,
    TripleDouble,
    TripleSingle,
    Backtick,
}

pub struct Highlighter {
    language: Language,
    revision: u64,
    kinds: Vec<Vec<SyntaxKind>>,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self {
            language: Language::Plain,
            revision: u64::MAX,
            kinds: Vec::new(),
        }
    }
}

impl Highlighter {
    pub fn update(&mut self, language: Language, revision: u64, lines: &[String]) {
        if self.language == language && self.revision == revision {
            return;
        }
        self.language = language;
        self.revision = revision;
        self.kinds = match language {
            Language::Markdown => highlight_markdown(lines),
            Language::Json => lines.iter().map(|line| highlight_json(line)).collect(),
            Language::Yaml => lines.iter().map(|line| highlight_yaml(line)).collect(),
            Language::Toml => lines.iter().map(|line| highlight_toml(line)).collect(),
            Language::Plain => lines
                .iter()
                .map(|line| vec![SyntaxKind::Default; line.chars().count()])
                .collect(),
            _ => highlight_code(lines, language),
        };
    }

    pub fn kind(&self, line: usize, column: usize) -> SyntaxKind {
        self.kinds
            .get(line)
            .and_then(|kinds| kinds.get(column))
            .copied()
            .unwrap_or(SyntaxKind::Default)
    }
}

fn highlight_code(lines: &[String], language: Language) -> Vec<Vec<SyntaxKind>> {
    let mut state = LexState::Normal;
    let mut result = Vec::with_capacity(lines.len());
    for line in lines {
        let (kinds, next_state) = highlight_code_line(line, language, state);
        state = next_state;
        result.push(kinds);
    }
    result
}

fn highlight_code_line(
    line: &str,
    language: Language,
    initial: LexState,
) -> (Vec<SyntaxKind>, LexState) {
    let chars: Vec<char> = line.chars().collect();
    let mut kinds = vec![SyntaxKind::Default; chars.len()];
    let mut state = initial;
    let mut index = 0usize;

    if state != LexState::Normal {
        let (consumed, ended) = consume_multiline(&chars, state, language);
        mark(&mut kinds, 0, consumed, multiline_kind(state));
        if !ended {
            return (kinds, state);
        }
        state = LexState::Normal;
        index = consumed;
    }

    let first_non_space = chars.iter().position(|c| !c.is_whitespace());
    if matches!(language, Language::C | Language::Cpp) {
        if let Some(start) = first_non_space {
            if chars[start] == '#' {
                mark(&mut kinds, start, chars.len(), SyntaxKind::Macro);
                return (kinds, state);
            }
        }
    }

    let mut expect_name: Option<SyntaxKind> = None;

    while index < chars.len() {
        let c = chars[index];

        if c.is_whitespace() {
            index += 1;
            continue;
        }

        if state != LexState::Normal {
            let (consumed, ended) = consume_multiline(&chars[index..], state, language);
            mark(&mut kinds, index, index + consumed, multiline_kind(state));
            index += consumed;
            if !ended {
                return (kinds, state);
            }
            state = LexState::Normal;
            continue;
        }

        if let Some(comment) = language::line_comment(language) {
            if starts_with(&chars, index, comment) {
                let comment_allowed = if language::hash_comment_needs_space(language) {
                    index == 0 || chars[index - 1].is_whitespace()
                } else {
                    true
                };
                if comment_allowed {
                    mark(&mut kinds, index, chars.len(), SyntaxKind::Comment);
                    break;
                }
            }
        }

        if let Some((open, close)) = language::block_comment(language) {
            if starts_with(&chars, index, open) {
                if let Some(end) = find_seq(&chars, index + open.chars().count(), close) {
                    let finish = end + close.chars().count();
                    mark(&mut kinds, index, finish, SyntaxKind::Comment);
                    index = finish;
                } else {
                    mark(&mut kinds, index, chars.len(), SyntaxKind::Comment);
                    return (kinds, LexState::BlockComment);
                }
                continue;
            }
        }

        if language == Language::Python && c == '"' && starts_with(&chars, index, "\"\"\"") {
            if let Some(end) = find_seq(&chars, index + 3, "\"\"\"") {
                let finish = end + 3;
                mark(&mut kinds, index, finish, SyntaxKind::String);
                index = finish;
            } else {
                mark(&mut kinds, index, chars.len(), SyntaxKind::String);
                return (kinds, LexState::TripleDouble);
            }
            continue;
        }

        if language == Language::Python && c == '\'' && starts_with(&chars, index, "'''") {
            if let Some(end) = find_seq(&chars, index + 3, "'''") {
                let finish = end + 3;
                mark(&mut kinds, index, finish, SyntaxKind::String);
                index = finish;
            } else {
                mark(&mut kinds, index, chars.len(), SyntaxKind::String);
                return (kinds, LexState::TripleSingle);
            }
            continue;
        }

        if c == '`'
            && matches!(
                language,
                Language::JavaScript | Language::TypeScript | Language::Go | Language::Shell
            )
        {
            let end = scan_quoted(&chars, index, '`');
            match end {
                Some(finish) => {
                    mark(&mut kinds, index, finish, SyntaxKind::String);
                    index = finish;
                }
                None => {
                    mark(&mut kinds, index, chars.len(), SyntaxKind::String);
                    if matches!(language, Language::JavaScript | Language::TypeScript) {
                        return (kinds, LexState::Backtick);
                    }
                    return (kinds, LexState::Normal);
                }
            }
            continue;
        }

        if c == '"' || c == '\'' {
            if language == Language::Rust && c == '\'' && rust_lifetime(&chars, index) {
                mark(&mut kinds, index, index + 1, SyntaxKind::Operator);
                index += 1;
                continue;
            }
            if let Some(finish) = scan_quoted(&chars, index, c) {
                mark(&mut kinds, index, finish, SyntaxKind::String);
                index = finish;
            } else {
                mark(&mut kinds, index, chars.len(), SyntaxKind::String);
                break;
            }
            continue;
        }

        if c == '@' && language == Language::Python {
            let end = word_end(&chars, index + 1);
            mark(&mut kinds, index, end, SyntaxKind::Function);
            index = end;
            continue;
        }

        if c == '$' && language == Language::Shell {
            let end = word_end(&chars, index + 1);
            mark(&mut kinds, index, end, SyntaxKind::Constant);
            index = end.max(index + 1);
            continue;
        }

        if c.is_ascii_digit() {
            let mut end = index + 1;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric()
                    || chars[end] == '_'
                    || chars[end] == '.'
                    || ((chars[end] == '+' || chars[end] == '-')
                        && end > index
                        && matches!(chars[end - 1], 'e' | 'E')))
            {
                end += 1;
            }
            mark(&mut kinds, index, end, SyntaxKind::Number);
            index = end;
            continue;
        }

        if is_word_start(c, language) {
            let end = word_end(chars.as_slice(), index + 1);
            let word: String = chars[index..end].iter().collect();
            let next = chars.get(end).copied();
            let mut kind = SyntaxKind::Default;
            if let Some(pending) = expect_name.take() {
                kind = pending;
            } else if language::keywords(language).contains(&word.as_str()) {
                kind = SyntaxKind::Keyword;
                match word.as_str() {
                    "class" | "struct" | "enum" | "trait" | "interface" | "type" | "union" => {
                        expect_name = Some(SyntaxKind::Type)
                    }
                    "def" | "fn" | "function" | "func" | "namespace" | "mod" => {
                        expect_name = Some(SyntaxKind::Function)
                    }
                    _ => {}
                }
            } else if language::constants(language).contains(&word.as_str()) {
                kind = SyntaxKind::Constant;
            } else if language::types(language).contains(&word.as_str()) {
                kind = SyntaxKind::Type;
            } else if language::builtins(language).contains(&word.as_str()) {
                kind = SyntaxKind::Function;
            } else if next == Some('!') && language == Language::Rust {
                kind = SyntaxKind::Macro;
            } else if next == Some('(') {
                kind = SyntaxKind::Function;
            } else if matches!(language, Language::C | Language::Cpp | Language::Rust)
                && word.chars().next().is_some_and(|ch| ch.is_uppercase())
            {
                kind = SyntaxKind::Type;
            }
            mark(&mut kinds, index, end, kind);
            index = end;
            continue;
        }

        if "+-*/%=<>!&|^~?:".contains(c) {
            mark(&mut kinds, index, index + 1, SyntaxKind::Operator);
        }
        index += 1;
    }

    (kinds, state)
}

fn multiline_kind(state: LexState) -> SyntaxKind {
    match state {
        LexState::BlockComment => SyntaxKind::Comment,
        _ => SyntaxKind::String,
    }
}

fn consume_multiline(chars: &[char], state: LexState, language: Language) -> (usize, bool) {
    match state {
        LexState::Normal => (0, true),
        LexState::BlockComment => {
            let close = language::block_comment(language)
                .map(|(_, close)| close)
                .unwrap_or("*/");
            if let Some(end) = find_seq(chars, 0, close) {
                (end + close.chars().count(), true)
            } else {
                (chars.len(), false)
            }
        }
        LexState::TripleDouble => {
            if let Some(end) = find_seq(chars, 0, "\"\"\"") {
                (end + 3, true)
            } else {
                (chars.len(), false)
            }
        }
        LexState::TripleSingle => {
            if let Some(end) = find_seq(chars, 0, "'''") {
                (end + 3, true)
            } else {
                (chars.len(), false)
            }
        }
        LexState::Backtick => {
            let mut escaped = false;
            for (position, ch) in chars.iter().enumerate() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if *ch == '\\' {
                    escaped = true;
                } else if *ch == '`' {
                    return (position + 1, true);
                }
            }
            (chars.len(), false)
        }
    }
}

fn rust_lifetime(chars: &[char], index: usize) -> bool {
    if index + 1 >= chars.len() {
        return false;
    }
    let next = chars[index + 1];
    if !(next.is_alphabetic() || next == '_') {
        return false;
    }
    if index + 2 < chars.len() && chars[index + 2] == '\'' {
        return false;
    }
    let end = word_end(chars, index + 1);
    chars.get(end).copied() != Some('\'')
}

fn scan_quoted(chars: &[char], start: usize, quote: char) -> Option<usize> {
    let mut escaped = false;
    let mut index = start + 1;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn is_word_start(ch: char, language: Language) -> bool {
    ch.is_alphabetic()
        || ch == '_'
        || (ch == '$' && matches!(language, Language::JavaScript | Language::TypeScript))
}

fn word_end(chars: &[char], start: usize) -> usize {
    let mut end = start;
    while end < chars.len()
        && (chars[end].is_alphanumeric() || chars[end] == '_' || chars[end] == '$')
    {
        end += 1;
    }
    end
}

fn starts_with(chars: &[char], start: usize, pattern: &str) -> bool {
    for (index, expected) in (start..).zip(pattern.chars()) {
        if chars.get(index).copied() != Some(expected) {
            return false;
        }
    }
    true
}

fn find_seq(chars: &[char], start: usize, pattern: &str) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    let important: Vec<char> = pattern.chars().collect();
    if start >= chars.len() {
        return None;
    }
    let last = chars.len().saturating_sub(important.len());
    let mut index = start;
    while index <= last {
        if chars[index..index + important.len()] == important[..] {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn mark(kinds: &mut [SyntaxKind], start: usize, end: usize, kind: SyntaxKind) {
    if start >= end {
        return;
    }
    let finish = end.min(kinds.len());
    for slot in kinds.iter_mut().take(finish).skip(start) {
        if *slot == SyntaxKind::Default || kind != SyntaxKind::Default {
            *slot = kind;
        }
    }
}

fn highlight_json(line: &str) -> Vec<SyntaxKind> {
    let chars: Vec<char> = line.chars().collect();
    let mut kinds = vec![SyntaxKind::Default; chars.len()];
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '"' {
            let end = scan_quoted(&chars, index, '"').unwrap_or(chars.len());
            let mut look = end;
            while look < chars.len() && chars[look].is_whitespace() {
                look += 1;
            }
            let kind = if chars.get(look).copied() == Some(':') {
                SyntaxKind::Property
            } else {
                SyntaxKind::String
            };
            mark(&mut kinds, index, end, kind);
            index = end;
            continue;
        }
        if ch.is_ascii_digit()
            || (ch == '-' && chars.get(index + 1).is_some_and(|c| c.is_ascii_digit()))
        {
            let mut end = index + 1;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric()
                    || chars[end] == '.'
                    || chars[end] == '+'
                    || chars[end] == '-')
            {
                end += 1;
            }
            mark(&mut kinds, index, end, SyntaxKind::Number);
            index = end;
            continue;
        }
        if ch.is_alphabetic() {
            let end = word_end(&chars, index + 1);
            let word: String = chars[index..end].iter().collect();
            if matches!(word.as_str(), "true" | "false" | "null") {
                mark(&mut kinds, index, end, SyntaxKind::Constant);
            }
            index = end;
            continue;
        }
        if "{}[],:".contains(ch) {
            mark(&mut kinds, index, index + 1, SyntaxKind::Operator);
        }
        index += 1;
    }
    kinds
}

fn highlight_yaml(line: &str) -> Vec<SyntaxKind> {
    let chars: Vec<char> = line.chars().collect();
    let mut kinds = vec![SyntaxKind::Default; chars.len()];
    if chars.is_empty() {
        return kinds;
    }
    let first = chars.iter().position(|ch| !ch.is_whitespace());
    let Some(first) = first else {
        return kinds;
    };
    if starts_with(&chars, first, "---") || starts_with(&chars, first, "...") {
        mark(&mut kinds, first, chars.len(), SyntaxKind::Operator);
        return kinds;
    }
    if chars[first] == '#' {
        mark(&mut kinds, first, chars.len(), SyntaxKind::Comment);
        return kinds;
    }
    if chars[first] == '-' && chars.get(first + 1).is_some_and(|ch| ch.is_whitespace()) {
        mark(&mut kinds, first, first + 1, SyntaxKind::Operator);
    }
    let mut colon: Option<usize> = None;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (position, ch) in chars.iter().enumerate().skip(first) {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if *ch == '\\' {
                escaped = true;
            } else if *ch == active_quote {
                quote = None;
            }
            continue;
        }
        if *ch == '"' || *ch == '\'' {
            quote = Some(*ch);
        } else if *ch == ':' {
            colon = Some(position);
            break;
        } else if *ch == '#' && position > first && chars[position - 1].is_whitespace() {
            mark(&mut kinds, position, chars.len(), SyntaxKind::Comment);
            break;
        }
    }
    if let Some(colon_pos) = colon {
        let key_end = (first..colon_pos)
            .rev()
            .find(|position| !chars[*position].is_whitespace())
            .map(|position| position + 1)
            .unwrap_or(colon_pos);
        mark(&mut kinds, first, key_end, SyntaxKind::Property);
        mark(&mut kinds, colon_pos, colon_pos + 1, SyntaxKind::Operator);
    }
    let mut index = colon.map_or(first, |position| position + 1);
    while index < chars.len() {
        let ch = chars[index];
        if ch == '#' && (index == 0 || chars[index - 1].is_whitespace()) {
            mark(&mut kinds, index, chars.len(), SyntaxKind::Comment);
            break;
        }
        if ch == '"' || ch == '\'' {
            let end = scan_quoted(&chars, index, ch).unwrap_or(chars.len());
            mark(&mut kinds, index, end, SyntaxKind::String);
            index = end;
            continue;
        }
        if ch == '&' || ch == '*' {
            let end = word_end(&chars, index + 1);
            mark(&mut kinds, index, end, SyntaxKind::Tag);
            index = end.max(index + 1);
            continue;
        }
        if ch.is_ascii_digit()
            || (ch == '-' && chars.get(index + 1).is_some_and(|c| c.is_ascii_digit()))
        {
            let mut end = index + 1;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric()
                    || chars[end] == '.'
                    || chars[end] == ':'
                    || chars[end] == '-')
            {
                end += 1;
            }
            mark(&mut kinds, index, end, SyntaxKind::Number);
            index = end;
            continue;
        }
        if ch.is_alphabetic() {
            let end = word_end(&chars, index + 1);
            let word: String = chars[index..end].iter().collect();
            if language::constants(Language::Yaml).contains(&word.to_ascii_lowercase().as_str()) {
                mark(&mut kinds, index, end, SyntaxKind::Constant);
            }
            index = end;
            continue;
        }
        index += 1;
    }
    kinds
}

fn highlight_toml(line: &str) -> Vec<SyntaxKind> {
    let chars: Vec<char> = line.chars().collect();
    let mut kinds = vec![SyntaxKind::Default; chars.len()];
    let first = chars.iter().position(|ch| !ch.is_whitespace());
    let Some(first) = first else {
        return kinds;
    };
    if chars[first] == '#' {
        mark(&mut kinds, first, chars.len(), SyntaxKind::Comment);
        return kinds;
    }
    if chars[first] == '[' {
        if let Some(close) = chars.iter().position(|ch| *ch == ']') {
            mark(&mut kinds, first, close + 1, SyntaxKind::Type);
        }
        return kinds;
    }
    let mut equals: Option<usize> = None;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (position, ch) in chars.iter().enumerate().skip(first) {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if *ch == '\\' {
                escaped = true;
            } else if *ch == active {
                quote = None;
            }
            continue;
        }
        if *ch == '"' || *ch == '\'' {
            quote = Some(*ch);
        } else if *ch == '=' {
            equals = Some(position);
            break;
        } else if *ch == '#' {
            mark(&mut kinds, position, chars.len(), SyntaxKind::Comment);
            return kinds;
        }
    }
    let Some(equals_pos) = equals else {
        let mut index = first;
        while index < chars.len() {
            if chars[index] == '#' {
                mark(&mut kinds, index, chars.len(), SyntaxKind::Comment);
                break;
            }
            index += 1;
        }
        return kinds;
    };
    let key_end = (first..equals_pos)
        .rev()
        .find(|position| !chars[*position].is_whitespace())
        .map(|position| position + 1)
        .unwrap_or(equals_pos);
    mark(&mut kinds, first, key_end, SyntaxKind::Property);
    mark(&mut kinds, equals_pos, equals_pos + 1, SyntaxKind::Operator);
    let mut index = equals_pos + 1;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '#' {
            mark(&mut kinds, index, chars.len(), SyntaxKind::Comment);
            break;
        }
        if ch == '"' || ch == '\'' {
            let end = scan_quoted(&chars, index, ch).unwrap_or(chars.len());
            mark(&mut kinds, index, end, SyntaxKind::String);
            index = end;
            continue;
        }
        if ch.is_ascii_digit() {
            let mut end = index + 1;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric()
                    || chars[end] == '-'
                    || chars[end] == ':'
                    || chars[end] == '.'
                    || chars[end] == '_')
            {
                end += 1;
            }
            mark(&mut kinds, index, end, SyntaxKind::Number);
            index = end;
            continue;
        }
        if ch.is_alphabetic() {
            let end = word_end(&chars, index + 1);
            let word: String = chars[index..end].iter().collect();
            if matches!(word.as_str(), "true" | "false" | "inf" | "nan") {
                mark(&mut kinds, index, end, SyntaxKind::Constant);
            }
            index = end;
            continue;
        }
        index += 1;
    }
    kinds
}

fn highlight_markdown(lines: &[String]) -> Vec<Vec<SyntaxKind>> {
    let mut result = Vec::with_capacity(lines.len());
    let mut in_fence = false;
    let mut fence_marker = String::new();
    for line in lines {
        let chars: Vec<char> = line.chars().collect();
        let mut kinds = vec![SyntaxKind::Default; chars.len()];
        let trimmed_start = chars.iter().position(|ch| !ch.is_whitespace());
        let Some(start) = trimmed_start else {
            result.push(kinds);
            continue;
        };
        let tail: String = chars[start..].iter().collect();
        let is_tick_fence = tail.starts_with("```");
        let is_tilde_fence = tail.starts_with("~~~");
        if is_tick_fence || is_tilde_fence {
            let marker = if is_tick_fence { "```" } else { "~~~" };
            mark(&mut kinds, start, chars.len(), SyntaxKind::Tag);
            if in_fence && fence_marker == marker {
                in_fence = false;
                fence_marker.clear();
            } else if !in_fence {
                in_fence = true;
                fence_marker = marker.to_string();
            }
            result.push(kinds);
            continue;
        }
        if in_fence {
            mark(&mut kinds, 0, chars.len(), SyntaxKind::Code);
            result.push(kinds);
            continue;
        }
        if chars[start] == '#' {
            let count = chars[start..].iter().take_while(|ch| **ch == '#').count();
            if count <= 6 && chars.get(start + count).is_none_or(|ch| ch.is_whitespace()) {
                mark(&mut kinds, start, chars.len(), SyntaxKind::Heading);
                result.push(kinds);
                continue;
            }
        }
        if chars[start] == '>' {
            mark(&mut kinds, start, start + 1, SyntaxKind::Tag);
        }
        if (chars[start] == '-' || chars[start] == '*' || chars[start] == '+')
            && chars.get(start + 1).is_some_and(|ch| ch.is_whitespace())
        {
            mark(&mut kinds, start, start + 1, SyntaxKind::Operator);
        }
        if chars[start].is_ascii_digit() {
            let end = chars[start..]
                .iter()
                .position(|ch| !ch.is_ascii_digit())
                .map(|offset| start + offset)
                .unwrap_or(chars.len());
            if chars.get(end).copied() == Some('.') {
                mark(&mut kinds, start, end + 1, SyntaxKind::Operator);
            }
        }
        let mut index = start;
        while index < chars.len() {
            let ch = chars[index];
            if ch == '\\' {
                index += 2;
                continue;
            }
            if ch == '`' {
                let end = find_char(&chars, index + 1, '`').map(|position| position + 1);
                let finish = end.unwrap_or(chars.len());
                mark(&mut kinds, index, finish, SyntaxKind::Code);
                index = finish;
                continue;
            }
            if ch == '[' {
                if let Some(close) = find_char(&chars, index + 1, ']') {
                    if chars.get(close + 1).copied() == Some('(') {
                        if let Some(paren) = find_char(&chars, close + 2, ')') {
                            mark(&mut kinds, index, paren + 1, SyntaxKind::Link);
                            index = paren + 1;
                            continue;
                        }
                    }
                }
            }
            if ch == '<' {
                if let Some(close) = find_char(&chars, index + 1, '>') {
                    mark(&mut kinds, index, close + 1, SyntaxKind::Tag);
                    index = close + 1;
                    continue;
                }
            }
            if ch == '*' || ch == '_' {
                let mut end = index;
                while end < chars.len() && chars[end] == ch {
                    end += 1;
                }
                if end - index >= 2 {
                    mark(&mut kinds, index, end, SyntaxKind::Operator);
                    index = end;
                    continue;
                }
            }
            index += 1;
        }
        result.push(kinds);
    }
    result
}

fn find_char(chars: &[char], start: usize, needle: char) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, ch)| **ch == needle)
        .map(|(index, _)| index)
}
