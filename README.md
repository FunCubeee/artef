# artef

Терминальная среда разработки для **Termux (Android, arm64)**: дерево файлов,
редактор кода с подсветкой и ghost-подсказками, панель вывода с эмуляцией
терминала и интерактивным вводом, запуск файлов по расширению.

Это единая среда без режимов: слева дерево, справа редактор, снизу вывод
запущенной программы. Пока программа работает, `Tab`/`Ctrl+O` переключает
фокус на панель вывода, и клавиатура напрямую идёт в `stdin` процесса через
PTY — работают `input()`, REPL, `readline`, `curses`, прогресс-бары.

## Возможности

- Дерево проекта с исключением `.git`, `node_modules`, `target`, `__pycache__`, `.venv`, `dist`, `build`.
- Редактор: номера строк, word wrap (кольцевой перенос по словам, `Alt+W`), выделение, поиск, переход к строке, undo/redo, внутренний буфер обмена.
- Bracketed paste: вставка 10 КБ multiline-текста не ломает TUI; вставка атомарна, `\r`→`\n`, управляющие символы вырезаются.
- Подсветка Python, JavaScript, TypeScript, Rust, C, C++, Go, JSON, YAML, Markdown, Shell и TOML.
- Ghost-подсказки из встроенных словарей; внешние — в `~/.config/artef/dicts`.
- Запуск через PTY с ANSI-цветами: `python3`, `node`, `tsx`, `cargo run`, `go run`, `gcc`, `g++`, `bash`; для файлов без расширения — shebang или диалог выбора рантайма.
- `Ctrl+C` в фокусе вывода → SIGINT процессу, не `artef`.
- `Esc`/`Ctrl+Q` в редакторе → возврат к дереву (выход только `Alt+Q`/`F10` с подтверждением).
- `Ctrl+W` — текущая папка дерева становится рабочей (там создаются файлы, относительно неё резолвится cwd процесса).
- Настраиваемая тема gruvbox, горячие клавиши, правила запуска (`~/.config/artef/config.toml`).
- Двухстрочный статус-бар, ASCII-символы на узких экранах (`ARTEF_ASCII=1` — принудительно).

## Сборка в Termux

```bash
pkg update
pkg install rust clang python nodejs golang make
git clone <репозиторий> artef || cd existing
cd artef
cargo build --release --target aarch64-linux-android
cp target/aarch64-linux-android/release/artef $PREFIX/bin/artef
chmod +x $PREFIX/bin/artef
```

Альтернатива без явного target (наиболее частый путь на Termux, хост сам
является aarch64-linux-android):

```bash
cargo build --release
cp target/release/artef $PREFIX/bin/artef
```

Рантаймы для запуска файлов берутся из `$PATH` Termux:
`pkg install python nodejs golang`; `clang`/`gcc` для C, `g++` для C++,
`cargo` уже установлен вместе с `rust`.

## Запуск

```bash
cd ~/project
artef            # дерево текущего каталога
artef ~/project  # проект
artef ~/project/main.py
artef --help
```

## Горячие клавиши (по умолчанию)

| Действие | Клавиша |
|---|---|
| Сохранить | `Ctrl+S` |
| Запустить текущий файл | `Ctrl+R` |
| Фокус: дерево/редактор | `Ctrl+B`, `Tab` |
| Фокус: панель вывода (во время запуска) | `Tab` / `Ctrl+O` |
| SIGINT процессу | `Ctrl+C` (в фокусе вывода) |
| Прокрутка вывода | `PageUp`/`PageDown` |
| Новый файл (имя, расширение опционально) | `Ctrl+N` / `F6` |
| Новая папка | `Alt+N` / `F7` |
| Удалить файл/папку (в дереве) | `D` / `Delete`, с подтверждением |
| Сделать папку рабочей | `Ctrl+W` |
| Найти | `Ctrl+F`, далее `F3` |
| Перейти к строке | `Ctrl+G` |
| Копировать/Вставить | `Ctrl+C` / `Ctrl+V` |
| Отмена/Повтор | `Ctrl+Z` / `Ctrl+Y` |
| Word wrap | `Alt+W` |
| Возврат к дереву из редактора | `Esc` / `Ctrl+Q` |
| Выход из artef целиком | `Alt+Q` / `F10` (с подтверждением) |

## Конфиг и логи

- Конфиг: `~/.config/artef/config.toml` (тема, шорткаты через `shortcuts.*`, `workdir` по умолчанию, правила запуска в `[run]` — `{file}`, `{python}`, `{node}`, `{tsx}`, `{go}`, `{cc}`, `{cxx}`, `{cargo}`, `{bash}`, `{temp}`).
- Лог: `~/.local/state/artef/artef.log`.

## Поведение на выходе

Завершение всегда завершает дочерний процесс: SIGTERM → до 600 мс ожидания →
SIGKILL, PTY закрывается, raw mode и bracketed paste восстанавливаются — то же
самое делает panic-hook при аварийном завершении.
