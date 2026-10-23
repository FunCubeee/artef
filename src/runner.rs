use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutSource {
    Stdout,
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
                self.state = ParserState::Ground;
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
                        self.state = ParserState::Ground;
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
                    0x07 | 0x00..=0x06 | 0x0b..=0x17 | 0x19 | 0x1c..=0x1f => {
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
                    if self.state != ParserState::Ground {
                        index += 1;
                        continue;
                    }
                }
                ParserState::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        if byte == b'm' {
                            self.apply_sgr();
                        } else if byte == b'H'
                            || byte == b'f'
                            || byte == b'J'
                            || byte == b'K'
                        {
                            self.current.clear();
                            self.visual_col = 0;
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
    pty: StreamBuffer,
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
            pty: StreamBuffer::new(OutSource::Stdout),
            offset: 0,
            follow: true,
            view_height: 10,
            max_offset: 0,
        }
    }

    pub fn clear(&mut self) {
        self.completed.clear();
        self.pty = StreamBuffer::new(OutSource::Stdout);
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

    pub fn feed(&mut self, bytes: &[u8]) {
        self.pty.feed(bytes, &mut self.completed);
        self.trim();
    }

    pub fn finish_streams(&mut self) {
        let mut extra = Vec::new();
        self.pty.finish(&mut extra);
        self.completed.extend(extra);
        self.trim();
        self.follow = true;
    }

    pub fn completed(&self) -> &[OutLine] {
        &self.completed
    }

    pub fn partial_lines(&self) -> Vec<OutLine> {
        if self.pty.current.is_empty() {
            return Vec::new();
        }
        vec![OutLine {
            spans: self.pty.current.clone(),
            source: OutSource::Stdout,
        }]
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
    Data { bytes: Vec<u8> },
    ReaderDone,
}

static LIVE_PID: std::sync::OnceLock<std::sync::Mutex<Option<libc::pid_t>>> =
    std::sync::OnceLock::new();

fn live_pid_cell() -> &'static std::sync::Mutex<Option<libc::pid_t>> {
    LIVE_PID.get_or_init(|| std::sync::Mutex::new(None))
}

/// Store the pid of the running child so panic hooks can kill it.
pub fn register_live_pid(pid: Option<libc::pid_t>) {
    if let Ok(mut cell) = live_pid_cell().lock() {
        *cell = pid;
    }
}

pub fn kill_registered_child() {
    let pid = live_pid_cell().lock().ok().and_then(|mut cell| cell.take());
    if let Some(pid) = pid {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        for _ in 0..60 {
            if wait_pid(pid).is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        let _ = wait_pid(pid);
    }
}

struct ActiveRun {
    master_fd: i32,
    pid: u32,
    receiver: Receiver<RunEvent>,
    reader_done: bool,
}

impl ActiveRun {
    fn new(master_fd: i32, pid: u32, receiver: Receiver<RunEvent>) -> Self {
        Self {
            master_fd,
            pid,
            receiver,
            reader_done: false,
        }
    }
}

pub struct Runner {
    pub output: OutputBuffer,
    pub exit: Option<RunExit>,
    active: Option<ActiveRun>,
    winsize: (u16, u16),
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
            winsize: (80, 24),
        }
    }

    pub fn set_winsize(&mut self, cols: u16, rows: u16) {
        self.winsize = (cols.max(1), rows.max(1));
        if let Some(active) = &self.active {
            let size = libc::winsize {
                ws_col: self.winsize.0,
                ws_row: self.winsize.1,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            unsafe {
                libc::ioctl(
                    active.master_fd,
                    libc::TIOCSWINSZ,
                    &size as *const libc::winsize,
                );
            }
        }
    }

    pub fn start(&mut self, plan: &RunPlan) -> Result<(), String> {
        self.shutdown();
        self.output.clear();
        self.exit = None;
        self.output.add_system(format!("$ {}", plan.command));
        self.spawn_pty(plan)
    }

    fn spawn_pty(&mut self, plan: &RunPlan) -> Result<(), String> {
        let shell = platform::find_program("sh")
            .or_else(|| platform::find_program("bash"))
            .unwrap_or_else(|| PathBuf::from("/system/bin/sh"));
        let shell_c = c_string(&shell.to_string_lossy())?;
        let command_c = c_string(&plan.command)?;
        let cwd_c = c_string(&plan.cwd.to_string_lossy())?;
        let (master_fd, slave_fd) = open_pty(self.winsize)?;

        let fork_result = unsafe { libc::fork() };
        if fork_result < 0 {
            unsafe {
                libc::close(master_fd);
                libc::close(slave_fd);
            }
            return Err(format!("fork не удался: {}", std::io::Error::last_os_error()));
        }
        if fork_result == 0 {
            unsafe {
                libc::close(master_fd);
                if libc::setsid() < 0 {
                    libc::_exit(121);
                }
                if libc::ioctl(
                    slave_fd,
                    libc::TIOCSCTTY,
                    std::ptr::null_mut::<libc::c_void>(),
                ) < 0
                {
                    libc::_exit(122);
                }
                if libc::dup2(slave_fd, 0) < 0
                    || libc::dup2(slave_fd, 1) < 0
                    || libc::dup2(slave_fd, 2) < 0
                {
                    libc::_exit(123);
                }
                if slave_fd > 2 {
                    libc::close(slave_fd);
                }
                if libc::chdir(cwd_c.as_ptr()) < 0 {
                    libc::_exit(124);
                }
                std::env::set_var("PYTHONUNBUFFERED", "1");
                std::env::set_var("PYTHONIOENCODING", "utf-8");
                std::env::set_var("FORCE_COLOR", "1");
                std::env::set_var("CLICOLOR_FORCE", "1");
                std::env::remove_var("NO_COLOR");
                let argv: [*const libc::c_char; 4] = [
                    shell_c.as_ptr(),
                    c"-c".as_ptr(),
                    command_c.as_ptr(),
                    std::ptr::null(),
                ];
                libc::execv(shell_c.as_ptr(), argv.as_ptr().cast_mut());
                libc::_exit(127);
            }
        }

        let pid = fork_result as u32;
        unsafe {
            libc::close(slave_fd);
        }
        register_live_pid(Some(pid as libc::pid_t));

        let (sender, receiver) = mpsc::channel::<RunEvent>();
        {
            let sender = sender.clone();
            thread::spawn(move || {
                read_pty(master_fd, sender);
            });
        }
        self.output.follow = true;
        self.active = Some(ActiveRun::new(master_fd, pid, receiver));
        Ok(())
    }

    pub fn tick(&mut self) {
        let finished;
        let mut drained = false;
        {
            let Some(active) = self.active.as_mut() else {
                return;
            };
            loop {
                match active.receiver.try_recv() {
                    Ok(RunEvent::Data { bytes }) => {
                        self.output.feed(&bytes);
                    }
                    Ok(RunEvent::ReaderDone) => {
                        active.reader_done = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => {
                        drained = true;
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        active.reader_done = true;
                        break;
                    }
                }
            }

            if active.reader_done && active.master_fd >= 0 {
                unsafe {
                    libc::close(active.master_fd);
                }
                active.master_fd = -1;
            }

            if let Some(exit) = wait_pid(active.pid as libc::pid_t) {
                register_live_pid(None);
                if self.exit.is_none() {
                    self.exit = Some(exit);
                }
            }

            let exited = self.exit.is_some();
            finished = exited && active.reader_done;
            let _ = drained;
        }
        if finished {
            self.output.finish_streams();
            self.active = None;
            if let Some(exit) = self.exit {
                if exit.success {
                    self.output.add_system("[artef] процесс завершён успешно");
                } else {
                    let code = match exit.code {
                        Some(code) => code.to_string(),
                        None => "сигнал".to_string(),
                    };
                    self.output.add_system(format!("[artef] процесс завершился, код {code}"));
                }
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.active.is_some() && self.exit.is_none()
    }

    pub fn send_input(&mut self, bytes: &[u8]) {
        let Some(active) = &self.active else {
            return;
        };
        if active.master_fd < 0 {
            return;
        }
        let mut written = 0usize;
        while written < bytes.len() {
            let count = unsafe {
                libc::write(
                    active.master_fd,
                    bytes[written..].as_ptr().cast(),
                    bytes.len() - written,
                )
            };
            if count <= 0 {
                break;
            }
            written += count as usize;
        }
    }

    pub fn shutdown(&mut self) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        let deadline = Instant::now() + Duration::from_millis(600);
        unsafe {
            libc::kill(active.pid as i32, libc::SIGTERM);
        }
        register_live_pid(None);
        loop {
            if wait_pid(active.pid as libc::pid_t).is_some() {
                break;
            }
            if Instant::now() >= deadline {
                unsafe {
                    libc::kill(active.pid as i32, libc::SIGKILL);
                }
                let _ = wait_pid(active.pid as libc::pid_t);
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
        if active.master_fd >= 0 {
            unsafe {
                libc::close(active.master_fd);
            }
            active.master_fd = -1;
        }
        let deadline = Instant::now() + Duration::from_millis(200);
        while let Ok(event) = active.receiver.try_recv() {
            match event {
                RunEvent::Data { bytes } => {
                    self.output.feed(&bytes);
                }
                _ => break,
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        if !self.output.partial_lines().is_empty() {
            self.output.finish_streams();
        }
        if self.exit.is_none() {
            self.exit = Some(RunExit {
                code: None,
                success: false,
            });
            self.output.add_system("[artef] процесс остановлен (прерван)");
        }
    }
}

fn read_pty(fd: i32, sender: mpsc::Sender<RunEvent>) {
    let mut buffer = [0u8; 8192];
    loop {
        let count = unsafe {
            libc::read(
                fd,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if count <= 0 {
            break;
        }
        let count = count as usize;
        if sender
            .send(RunEvent::Data {
                bytes: buffer[..count].to_vec(),
            })
            .is_err()
        {
            break;
        }
    }
    let _ = sender.send(RunEvent::ReaderDone);
}

fn open_pty(winsize: (u16, u16)) -> Result<(i32, i32), String> {
    unsafe {
        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        let ws = libc::winsize {
            ws_col: winsize.0,
            ws_row: winsize.1,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &ws) < 0
        {
            return Err(format!(
                "openpty не удался: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok((master, slave))
    }
}

fn c_string(value: &str) -> Result<std::ffi::CString, String> {
    std::ffi::CString::new(value.as_bytes())
        .map_err(|error| format!("недопустимый байт в пути или команде: {error}"))
}

fn wait_pid(pid: libc::pid_t) -> Option<RunExit> {
    unsafe {
        let mut status: libc::c_int = 0;
        let result = libc::waitpid(pid, &mut status, libc::WNOHANG);
        if result == 0 {
            return None;
        }
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                return None;
            }
            return Some(RunExit {
                code: None,
                success: false,
            });
        }
        if libc::WIFEXITED(status) {
            let code = libc::WEXITSTATUS(status);
            return Some(RunExit {
                code: Some(code),
                success: code == 0,
            });
        }
        if libc::WIFSIGNALED(status) {
            return Some(RunExit {
                code: None,
                success: false,
            });
        }
        None
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
    let plan = plan_for_extension(file, &extension, run_rules, workspace_root)?;
    Ok(plan)
}

pub fn plan_for_extension(
    file: &Path,
    extension: &str,
    run_rules: &BTreeMap<String, String>,
    workspace_root: &Path,
) -> Result<RunPlan, String> {
    let cargo_root = find_cargo_root(file);
    let mut template = run_rules
        .get(&format!(".{extension}"))
        .or_else(|| run_rules.get(extension))
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_template(extension).to_string());
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
        workspace_root.to_path_buf()
    };
    let temp = temporary_executable(file);
    let mut substitutions: Vec<(&str, String)> = vec![
        ("{file}", platform::quote_arg(file.as_os_str())),
        ("{dir}", platform::quote_arg(file_dir.as_os_str())),
        ("{name}", platform::quote_str(&file_name_text(file))),
        ("{stem}", platform::quote_str(&file_stem_text(file))),
        ("{ext}", extension.to_string()),
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
        "{python}" => &["python3", "python"],
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
        "не найдена программа '{label}', установите её и повторите запуск (pkg install {label})"
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
    std::env::temp_dir().join(format!("artef-{}-{}", std::process::id(), stem))
}

pub fn shebang_command(file: &Path) -> Option<String> {
    let content = fs::read_to_string(file).ok()?;
    let first = content.lines().next()?;
    if !first.starts_with("#!") {
        return None;
    }
    let shebang = first.trim_start_matches("#!").trim();
    if shebang.is_empty() {
        return None;
    }
    if shebang.contains("env ") {
        let mut parts = shebang.split_whitespace();
        let _env = parts.next()?;
        let interpreter = parts.next()?;
        let mut command = format!("{interpreter} {file:?}");
        for argument in parts {
            command.push(' ');
            command.push_str(argument);
        }
        return Some(command);
    }
    Some(format!("{shebang} {file:?}"))
}

fn file_name_text(file: &Path) -> String {
    file.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string()
}

fn file_stem_text(file: &Path) -> String {
    file.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_templates_cover_popular_extensions() {
        for extension in ["py", "js", "ts", "go", "c", "cpp", "sh"] {
            assert!(
                !default_template(extension).is_empty(),
                "нет шаблона для {extension}"
            );
        }
    }

    #[test]
    fn substitute_replaces_tokens() {
        let result = substitute(
            "{python} -u {file}",
            &[("{python}", "py".into()), ("{file}", "main.py".into())],
        );
        assert_eq!(result, "py -u main.py");
    }

    #[test]
    fn output_buffer_wraps_carriage_rewrites() {
        let mut buffer = OutputBuffer::new();
        buffer.feed(b"\x1b[1;32mloading\x1b[0m\r\x08\x1b[K\r done\n");
        buffer.finish_streams();
        let lines = buffer.completed();
        assert!(!lines.is_empty());
    }

    #[test]
    fn shebang_or_none() {
        let dir = std::env::temp_dir().join(format!("artef-test-{}", std::process::id()));
        fs::create_dir_all(&dir).ok();
        let path = dir.join("launcher");
        fs::write(&path, "#!/usr/bin/env python3\nprint(1)\n").unwrap();
        let command = shebang_command(&path).unwrap();
        assert!(command.starts_with("python3"), "команда: {command}");
        let plain = dir.join("nobody");
        fs::write(&plain, "print(2)\n").unwrap();
        assert!(shebang_command(&plain).is_none());
        fs::remove_dir_all(&dir).ok();
    }
}
