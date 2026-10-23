use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::platform;

static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();

pub fn init() {
    let path = platform::log_path();
    if let Some(parent) = path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            eprintln!("artef: не удалось создать каталог лога: {error}");
            return;
        }
    }
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => {
            let _ = LOG_FILE.set(Mutex::new(file));
            write_line("INFO", "artef запущен");
        }
        Err(error) => {
            eprintln!("artef: не удалось открыть лог {}: {error}", path.display());
        }
    }
}

pub fn write_line(level: &str, message: &str) {
    let Some(lock) = LOG_FILE.get() else {
        return;
    };
    let Ok(mut file) = lock.lock() else {
        return;
    };
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let _ = writeln!(file, "[{seconds}] [{level}] {message}");
    let _ = file.flush();
}

pub fn info(message: &str) {
    write_line("INFO", message);
}

pub fn warn(message: &str) {
    write_line("WARN", message);
}

pub fn error(message: &str) {
    write_line("ERROR", message);
}
