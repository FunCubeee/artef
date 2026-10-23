use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::logging;

const HIDDEN_NAMES: [&str; 7] = [
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "dist",
    "build",
];

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub expanded: bool,
    pub ancestors_more: Vec<bool>,
    pub is_last: bool,
}

impl TreeEntry {
    pub fn depth(&self) -> usize {
        self.ancestors_more.len()
    }
}

pub struct FileTree {
    pub root: PathBuf,
    pub entries: Vec<TreeEntry>,
    pub selected: usize,
    pub scroll: usize,
    expanded: HashSet<PathBuf>,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let canonical = fs::canonicalize(&root).unwrap_or(root);
        let mut tree = Self {
            root: canonical,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            expanded: HashSet::new(),
        };
        tree.rebuild();
        tree
    }

    pub fn selected_entry(&self) -> Option<&TreeEntry> {
        self.entries.get(self.selected)
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.selected_entry().map(|entry| entry.path.as_path())
    }

    pub fn move_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = self.entries.len().saturating_sub(1);
        }
    }

    pub fn move_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.entries.len();
    }

    pub fn expand_selected(&mut self) -> bool {
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        if !entry.is_dir {
            return false;
        }
        if !entry.expanded {
            let path = entry.path.clone();
            self.expanded.insert(path);
            self.rebuild();
        }
        true
    }

    pub fn collapse_or_parent(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        if entry.is_dir && entry.expanded {
            let path = entry.path.clone();
            self.expanded.remove(&path);
            self.rebuild();
            return;
        }
        let Some(parent) = entry.path.parent().map(Path::to_path_buf) else {
            return;
        };
        if parent == self.root {
            return;
        }
        if let Some(index) = self.entries.iter().position(|item| item.path == parent) {
            self.selected = index;
        }
    }

    pub fn toggle_selected(&mut self) -> bool {
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        if !entry.is_dir {
            return false;
        }
        let path = entry.path.clone();
        if self.expanded.contains(&path) {
            self.expanded.remove(&path);
        } else {
            self.expanded.insert(path);
        }
        self.rebuild();
        true
    }

    pub fn refresh(&mut self) {
        let selected = self.selected_path().map(Path::to_path_buf);
        let previous_index = self.selected;
        self.rebuild();
        if let Some(path) = selected {
            if let Some(index) = self.entries.iter().position(|entry| entry.path == path) {
                self.selected = index;
            } else {
                self.selected = previous_index.min(self.entries.len().saturating_sub(1));
            }
        }
    }

    pub fn refresh_reveal(&mut self, path: &Path) {
        let mut current = path.parent();
        while let Some(parent) = current {
            if parent == self.root || !parent.starts_with(&self.root) {
                break;
            }
            self.expanded.insert(parent.to_path_buf());
            current = parent.parent();
        }
        self.refresh();
        if let Some(index) = self.entries.iter().position(|entry| entry.path == path) {
            self.selected = index;
        }
    }

    pub fn reveal_file(&mut self, path: &Path) {
        self.refresh_reveal(path);
    }

    pub fn current_directory(&self) -> PathBuf {
        match self.selected_entry() {
            Some(entry) if entry.is_dir => entry.path.clone(),
            Some(entry) => entry
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.root.clone()),
            None => self.root.clone(),
        }
    }

    pub fn relative_display(&self, path: &Path) -> String {
        match path.strip_prefix(&self.root) {
            Ok(relative) if relative.as_os_str().is_empty() => self.root.display().to_string(),
            Ok(relative) => relative.display().to_string(),
            Err(_) => path.display().to_string(),
        }
    }

    pub fn ensure_visible(&mut self, height: usize) {
        if height == 0 || self.entries.is_empty() {
            self.scroll = 0;
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
        let max_scroll = self.entries.len().saturating_sub(height);
        self.scroll = self.scroll.min(max_scroll);
    }

    fn rebuild(&mut self) {
        let mut entries = Vec::new();
        let mut ancestors = Vec::new();
        self.scan_directory(&self.root.clone(), &mut ancestors, &mut entries);
        self.entries = entries;
        if self.entries.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.entries.len() - 1);
        }
        self.scroll = 0;
    }

    fn scan_directory(
        &self,
        directory: &Path,
        ancestors_more: &mut Vec<bool>,
        output: &mut Vec<TreeEntry>,
    ) {
        let read = match fs::read_dir(directory) {
            Ok(read) => read,
            Err(error) => {
                logging::error(&format!(
                    "не удалось прочитать каталог {}: {error}",
                    directory.display()
                ));
                return;
            }
        };
        let mut children = Vec::new();
        for item in read {
            let Ok(item) = item else {
                continue;
            };
            let name = item.file_name().to_string_lossy().to_string();
            if HIDDEN_NAMES.contains(&name.as_str()) {
                continue;
            }
            let path = item.path();
            let file_type = match item.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    logging::error(&format!(
                        "не удалось определить тип {}: {error}",
                        path.display()
                    ));
                    continue;
                }
            };
            let is_dir = file_type.is_dir();
            children.push((name, path, is_dir));
        }
        children.sort_by(|left, right| {
            let left_key = (!left.2, left.0.to_ascii_lowercase());
            let right_key = (!right.2, right.0.to_ascii_lowercase());
            left_key.cmp(&right_key)
        });

        let count = children.len();
        for (index, (name, path, is_dir)) in children.into_iter().enumerate() {
            let is_last = index + 1 == count;
            let expanded = is_dir && self.expanded.contains(&path);
            output.push(TreeEntry {
                path: path.clone(),
                name,
                is_dir,
                expanded,
                ancestors_more: ancestors_more.clone(),
                is_last,
            });
            if expanded {
                ancestors_more.push(!is_last);
                self.scan_directory(&path, ancestors_more, output);
                ancestors_more.pop();
            }
        }
    }
}
