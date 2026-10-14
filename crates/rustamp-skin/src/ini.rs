//! A minimal `pledit.txt`-style INI parser.
//!
//! Classic skins use a small INI dialect for `pledit.txt` and `region.txt`. The
//! rules implemented here (section headings in brackets, `key = value`, keys
//! lowercased, text after a second `=` dropped, one pair of surrounding quotes
//! stripped) follow the behaviour documented for those files. The exact
//! dialect is one of the things spec **S17** must confirm.

use std::collections::HashMap;

/// A parsed INI file: section name (lowercased) to key/value map (keys
/// lowercased).
pub type Ini = HashMap<String, HashMap<String, String>>;

/// Parses INI text.
///
/// Properties that appear before the first section are ignored, matching the
/// classic format. Values keep everything up to a second `=`.
pub fn parse_ini(text: &str) -> Ini {
    let mut data: Ini = HashMap::new();
    let mut section: Option<String> = None;

    for raw_line in text.split(['\r', '\n']) {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(name) = as_section(line) {
            section = Some(name.to_ascii_lowercase());
            data.entry(name.to_ascii_lowercase()).or_default();
            continue;
        }

        let Some(section_name) = &section else {
            continue;
        };

        let Some((key, value)) = split_property(line) else {
            continue;
        };
        data.entry(section_name.clone())
            .or_default()
            .insert(key.to_ascii_lowercase(), value);
    }

    data
}

/// Returns the section name when `line` is `[name]`.
fn as_section(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let name = inner.trim();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

/// Splits a `key = value` line, applying the value normalisation.
fn split_property(line: &str) -> Option<(String, String)> {
    // The key must start with a non-`;` character, so comments are skipped.
    if line.starts_with(';') {
        return None;
    }
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    // Drop anything after a second `=`, then trim, then strip one pair of
    // surrounding quotes.
    let value = rest.split('=').next().unwrap_or("").trim();
    let value = strip_quotes(value);
    Some((key.to_string(), value))
}

fn strip_quotes(value: &str) -> String {
    let mut v = value;
    if let Some(rest) = v.strip_prefix('"').or_else(|| v.strip_prefix('\'')) {
        v = rest;
    }
    if let Some(rest) = v.strip_suffix('"').or_else(|| v.strip_suffix('\'')) {
        v = rest;
    }
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_keys_case_insensitively() {
        let ini = parse_ini("[Text]\r\nNormal=#00FF00\r\nFont=Arial\r\n");
        let text = ini.get("text").unwrap();
        assert_eq!(text.get("normal").unwrap(), "#00FF00");
        assert_eq!(text.get("font").unwrap(), "Arial");
    }

    #[test]
    fn drops_second_equals_and_strips_quotes() {
        let ini = parse_ini("[a]\nx = \"he=llo\"\n");
        assert_eq!(ini["a"]["x"], "he");
    }

    #[test]
    fn ignores_comments_and_properties_before_a_section() {
        let ini = parse_ini("orphan=1\n; comment\n[a]\n; c=2\nb=3\n");
        assert!(!ini.contains_key("orphan"));
        assert_eq!(ini["a"]["b"], "3");
        assert!(!ini["a"].contains_key("c"));
    }

    #[test]
    fn empty_text_is_empty() {
        assert!(parse_ini("").is_empty());
    }
}
