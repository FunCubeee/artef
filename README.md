# artef

Терминальный редактор и запуск программ для Termux, Linux и Windows.

`artef` объединяет дерево файлов, редактор с подсветкой синтаксиса и ghost-подсказками, запуск кода с разбором ANSI-цветов и два режима работы:

- **Explorer** — навигация по проекту и запуск файлов.
- **IDE** — редактор, поиск, создание файлов и папок, запуск.

Переключение режимов — `F2`. Открытый файл, позиция курсора и раскрытые папки сохраняются.

## Возможности

- Дерево проекта с исключением `.git`, `node_modules`, `target`, `__pycache__`, `.venv`, `dist`, `build`.
- Редактор с номерами строк, выделением, поиском, переходом к строке, undo/redo и внутренним буфером обмена.
- Подсветка Python, JavaScript, TypeScript, Rust, C, C++, Go, JSON, YAML, Markdown, Shell и TOML.
- Ghost-подсказки из встроенных словарей; внешние словари можно положить в каталог конфига.
- Запуск Python, JavaScript, TypeScript, Rust, Go, C, C++ и Shell.
- Потоковый вывод stdout/stderr, ANSI-цвета, прокрутка `PageUp`/`PageDown`.
- Предупреждение при запуске GUI/интерактивных программ и запуск во внешнем терминале.
- Настраиваемая тема gruvbox, горячие клавиши и правила запуска.
- Двухстрочный статус-бар в Termux и однострочный в Linux/Windows.
- ASCII-символы для узких терминалов или при `ARTEF_ASCII=1`.

## Сборка

Нужен Rust 1.82 или новее.

### Termux

```bash
pkg update
pkg install rust clang make pkg-config
cargo build --release
./target/release/artef
```

Для запуска языков установите соответствующие пакеты, например `pkg install python nodejs golang`. Для C/C++ уже достаточно `clang`.

### Linux

Установите Rust через `rustup` и системный компилятор:

```bash
sudo apt install build-essential pkg-config
cargo build --release
./target/release/artef
```

Для полностью статической сборки Linux можно использовать target `x86_64-unknown-linux-musl`.

### Windows

Установите Rust с toolchain `stable-x86_64-pc-windows-msvc` и Visual Studio Build Tools с компонентом Desktop development with C++:

```powershell
cargo build --release
.\target\release\artef.exe
```

Бинарник один; на Android он зависит от системной Bionic libc, на Windows — от системного runtime. Интерпретаторы Python/Node/Go и компиляторы C/C++/Rust нужны только для запуска соответствующих файлов.

## Запуск

```bash
artef
artef --mode ide .
artef --mode explorer ~/project
artef --mode ide ~/project/main.py
artef --help
```

Без `--mode` используется `default_mode` из конфига. При `default_mode = "ask"` показывается выбор режима и каталога проекта.

## Горячие клавиши

| Действие | Клавиша |
|---|---|
| Сохранить | `Ctrl+S` |
| Запустить | `Ctrl+R` |
| Новый файл | `Ctrl+N` |
| Новая папка | `F7` (`Ctrl+Shift+N`, если терминал передаёт Shift) |
| Поиск | `Ctrl+F` |
| Следующее совпадение | `F3` |
| Перейти к строке | `Ctrl+G` |
| Копировать | `Ctrl+C` |
| Вставить | `Ctrl+V` |
| Отменить / повторить | `Ctrl+Z` / `Ctrl+Y` |
| Сменить режим | `F2` |
| Фокус дерево/редактор | `Ctrl+B` |
| Обновить дерево | `F5` |
| Принять ghost-подсказку / таб | `Tab` |
| Прокрутка вывода | `PageUp` / `PageDown` |
| Выход | `Ctrl+Q` |

При выходе текущий изменённый файл сохраняется. Если сохранить не удалось, выход отменяется.

## Конфигурация

Конфиг создаётся автоматически:

- Termux/Linux: `~/.config/artef/config.toml`
- Windows: `%APPDATA%\artef\config.toml`

Лог:

- Termux/Linux: `~/.local/state/artef/artef.log`
- Windows: `%LOCALAPPDATA%\artef\state\artef.log`

Основные секции:

```toml
default_mode = "ask"

[theme]
accent = "#b8bb26"
background = "#1d2021"
panel = "#282828"

[shortcuts]
save = "ctrl+s"
run = "ctrl+r"
new_file = "ctrl+n"
new_folder = "f7"
new_folder_alt = "ctrl+shift+n"
toggle_mode = "f2"

[run]
".py" = "{python} -u {file}"
".js" = "{node} {file}"
".ts" = "{tsx} {file}"
".rs" = "{cargo} run --release"
".go" = "{go} run {file}"
".c" = "{cc} {file} -o {temp} && {temp}"
".cpp" = "{cxx} {file} -o {temp} && {temp}"
".sh" = "{bash} {file}"
```

В правилах запуска доступны подстановки:

- `{file}` — путь к файлу
- `{dir}` — каталог файла
- `{name}` — имя файла
- `{stem}` — имя без расширения
- `{ext}` — расширение
- `{temp}` — временный исполняемый файл
- `{python}`, `{node}`, `{tsx}`, `{bash}`, `{cargo}`, `{go}`, `{cc}`, `{cxx}`, `{rustc}` — поиск программы в `PATH`

Для Rust в проекте с `Cargo.toml` используется `cargo run --release`; одиночный `.rs` файл собирается через `rustc`.

## Ghost-словари

Встроенные словари находятся в `dicts/*.json` и включаются в бинарник. Чтобы переопределить словарь без пересборки, создайте файл:

- Termux/Linux: `~/.config/artef/dicts/python.json`
- Windows: `%APPDATA%\artef\dicts\python.json`

Формат:

```json
{
  "prefixes": {
    "pr": "int(...)",
    "def": " function name():"
  }
}
```

## Проверка

```bash
cargo fmt --check
cargo check
cargo test
cargo build --release
```

## Структура

- `src/main.rs` — CLI, терминал, цикл событий.
- `src/app.rs` — состояние, режимы, диалоги, горячие клавиши.
- `src/ui.rs` — отрисовка дерева, редактора, вывода, статуса и модальных окон.
- `src/editor.rs` — редактирование, выделение, поиск, undo/redo.
- `src/highlight.rs` — лексеры и подсветка.
- `src/runner.rs` — правила запуска, процессы, ANSI-парсер и вывод.
- `src/file_tree.rs` — дерево проекта.
- `src/config.rs`, `src/theme.rs`, `src/dictionary.rs` — конфиг, тема, словари.
- `src/platform.rs`, `src/logging.rs` — платформенные пути, запуск терминала, лог.
