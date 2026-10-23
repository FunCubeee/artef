use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub fn is_termux() -> bool {
    cfg!(target_os = "android") || env::var_os("TERMUX_VERSION").is_some()
}

pub fn home_dir() -> PathBuf {
    if let Some(home) = env::var_os("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(".")
}

pub fn config_dir() -> PathBuf {
    if let Some(base) = env::var_os("XDG_CONFIG_HOME") {
        if !base.is_empty() {
            return PathBuf::from(base).join("artef");
        }
    }
    home_dir().join(".config").join("artef")
}

pub fn state_dir() -> PathBuf {
    if let Some(base) = env::var_os("XDG_STATE_HOME") {
        if !base.is_empty() {
            return PathBuf::from(base).join("artef");
        }
    }
    home_dir().join(".local").join("state").join("artef")
}

pub fn log_path() -> PathBuf {
    state_dir().join("artef.log")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn dict_dir() -> PathBuf {
    config_dir().join("dicts")
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

pub fn find_program(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let direct = PathBuf::from(name);
    if direct.components().count() > 1 || direct.is_absolute() {
        return if is_executable(&direct) {
            Some(direct)
        } else {
            None
        };
    }
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub fn quote_arg(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    let safe = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:@%+=,-".contains(c));
    if safe {
        text.into_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\"'\"'"))
    }
}

pub fn quote_str(value: &str) -> String {
    quote_arg(OsStr::new(value))
}
