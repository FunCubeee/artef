use std::collections::BTreeMap;
use std::fs;

use serde::Deserialize;

use crate::language::Language;
use crate::logging;
use crate::platform;

#[derive(Debug, Deserialize)]
struct DictionaryFile {
    #[serde(default)]
    prefixes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    prefixes: BTreeMap<String, String>,
}

impl Dictionary {
    pub fn load(language: Language) -> Self {
        let Some(name) = language.dictionary_name() else {
            return Self::default();
        };
        let external_path = platform::dict_dir().join(format!("{name}.json"));
        if external_path.exists() {
            match fs::read_to_string(&external_path) {
                Ok(text) => match serde_json::from_str::<DictionaryFile>(&text) {
                    Ok(file) => {
                        logging::info(&format!(
                            "загружен словарь {} из {}",
                            name,
                            external_path.display()
                        ));
                        return Self {
                            prefixes: clean_prefixes(file.prefixes),
                        };
                    }
                    Err(error) => {
                        logging::error(&format!(
                            "ошибка словаря {}: {error}; используется встроенный",
                            external_path.display()
                        ));
                    }
                },
                Err(error) => {
                    logging::error(&format!(
                        "не удалось прочитать словарь {}: {error}",
                        external_path.display()
                    ));
                }
            }
        }
        let embedded = embedded_dictionary(name);
        match serde_json::from_str::<DictionaryFile>(embedded) {
            Ok(file) => Self {
                prefixes: clean_prefixes(file.prefixes),
            },
            Err(error) => {
                logging::error(&format!("встроенный словарь {name} повреждён: {error}"));
                Self::default()
            }
        }
    }

    pub fn suggestion(&self, prefix: &str) -> Option<&str> {
        self.prefixes
            .get(prefix)
            .map(String::as_str)
            .filter(|value| !value.is_empty() && *value != prefix)
    }
}

fn clean_prefixes(input: BTreeMap<String, String>) -> BTreeMap<String, String> {
    input
        .into_iter()
        .filter_map(|(prefix, completion)| {
            let completion = completion.split_whitespace().collect::<Vec<_>>().join(" ");
            if prefix.is_empty() || completion.is_empty() {
                None
            } else {
                Some((prefix, completion))
            }
        })
        .collect()
}

fn embedded_dictionary(name: &str) -> &'static str {
    match name {
        "python" => include_str!("../dicts/python.json"),
        "javascript" => include_str!("../dicts/javascript.json"),
        "typescript" => include_str!("../dicts/typescript.json"),
        "rust" => include_str!("../dicts/rust.json"),
        "c" => include_str!("../dicts/c.json"),
        "cpp" => include_str!("../dicts/cpp.json"),
        "go" => include_str!("../dicts/go.json"),
        "json" => include_str!("../dicts/json.json"),
        "yaml" => include_str!("../dicts/yaml.json"),
        "markdown" => include_str!("../dicts/markdown.json"),
        "shell" => include_str!("../dicts/shell.json"),
        "toml" => include_str!("../dicts/toml.json"),
        _ => "{\"prefixes\":{}}",
    }
}
