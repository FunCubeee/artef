use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

use crate::language::Language;
use crate::logging;
use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutSource {
    Stdout,
    Stderr,
    System,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutStyle {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub modifier: Modifier,
}

impl Default for OutStyle {
    fn default() -> Self {
        Self {
            fg: None,
            bg: None,
            modifier: Modifier::empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutSpan {
    pub text: String,
    pub style: OutStyle,
}

#[derive(Debug, Clone)]
pub struct OutLine {
    pub spans: Vec<OutSpan>,
    pub source: OutSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunExit {
    pub code: Option<i32>,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPlan {
    pub file: PathBuf,
    pub cwd: PathBuf,
    pub command: String,
}

#[derive(Debug, Clone)]
pub struct GuiWarning {
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserState {
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
    Skip,
}

struct StreamBuffer {
    source: OutSource,
    state: ParserState,
    csi: Vec<u8>,
    style: OutStyle,
    current: Vec<OutSpan>,
    pending: Vec<u8>,
    visual_col: usize,
    carriage_pending: bool,
}

impl StreamBuffer {
    fn new(source: OutSource) -> Self {
        Self {
            source,
            state: ParserState::Ground,
            csi: Vec::new(),
            style: OutStyle::default(),
            current: Vec::new(),
            pending: Vec::new(),
            visual_col: 0,
            carriage_pending: false,
        }
    }

    fn feed(&mut self, bytes: &[u8], output: &mut Vec<OutLine>) {
        let mut index = 0usize;
        while index < bytes.len() {
            let byte = bytes[index];
            if self.carriage_pending {
                if byte == b'\n' {
                    self.carriage_pending = false;
                    self.finish_line(output);
                    index += 1;
                    continue;
                }
                if byte == b'\r' {
                    index += 1;
                    continue;
                }
                self.flush_pending();
                self.current.clear();
                self.visual_col = 0;
                self.carriage_pending = false;
            }

            match self.state {
                ParserState::Ground => match byte {
                    0x1b => {
                        self.flush_pending();
                        self.state = ParserState::Escape;
                    }
                    b'\n' => {
                        self.flush_pending();
                        self.finish_line(output);
                    }
                    b'\r' => {
                        self.flush_pending();
                        self.carriage_pending = true;
                    }
                    b'\t' => {
                        self.flush_pending();
                        let spaces = 4 - (self.visual_col % 4);
                        let text = " ".repeat(spaces);
                        self.append_text(&text);
                    }
                    0x08 => {
                        self.flush_pending();
                        self.pop_last();
                    }
                    value if value < 0x20 => {
                        self.flush_pending();
                    }
                    value => {
                        self.pending.push(value);
                    }
                },
                ParserState::Escape => {
                    self.state = match byte {
                        b'[' => {
                            self.csi.clear();
                            ParserState::Csi
                        }
                        b']' | b'P' | b'^' | b'_' => ParserState::Osc,
                        b'(' | b')' | b'*' | b'+' | b'#' | b'%' => ParserState::Skip,
                        0x1b => ParserState::Escape,
                        _ => ParserState::Ground,
                    };
                }
                ParserState::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        if byte == b'm' {
                            self.apply_sgr();
                        }
                        self.csi.clear();
                        self.state = ParserState::Ground;
                    } else if self.csi.len() < 128 {
                        self.csi.push(byte);
                    } else {
                        self.csi.clear();
                        self.state = ParserState::Ground;
                    }
                }
                ParserState::Osc => {
                    if byte == 0x07 {
                        self.state = ParserState::Ground;
                    } else if byte == 0x1b {
                        self.state = ParserState::OscEscape;
                    }
                }
                ParserState::OscEscape => {
                    if byte == b'\\' {
                        self.state = ParserState::Ground;
                    } else {
                        self.state = ParserState::Osc;
                    }
                }
                ParserState::Skip => {
                    self.state = ParserState::Ground;
                }
            }
            index += 1;
        }
        self.flush_pending();
    }

    fn finish(&mut self, output: &mut Vec<OutLine>) {
        self.flush_pending_final();
        if !self.current.is_empty() {
            self.finish_line(output);
        }
        self.carriage_pending = false;
    }

    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        let mut slice = pending.as_slice();
        loop {
            if slice.is_empty() {
                break;
            }
            match std::str::from_utf8(slice) {
                Ok(text) => {
                    self.append_text(text);
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid > 0 {
                        let text = std::str::from_utf8(&slice[..valid]).unwrap_or("");
                        self.append_text(text);
                    }
                    match error.error_len() {
                        Some(length) => {
                            self.append_text("\u{fffd}");
                            if valid + length >= slice.len() {
                                break;
                            }
                            slice = &slice[valid + length..];
                        }
                        None => {
                            self.pending = slice[valid..].to_vec();
                            break;
                        }
                    }
                }
            }
        }
    }

    fn flush_pending_final(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        self.append_text(&String::from_utf8_lossy(&pending));
    }

    fn append_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.visual_col += UnicodeWidthStr::width(text);
        if let Some(last) = self.current.last_mut() {
            if last.style == self.style {
                last.text.push_str(text);
                return;
            }
        }
        self.current.push(OutSpan {
            text: text.to_string(),
            style: self.style,
        });
    }

    fn finish_line(&mut self, output: &mut Vec<OutLine>) {
        let spans = std::mem::take(&mut self.current);
        output.push(OutLine {
            spans,
            source: self.source,
        });
        self.visual_col = 0;
        self.carriage_pending = false;
    }

    fn pop_last(&mut self) {
        loop {
            let Some(last) = self.current.last_mut() else {
                return;
            };
            if let Some(ch) = last.text.pop() {
                if last.text.is_empty() {
                    self.current.pop();
                }
                self.visual_col = self
                    .current
                    .iter()
                    .map(|span| UnicodeWidthStr::width(span.text.as_str()))
                    .sum();
                let _ = ch;
                return;
            }
            self.current.pop();
        }
    }

    fn apply_sgr(&mut self) {
        let text = String::from_utf8_lossy(&self.csi);
        let mut values = Vec::new();
        if text.is_empty() {
            values.push(0u16);
        } else {
            for part in text.split(';') {
                if part.is_empty() {
                    values.push(0);
                } else if let Ok(value) = part.parse::<u16>() {
                    values.push(value);
                } else {
                    return;
                }
            }
        }
        let mut index = 0usize;
        while index < values.len() {
            let value = values[index];
            match value {
                0 => self.style = OutStyle::default(),
                1 => self.style.modifier.insert(Modifier::BOLD),
                2 => self.style.modifier.insert(Modifier::DIM),
                3 => self.style.modifier.insert(Modifier::ITALIC),
                4 => self.style.modifier.insert(Modifier::UNDERLINED),
                7 => self.style.modifier.insert(Modifier::REVERSED),
                8 => self.style.modifier.insert(Modifier::HIDDEN),
                9 => self.style.modifier.insert(Modifier::CROSSED_OUT),
                21 | 22 => self.style.modifier.remove(Modifier::BOLD | Modifier::DIM),
                23 => self.style.modifier.remove(Modifier::ITALIC),
                24 => self.style.modifier.remove(Modifier::UNDERLINED),
                27 => self.style.modifier.remove(Modifier::REVERSED),
                29 => self.style.modifier.remove(Modifier::CROSSED_OUT),
                30..=37 => self.style.fg = Some(ansi_color((value - 30) as u8)),
                38 => {
                    if let Some((color, consumed)) = extended_color(&values, index) {
                        self.style.fg = Some(color);
                        index += consumed;
                        continue;
                    }
                }
                39 => self.style.fg = None,
                40..=47 => self.style.bg = Some(ansi_color((value - 40) as u8)),
                48 => {
                    if let Some((color, consumed)) = extended_color(&values, index) {
                        self.style.bg = Some(color);
                        index += consumed;
                        continue;
                    }
                }
                49 => self.style.bg = None,
                90..=97 => self.style.fg = Some(ansi_color((value - 90 + 8) as u8)),
                100..=107 => self.style.bg = Some(ansi_color((value - 100 + 8) as u8)),
                _ => {}
            }
            index += 1;
        }
    }
}

fn extended_color(values: &[u16], start: usize) -> Option<(Color, usize)> {
    let mode = *values.get(start + 1)?;
    if mode == 5 {
        let number = *values.get(start + 2)?;
        return Some((Color::Indexed(number.min(255) as u8), 3));
    }
    if mode == 2 {
        let red = *values.get(start + 2)?;
        let green = *values.get(start + 3)?;
        let blue = *values.get(start + 4)?;
        return Some((
            Color::Rgb(
                red.min(255) as u8,
                green.min(255) as u8,
                blue.min(255) as u8,
            ),
            5,
        ));
    }
    None
}

fn ansi_color(index: u8) -> Color {
    match index {
        0 => Color::Rgb(0x28, 0x28, 0x28),
        1 => Color::Rgb(0xfb, 0x49, 0x34),
        2 => Color::Rgb(0xb8, 0xbb, 0x26),
        3 => Color::Rgb(0xfa, 0xbd, 0x2f),
        4 => Color::Rgb(0x83, 0xa5, 0x98),
        5 => Color::Rgb(0xd3, 0x86, 0x9b),
        6 => Color::Rgb(0x8e, 0xc0, 0x7c),
        7 => Color::Rgb(0xeb, 0xdb, 0xb2),
        8 => Color::Rgb(0x92, 0x83, 0x74),
        9 => Color::Rgb(0xfb, 0x49, 0x34),
        10 => Color::Rgb(0xb8, 0xbb, 0x26),
        11 => Color::Rgb(0xfa, 0xbd, 0x2f),
        12 => Color::Rgb(0x83, 0xa5, 0x98),
        13 => Color::Rgb(0xd3, 0x86, 0x9b),
        14 => Color::Rgb(0x8e, 0xc0, 0x7c),
        _ => Color::Rgb(0xeb, 0xdb, 0xb2),
    }
}

pub struct OutputBuffer {
    completed: Vec<OutLine>,
    stdout: StreamBuffer,
    stderr: StreamBuffer,
    pub offset: usize,
    pub follow: bool,
    pub view_height: usize,
    pub max_offset: usize,
}

impl Default for OutputBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputBuffer {
    pub fn new() -> Self {
        Self {
            completed: Vec::new(),
            stdout: StreamBuffer::new(OutSource::Stdout),
            stderr: StreamBuffer::new(OutSource::Stderr),
            offset: 0,
            follow: true,
            view_height: 10,
            max_offset: 0,
        }
    }

    pub fn clear(&mut self) {
        self.completed.clear();
        self.stdout = StreamBuffer::new(OutSource::Stdout);
        self.stderr = StreamBuffer::new(OutSource::Stderr);
        self.offset = 0;
        self.follow = true;
        self.max_offset = 0;
    }

    pub fn add_system(&mut self, text: impl Into<String>) {
        self.completed.push(OutLine {
            spans: vec![OutSpan {
                text: text.into(),
                style: OutStyle::default(),
            }],
            source: OutSource::System,
        });
        self.trim();
    }

    pub fn add_error(&mut self, text: impl Into<String>) {
        self.completed.push(OutLine {
            spans: vec![OutSpan {
                text: text.into(),
                style: OutStyle::default(),
            }],
            source: OutSource::Error,
        });
        self.trim();
    }

    pub fn feed(&mut self, stderr: bool, bytes: &[u8]) {
        if stderr {
            self.stderr.feed(bytes, &mut self.completed);
        } else {
            self.stdout.feed(bytes, &mut self.completed);
        }
        self.trim();
    }

    pub fn finish_streams(&mut self) {
        let mut extra = Vec::new();
        self.stdout.finish(&mut extra);
        self.stderr.finish(&mut extra);
        self.completed.extend(extra);
        self.trim();
        self.follow = true;
    }

    pub fn completed(&self) -> &[OutLine] {
        &self.completed
    }

    pub fn partial_lines(&self) -> Vec<OutLine> {
        let mut result = Vec::new();
        if !self.stdout.current.is_empty() {
            result.push(OutLine {
                spans: self.stdout.current.clone(),
                source: OutSource::Stdout,
            });
        }
        if !self.stderr.current.is_empty() {
            result.push(OutLine {
                spans: self.stderr.current.clone(),
                source: OutSource::Stderr,
            });
        }
        result
    }

    pub fn note_view(&mut self, height: usize, max_offset: usize) {
        self.view_height = height.max(1);
        self.max_offset = max_offset;
        if self.follow {
            self.offset = self.max_offset;
        } else {
            self.offset = self.offset.min(self.max_offset);
            if self.offset == self.max_offset {
                self.follow = true;
            }
        }
    }

    pub fn page_up(&mut self) {
        if self.max_offset == 0 && self.offset == 0 {
            return;
        }
        self.follow = false;
        let page = self.view_height.saturating_sub(1).max(1);
        self.offset = self.offset.saturating_sub(page);
    }

    pub fn page_down(&mut self) {
        let page = self.view_height.saturating_sub(1).max(1);
        self.offset = (self.offset + page).min(self.max_offset);
        if self.offset >= self.max_offset {
            self.follow = true;
        }
    }

    fn trim(&mut self) {
        const LIMIT: usize = 20000;
        if self.completed.len() > LIMIT {
            let remove = self.completed.len() - LIMIT;
            self.completed.drain(0..remove);
            self.offset = self.offset.saturating_sub(remove);
        }
    }
}

enum RunEvent {
    Data { stderr: bool, bytes: Vec<u8> },
    ReaderDone,
}

struct ActiveRun {
    child: Option<Child>,
    receiver: Receiver<RunEvent>,
    readers_left: usize,
}

pub struct Runner {
    pub output: OutputBuffer,
    pub exit: Option<RunExit>,
    active: Option<ActiveRun>,
}

impl Default for Runner {
    fn default() -> Self {
        Self::new()
    }
}

impl Runner {
    pub fn new() -> Self {
        Self {
            output: OutputBuffer::new(),
            exit: None,
            active: None,
        }
    }

    pub fn start(&mut self, plan: &RunPlan) -> Result<(), String> {
        self.shutdown();
        self.output.clear();
        self.exit = None;
        self.output.add_system(format!("$ {}", plan.command));
        let mut command = shell_command(&plan.command);
        command
            .current_dir(&plan.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("FORCE_COLOR", "1")
            .env("CLICOLOR_FORCE", "1")
            .env_remove("NO_COLOR");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0000_0200);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("не удалось запустить программу: {error}"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (sender, receiver) = mpsc::channel();
        let mut readers = 0usize;
        if let Some(stream) = stdout {
            readers += 1;
            let sender = sender.clone();
            thread::spawn(move || {
                read_stream(stream, false, sender);
            });
        }
        if let Some(stream) = stderr {
            readers += 1;
            let sender = sender.clone();
            thread::spawn(move || {
                read_stream(stream, true, sender);
            });
        }
        drop(sender);
        self.output.follow = true;
        self.active = Some(ActiveRun {
            child: Some(child),
            receiver,
            readers_left: readers,
        });
        Ok(())
    }

    pub fn tick(&mut self) {
        let finished;
        {
            let Some(active) = self.active.as_mut() else {
                return;
            };
            loop {
                match active.receiver.try_recv() {
                    Ok(RunEvent::Data { stderr, bytes }) => {
                        self.output.feed(stderr, &bytes);
                    }
                    Ok(RunEvent::ReaderDone) => {
                        active.readers_left = active.readers_left.saturating_sub(1);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        active.readers_left = 0;
                        break;
                    }
                }
            }

            if let Some(child) = active.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let exit = RunExit {
                            code: status.code(),
                            success: status.success(),
                        };
                        self.exit = Some(exit);
                        active.child = None;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        logging::error(&format!("ошибка ожидания процесса: {error}"));
                        active.child = None;
                        self.exit = Some(RunExit {
                            code: None,
                            success: false,
                        });
                    }
                }
            }

            finished = active.readers_left == 0 && active.child.is_none();
        }
        if finished {
            self.output.finish_streams();
            self.active = None;
        }
    }

    pub fn is_running(&self) -> bool {
        self.active
            .as_ref()
            .and_then(|active| active.child.as_ref())
            .is_some()
    }

    pub fn shutdown(&mut self) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if let Some(mut child) = active.child.take() {
            let pid = child.id();
            terminate_process(&mut child, pid);
        }
        self.active = None;
    }
}

fn read_stream(mut stream: impl Read, stderr: bool, sender: mpsc::Sender<RunEvent>) {
    let mut buffer = [0u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if sender
                    .send(RunEvent::Data {
                        stderr,
                        bytes: buffer[..count].to_vec(),
                    })
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                logging::error(&format!("ошибка чтения вывода: {error}"));
                break;
            }
        }
    }
    let _ = sender.send(RunEvent::ReaderDone);
}

fn terminate_process(child: &mut Child, pid: u32) {
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut process = Command::new("cmd.exe");
        process.args(["/C", command]);
        process
    }
    #[cfg(not(windows))]
    {
        let shell = platform::find_program("sh").unwrap_or_else(|| PathBuf::from("sh"));
        let mut process = Command::new(shell);
        process.arg("-c").arg(command);
        process
    }
}

pub fn build_plan(
    file: &Path,
    run_rules: &BTreeMap<String, String>,
    workspace_root: &Path,
) -> Result<RunPlan, String> {
    if !file.is_file() {
        return Err(format!("{} не является файлом", file.display()));
    }
    let extension = file
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension.is_empty() {
        return Err("у файла нет расширения, правило запуска не найдено".to_string());
    }
    let cargo_root = find_cargo_root(file);
    let mut template = run_rules
        .get(&format!(".{extension}"))
        .or_else(|| run_rules.get(&extension))
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_template(&extension).to_string());
    if template.trim().is_empty() {
        return Err(format!("для .{extension} нет правила запуска"));
    }

    if extension == "rs" && cargo_root.is_none() && template.contains("cargo") {
        template = "{rustc} {file} -o {temp} && {temp}".to_string();
    }

    let file_dir = file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| workspace_root.to_path_buf());
    let run_cwd = if extension == "rs" {
        cargo_root.clone().unwrap_or_else(|| file_dir.clone())
    } else {
        file_dir.clone()
    };
    let temp = temporary_executable(file);
    let mut substitutions: Vec<(&str, String)> = vec![
        ("{file}", platform::quote_arg(file.as_os_str())),
        ("{dir}", platform::quote_arg(file_dir.as_os_str())),
        (
            "{name}",
            platform::quote_str(
                file.file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(""),
            ),
        ),
        (
            "{stem}",
            platform::quote_str(
                file.file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or(""),
            ),
        ),
        ("{ext}", extension.clone()),
        ("{temp}", platform::quote_arg(temp.as_os_str())),
    ];

    let needed_tokens = [
        "{python}", "{node}", "{tsx}", "{bash}", "{cargo}", "{go}", "{cc}", "{cxx}", "{rustc}",
    ];
    for token in needed_tokens {
        if !template.contains(token) {
            continue;
        }
        let resolved = resolve_program_token(token)?;
        substitutions.push((token, resolved));
    }
    let command = substitute(&template, &substitutions);
    if command.trim().is_empty() {
        return Err("команда запуска пуста".to_string());
    }
    Ok(RunPlan {
        file: file.to_path_buf(),
        cwd: run_cwd,
        command,
    })
}

fn default_template(extension: &str) -> &'static str {
    match extension {
        "py" | "pyw" => "{python} -u {file}",
        "js" | "jsx" | "mjs" | "cjs" => "{node} {file}",
        "ts" | "tsx" | "mts" | "cts" => "{tsx} {file}",
        "rs" => "{cargo} run --release",
        "go" => "{go} run {file}",
        "c" | "h" => "{cc} {file} -o {temp} && {temp}",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "{cxx} {file} -o {temp} && {temp}",
        "sh" | "bash" | "zsh" => "{bash} {file}",
        _ => "",
    }
}

fn resolve_program_token(token: &str) -> Result<String, String> {
    let candidates: &[&str] = match token {
        "{python}" => {
            if cfg!(windows) {
                &["python", "python3"]
            } else {
                &["python3", "python"]
            }
        }
        "{node}" => &["node"],
        "{tsx}" => &["tsx", "node"],
        "{bash}" => &["bash", "sh"],
        "{cargo}" => &["cargo"],
        "{go}" => &["go"],
        "{cc}" => &["gcc", "clang", "cc"],
        "{cxx}" => &["g++", "clang++", "c++"],
        "{rustc}" => &["rustc"],
        _ => &[],
    };
    for candidate in candidates {
        if let Some(path) = platform::find_program(candidate) {
            return Ok(platform::quote_arg(path.as_os_str()));
        }
    }
    let label = token.trim_matches(|ch| ch == '{' || ch == '}');
    Err(format!(
        "не найдена программа '{label}', установите её и повторите запуск"
    ))
}

fn substitute(template: &str, substitutions: &[(&str, String)]) -> String {
    let mut result = String::with_capacity(template.len() + 32);
    let mut index = 0usize;
    let bytes = template.as_bytes();
    while index < template.len() {
        if bytes[index] == b'{' {
            let mut matched = false;
            for (token, value) in substitutions {
                if template[index..].starts_with(token) {
                    result.push_str(value);
                    index += token.len();
                    matched = true;
                    break;
                }
            }
            if matched {
                continue;
            }
        }
        let Some(ch) = template[index..].chars().next() else {
            break;
        };
        result.push(ch);
        index += ch.len_utf8();
    }
    result
}

fn find_cargo_root(file: &Path) -> Option<PathBuf> {
    let mut current = file.parent();
    while let Some(directory) = current {
        if directory.join("Cargo.toml").is_file() {
            return Some(directory.to_path_buf());
        }
        current = directory.parent();
    }
    None
}

fn temporary_executable(file: &Path) -> PathBuf {
    let stem: String = file
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("program")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let path = std::env::temp_dir().join(format!("artef-{}-{}", std::process::id(), stem));
    #[cfg(windows)]
    let path = {
        let mut path = path;
        path.set_extension("exe");
        path
    };
    path
}

pub fn gui_warning(file: &Path, language: Language, content: &str) -> Option<GuiWarning> {
    let lower = content.to_ascii_lowercase();
    let tokens: &[&str] = match language {
        Language::Python => &[
            "tkinter",
            "pygame",
            "pyqt",
            "pyside",
            "kivy",
            "wxpython",
            "urwid",
            "npyscreen",
            "textual.app",
            "turtle",
            "matplotlib.pyplot",
            "curses",
        ],
        Language::JavaScript | Language::TypeScript => &[
            "electron",
            "react-native",
            "blessed",
            "neo-blessed",
            "from 'ink'",
            "from \"ink\"",
            "browser-window",
        ],
        Language::Rust => &[
            "eframe",
            "egui",
            "iced",
            "tauri",
            "gtk",
            "winit",
            "bevy",
            "minifb",
            "sdl2",
            "ratatui",
            "crossterm",
            "windows_subsystem",
        ],
        Language::Go => &[
            "fyne.io",
            "ebiten",
            "gioui",
            "tcell",
            "bubbletea",
            "termui",
            "gocui",
        ],
        Language::C | Language::Cpp => &[
            "sdl_",
            "<sdl",
            "<gtk",
            "<qapplication",
            "<qwidget",
            "<x11",
            "<ncurses",
            "<curses",
            "winmain",
            "createwindow",
        ],
        Language::Shell => &["whiptail", "dialog ", "ncurses"],
        _ => &[],
    };
    for token in tokens {
        if lower.contains(token) {
            return Some(GuiWarning {
                reason: format!("обнаружено `{token}`"),
            });
        }
    }
    if language == Language::Rust {
        if let Some(root) = find_cargo_root(file) {
            let manifest = root.join("Cargo.toml");
            if let Ok(text) = fs::read_to_string(manifest) {
                let manifest_lower = text.to_ascii_lowercase();
                let gui_tokens = [
                    "egui", "eframe", "iced", "tauri", "gtk", "winit", "bevy", "sdl2", "minifb",
                ];
                for token in gui_tokens {
                    if manifest_lower.contains(token) {
                        return Some(GuiWarning {
                            reason: format!("Cargo.toml содержит `{token}`"),
                        });
                    }
                }
            }
        }
    }
    None
}
