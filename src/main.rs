mod app;
mod config;
mod dictionary;
mod editor;
mod file_tree;
mod highlight;
mod language;
mod logging;
mod platform;
mod runner;
mod theme;
mod ui;

use std::env;
use std::fs;
use std::io::{self, Stdout};
use std::panic;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::App;

struct Cli {
    path: Option<PathBuf>,
}

enum CliAction {
    Run(Cli),
    Help,
    Version,
}

fn main() {
    if let Err(error) = real_main() {
        eprintln!("artef: {error}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<(), String> {
    let action = parse_cli()?;
    let cli = match action {
        CliAction::Help => {
            print_help();
            return Ok(());
        }
        CliAction::Version => {
            println!("artef {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        CliAction::Run(cli) => cli,
    };
    logging::init();
    let settings = config::load();
    let (root, initial_file) = resolve_start(cli.path)?;
    let mut app = App::new(settings, root, initial_file);
    install_panic_hook();
    let mut terminal = setup_terminal()?;
    let loop_result = run_loop(&mut terminal, &mut app);
    app.runner.shutdown();
    restore_terminal(&mut terminal);
    loop_result
}

fn parse_cli() -> Result<CliAction, String> {
    let args = env::args_os().skip(1);
    let mut path = None;
    let mut positional_only = false;
    for argument in args {
        if !positional_only {
            let text = argument.to_string_lossy();
            if text == "--" {
                positional_only = true;
                continue;
            }
            if text == "--help" || text == "-h" {
                return Ok(CliAction::Help);
            }
            if text == "--version" || text == "-V" {
                return Ok(CliAction::Version);
            }
            if text.starts_with('-') {
                return Err(format!("неизвестный параметр {text}; используйте --help"));
            }
        }
        if path.is_some() {
            return Err("можно указать только один путь проекта".to_string());
        }
        path = Some(PathBuf::from(argument));
    }
    Ok(CliAction::Run(Cli { path }))
}

fn print_help() {
    println!(
        concat!(
            "artef {}\n",
            "Терминальный редактор и запуск программ для Termux.\n\n",
            "Использование:\n",
            "  artef [путь]\n\n",
            "Параметры:\n",
            "  -h, --help   показать справку\n",
            "  -V, --version показать версию\n\n",
            "Путь может быть каталогом проекта или файлом для открытия.\n",
            "Конфиг: ~/.config/artef/config.toml\n",
            "Лог:    ~/.local/state/artef/artef.log\n\n",
            "Клавиши:\n",
            "  Ctrl+N файл · Ctrl+Shift+N папка · Ctrl+W рабочая папка\n",
            "  Ctrl+S сохранить · Ctrl+R запустить · Ctrl+F поиск · Ctrl+G строка\n",
            "  Tab/Ctrl+O фокус панели вывода · Ctrl+Shift+Q или F10 выход"
        ),
        env!("CARGO_PKG_VERSION")
    );
}

fn resolve_start(path: Option<PathBuf>) -> Result<(PathBuf, Option<PathBuf>), String> {
    let current = env::current_dir()
        .map_err(|error| format!("не удалось определить текущий каталог: {error}"))?;
    let requested = match path {
        Some(path) => expand_home(path),
        None => current.clone(),
    };
    let absolute = if requested.is_absolute() {
        requested
    } else {
        current.join(requested)
    };
    let canonical = fs::canonicalize(&absolute)
        .map_err(|error| format!("путь {} недоступен: {error}", absolute.display()))?;
    let metadata = fs::metadata(&canonical)
        .map_err(|error| format!("не удалось прочитать {}: {error}", canonical.display()))?;
    if metadata.is_dir() {
        Ok((canonical, None))
    } else if metadata.is_file() {
        let parent = canonical
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| format!("у файла {} нет родительского каталога", canonical.display()))?;
        Ok((parent, Some(canonical)))
    } else {
        Err(format!(
            "{} не является файлом или каталогом",
            canonical.display()
        ))
    }
}

fn expand_home(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if text == "~" {
        return platform::home_dir();
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return platform::home_dir().join(rest);
    }
    path
}

fn install_panic_hook() {
    let original = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, DisableBracketedPaste, Show);
        runner::kill_registered_child();
        logging::error(&format!("паника: {info}"));
        original(info);
    }));
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, String> {
    enable_raw_mode().map_err(|error| format!("не удалось включить raw mode: {error}"))?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide, EnableBracketedPaste) {
        let _ = disable_raw_mode();
        return Err(format!("не удалось открыть alternate screen: {error}"));
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        Terminal::new(backend).map_err(|error| format!("не удалось создать терминал: {error}"))?;
    terminal
        .clear()
        .map_err(|error| format!("не удалось очистить терминал: {error}"))?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste,
        Show
    );
    let _ = terminal.show_cursor();
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<(), String> {
    loop {
        app.tick();
        app.sync_highlights();
        terminal
            .draw(|frame| ui::draw(frame, app))
            .map_err(|error| format!("ошибка отрисовки: {error}"))?;
        if app.should_quit {
            break;
        }
        if event::poll(Duration::from_millis(40))
            .map_err(|error| format!("ошибка ожидания ввода: {error}"))?
        {
            match event::read().map_err(|error| format!("ошибка чтения ввода: {error}"))?
            {
                Event::Key(key) => app.handle_key(key),
                Event::Paste(text) => app.handle_paste(&text),
                Event::Resize(width, height) => {
                    app.runner.set_winsize(width, height.saturating_sub(2));
                }
                _ => {}
            }
        }
    }
    Ok(())
}
