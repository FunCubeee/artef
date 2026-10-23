use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn is_termux() -> bool {
    cfg!(target_os = "android") || env::var_os("TERMUX_VERSION").is_some()
}

pub fn home_dir() -> PathBuf {
    if let Some(home) = env::var_os("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    if cfg!(windows) {
        if let Some(profile) = env::var_os("USERPROFILE") {
            if !profile.is_empty() {
                return PathBuf::from(profile);
            }
        }
        let drive = env::var_os("HOMEDRIVE").unwrap_or_default();
        let path = env::var_os("HOMEPATH").unwrap_or_default();
        let joined = PathBuf::from(drive).join(path);
        if !joined.as_os_str().is_empty() {
            return joined;
        }
    }
    PathBuf::from(".")
}

pub fn config_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(base) = env::var_os("APPDATA") {
            if !base.is_empty() {
                return PathBuf::from(base).join("artef");
            }
        }
    }
    if let Some(base) = env::var_os("XDG_CONFIG_HOME") {
        if !base.is_empty() {
            return PathBuf::from(base).join("artef");
        }
    }
    home_dir().join(".config").join("artef")
}

pub fn state_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(base) = env::var_os("LOCALAPPDATA") {
            if !base.is_empty() {
                return PathBuf::from(base).join("artef").join("state");
            }
        }
    }
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
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
    #[cfg(windows)]
    {
        let extensions: Vec<String> = env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .filter(|item| !item.is_empty())
            .map(|item| item.to_ascii_lowercase())
            .collect();
        let mut names = vec![name.to_string()];
        let lower = name.to_ascii_lowercase();
        if !extensions.iter().any(|ext| lower.ends_with(ext)) {
            for ext in &extensions {
                names.push(format!("{name}{ext}"));
            }
        }
        for dir in env::split_paths(&path_var) {
            for candidate_name in &names {
                let candidate = dir.join(candidate_name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

pub fn quote_arg(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    #[cfg(windows)]
    {
        format!("\"{}\"", text.replace('"', "\\\""))
    }
    #[cfg(not(windows))]
    {
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
}

pub fn quote_str(value: &str) -> String {
    quote_arg(OsStr::new(value))
}

#[cfg(unix)]
fn external_script(command: &str, cwd: &Path) -> String {
    format!(
        "cd {} && {}; __artef_status=$?; printf '\\n[artef] exit %s. Press Enter to close.\\n' \"$__artef_status\"; read __artef_done",
        quote_arg(cwd.as_os_str()),
        command
    )
}

#[cfg(unix)]
pub fn launch_external(command: &str, cwd: &Path) -> Result<String, String> {
    let script = external_script(command, cwd);
    let shell = if find_program("bash").is_some() {
        "bash"
    } else {
        "sh"
    };
    let candidates: [(&str, Vec<&str>); 7] = [
        ("gnome-terminal", vec!["--", shell, "-lc", "{script}"]),
        ("konsole", vec!["-e", shell, "-lc", "{script}"]),
        ("xfce4-terminal", vec!["--command", "{shell_script}"]),
        ("xterm", vec!["-e", shell, "-lc", "{script}"]),
        ("alacritty", vec!["-e", shell, "-lc", "{script}"]),
        ("kitty", vec![shell, "-lc", "{script}"]),
        ("foot", vec![shell, "-lc", "{script}"]),
    ];
    let mut failures = Vec::new();
    for (program, template) in candidates {
        if find_program(program).is_none() {
            continue;
        }
        let mut command_line = Command::new(program);
        for arg in template {
            if arg == "{script}" {
                command_line.arg(&script);
            } else if arg == "{shell_script}" {
                command_line.arg(format!("{} -lc {}", shell, quote_arg(OsStr::new(&script))));
            } else {
                command_line.arg(arg);
            }
        }
        match command_line
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok(program.to_string()),
            Err(error) => failures.push(format!("{program}: {error}")),
        }
    }
    if failures.is_empty() {
        Err("не найден внешний терминал (gnome-terminal, konsole, xterm)".to_string())
    } else {
        Err(format!(
            "не удалось открыть терминал: {}",
            failures.join("; ")
        ))
    }
}

#[cfg(windows)]
pub fn launch_external(command: &str, cwd: &Path) -> Result<String, String> {
    let cwd_text = cwd.to_string_lossy();
    if find_program("wt.exe").is_some() || find_program("wt").is_some() {
        let wt = if find_program("wt.exe").is_some() {
            "wt.exe"
        } else {
            "wt"
        };
        match Command::new(wt)
            .args([
                "-d",
                &cwd_text,
                "cmd.exe",
                "/K",
                &format!("{command} & echo. & pause"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok("Windows Terminal".to_string()),
            Err(error) => {
                crate::logging::error(&format!("не удалось запустить wt.exe: {error}"));
            }
        }
    }
    let mut started = Command::new("cmd.exe");
    started
        .args(["/C", "start", "", "/D"])
        .arg(&*cwd_text)
        .arg("cmd.exe")
        .args(["/K", &format!("{command} & echo. & pause")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match started.spawn() {
        Ok(_) => Ok("start".to_string()),
        Err(error) => Err(format!("не удалось открыть внешнее окно: {error}")),
    }
}

#[cfg(not(any(unix, windows)))]
pub fn launch_external(_command: &str, _cwd: &Path) -> Result<String, String> {
    Err("внешний запуск не поддержан на этой платформе".to_string())
}
