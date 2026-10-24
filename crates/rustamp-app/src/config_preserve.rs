// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Keeping the settings the player does not own.
//!
//! A save writes the settings the player honours and **keeps every other
//! setting the file already had**. The rule is the point of this module: a
//! setting the player did not apply must never be destroyed by a save that
//! did not apply it either. A user who writes `playlist.shuffle = true` into
//! `rustamp.toml` finds that line still there after the next clean exit,
//! together with the warning that the setting is not applied yet.
//!
//! The merge works on the **source text**, not on the parsed values: the
//! previous file is scanned for its key entries, every entry the player does
//! not own is copied with the text it was written with, and that text is
//! spliced back into the table it came from. A value-level merge would
//! rewrite the spelling (an array becomes a multi-line array) and would drop
//! the comment that stands on the line; the source-level merge keeps both.
//!
//! The scan is a small TOML **reader**, not a parser: it recognises the
//! shape of a document (tables, keys, values that continue on the next
//! line) and copies text. Whether the file is valid TOML is [`toml`]'s
//! business, and the app parses it anyway.
//!
//! A comment that documents a setting travels with it: the comment lines
//! above a key belong to that key, so the user's notes are kept together
//! with the setting they explain.

/// One part of a key: how it was written and what it names.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeyPart {
    /// The part's source text (a quoted key keeps its quotes).
    source: String,
    /// The name the part carries.
    name: String,
}

/// A key entry of a TOML document.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// The index of the section the key was written in.
    section: usize,
    /// The key's parts (a dotted key has several).
    key: Vec<KeyPart>,
    /// The entry's source lines: the comments that stand above the setting,
    /// then the key line, then the lines the value continues on.
    lines: Vec<String>,
    /// The index of the key line in [`Entry::lines`].
    key_line: usize,
    /// The byte range of the key in the key line.
    span: (usize, usize),
}

/// A section of a TOML document: a table and the lines that belong to it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    /// The table's path; empty for the root table.
    path: Vec<String>,
    /// The header line, exactly as it was written (empty for the root table).
    header: String,
    /// The lines that belong to the section.
    lines: Vec<String>,
    /// Whether the section is an array of tables (`[[name]]`).
    array: bool,
}

/// A scanned TOML document.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Scan {
    /// The document's sections, the root table first.
    sections: Vec<Section>,
    /// The document's key entries.
    entries: Vec<Entry>,
}

impl Entry {
    /// The logical path of the key: the table's path plus the key's parts.
    fn path(&self, sections: &[Section]) -> Vec<String> {
        let mut path = sections[self.section].path.clone();
        path.extend(self.key.iter().map(|part| part.name.clone()));
        path
    }
}

/// Merges the player's own settings with the settings of a previous file
/// that the player does not own.
///
/// `honest` is the player's own serialization (the settings it applies),
/// `previous` the file the settings came from. `owned` decides whether the
/// player owns a setting: an owned setting is written by the player, and the
/// previous file's value for it is dropped. Every other setting is copied
/// with the text it was written with.
///
/// The result is a TOML document: the player's own settings, plus the
/// settings the player does not own, unchanged.
pub(crate) fn merge(honest: &str, previous: &str, owned: &dyn Fn(&[String]) -> bool) -> String {
    let previous_scan = scan(previous);
    let mut document = Document {
        sections: scan(honest).sections,
        line_ending: line_ending(previous),
    };

    for entry in &previous_scan.entries {
        if previous_scan.sections[entry.section].array {
            // An array of tables is kept as a whole, below.
            continue;
        }
        if owned(&entry.path(&previous_scan.sections)) {
            continue;
        }
        let (table, lines) = relocate(entry, &previous_scan, &document);
        document.insert(&table, lines);
    }

    // An array of tables is a value of its own: its entries belong to one
    // element, so the whole section is copied when every one of its settings
    // belongs to the user.
    for (index, section) in previous_scan.sections.iter().enumerate() {
        if !section.array {
            continue;
        }
        let foreign = previous_scan
            .entries
            .iter()
            .filter(|entry| entry.section == index)
            .all(|entry| !owned(&entry.path(&previous_scan.sections)));
        if foreign {
            document.insert_array(section.clone());
        }
    }

    document.text()
}

/// The table a setting belongs in, and the text that carries it.
///
/// A dotted key names a table: `audio.buffer_ms` in the root table defines
/// `audio.buffer_ms`. When the player's own file defines that table as well
/// (it writes `[audio]`), the two would collide — TOML forbids defining the
/// same table twice. The setting therefore moves into the table the player's
/// file defines, and its key is rewritten without the table's name. The
/// setting itself is unchanged.
fn relocate(entry: &Entry, previous: &Scan, document: &Document) -> (Vec<String>, Vec<String>) {
    let section = &previous.sections[entry.section].path;
    if entry.key.len() < 2 {
        return (section.clone(), entry.lines.clone());
    }
    let mut table = section.clone();
    table.push(entry.key[0].name.clone());
    if !document.has_table(&table) {
        return (section.clone(), entry.lines.clone());
    }
    let key = entry.key[1..]
        .iter()
        .map(|part| part.source.as_str())
        .collect::<Vec<_>>()
        .join(".");
    let mut lines = entry.lines.clone();
    lines[entry.key_line].replace_range(entry.span.0..entry.span.1, &key);
    (table, lines)
}

/// The document the merged settings are written into.
struct Document {
    /// The document's sections, the root table first.
    sections: Vec<Section>,
    /// The line ending the document uses.
    line_ending: String,
}

impl Document {
    /// Whether the document defines the table at `path`.
    fn has_table(&self, path: &[String]) -> bool {
        self.sections
            .iter()
            .any(|section| !section.array && section.path == path)
    }

    /// Adds a setting to the table at `path`, creating the table when the
    /// document does not have it.
    fn insert(&mut self, path: &[String], lines: Vec<String>) {
        if let Some(index) = self
            .sections
            .iter()
            .position(|section| !section.array && section.path == path)
        {
            let section = &mut self.sections[index];
            // The setting goes before the table's trailing blank lines, so
            // the table keeps its shape.
            let mut at = section.lines.len();
            while at > 0 && section.lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            for (offset, line) in lines.into_iter().enumerate() {
                section.lines.insert(at + offset, line);
            }
            return;
        }
        // A table the player's file does not have: it is added before the
        // tables that live inside it, because TOML wants a table before its
        // sub-tables (`[eq]` before `[eq.state]`).
        let section = Section {
            path: path.to_vec(),
            header: render_header(path),
            lines,
            array: false,
        };
        let at = self
            .sections
            .iter()
            .position(|other| other.path.starts_with(path))
            .unwrap_or(self.sections.len());
        self.sections.insert(at, section);
    }

    /// Adds an array of tables at the end of the document.
    fn insert_array(&mut self, section: Section) {
        self.sections.push(section);
    }

    /// The document's text.
    fn text(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            if !section.header.is_empty() {
                out.push_str(&section.header);
                out.push_str(&self.line_ending);
            }
            for line in &section.lines {
                out.push_str(line);
                out.push_str(&self.line_ending);
            }
        }
        out
    }
}

/// The line ending a document uses.
fn line_ending(text: &str) -> String {
    if text.contains("\r\n") {
        "\r\n".to_string()
    } else {
        "\n".to_string()
    }
}

/// The header line of a table.
fn render_header(path: &[String]) -> String {
    let name = path
        .iter()
        .map(|part| render_key_part(part))
        .collect::<Vec<_>>()
        .join(".");
    format!("[{name}]")
}

/// A table name as it is written in a header.
fn render_key_part(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        return name.to_string();
    }
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Scans a TOML document into its sections and its key entries.
fn scan(text: &str) -> Scan {
    let mut sections = vec![Section {
        path: Vec::new(),
        header: String::new(),
        lines: Vec::new(),
        array: false,
    }];
    let mut entries: Vec<Entry> = Vec::new();
    // An entry whose value continues on the next line.
    let mut pending: Option<(Entry, ValueScan)> = None;
    // The comments that stand above the next setting.
    let mut comments: Vec<String> = Vec::new();

    for line in text.lines() {
        // A value that continues swallows this line: it belongs to the
        // entry, not to the table.
        let continues = match pending.as_mut() {
            Some((entry, value)) => {
                entry.lines.push(line.to_string());
                sections
                    .last_mut()
                    .expect("the document has a current table")
                    .lines
                    .push(line.to_string());
                value.feed(line)
            }
            None => false,
        };
        if pending.is_some() {
            if !continues {
                let (entry, _) = pending.take().expect("the entry is pending");
                entries.push(entry);
            }
            continue;
        }

        // A header starts a new table.
        if let Some((path, array)) = parse_header(line) {
            sections.push(Section {
                path,
                header: line.trim().to_string(),
                lines: Vec::new(),
                array,
            });
            continue;
        }

        // Otherwise the line belongs to the current table.
        let index = sections.len() - 1;
        let array = sections[index].array;
        // A comment on a line of its own stands with the setting below it:
        // it travels with that setting, so the user's notes survive a save
        // together with the setting they explain.
        if line.trim_start().starts_with('#') && !array {
            comments.push(line.to_string());
            continue;
        }
        sections
            .last_mut()
            .expect("the document has a current table")
            .lines
            .push(line.to_string());
        let Some(key_line) = parse_key_line(line) else {
            continue;
        };
        let mut value = ValueScan::new();
        let continues = value.feed(&key_line.value);
        let mut lines = comments;
        let key_line_index = lines.len();
        lines.push(line.to_string());
        comments = Vec::new();
        let entry = Entry {
            section: index,
            key: key_line.parts,
            lines,
            key_line: key_line_index,
            span: key_line.span,
        };
        if continues {
            pending = Some((entry, value));
        } else {
            entries.push(entry);
        }
    }

    // A file that ends inside a value keeps the entry it has.
    if let Some((entry, _)) = pending {
        entries.push(entry);
    }

    Scan { sections, entries }
}

/// The key of a `key = value` line.
struct KeyLine {
    /// The key's parts.
    parts: Vec<KeyPart>,
    /// The byte range of the key in the line.
    span: (usize, usize),
    /// The text of the value.
    value: String,
}

/// The table path of a header line, if the line is a header.
fn parse_header(line: &str) -> Option<(Vec<String>, bool)> {
    let trimmed = line.trim();
    let (inner, array) = if let Some(rest) = trimmed.strip_prefix("[[") {
        (rest.strip_suffix("]]")?, true)
    } else {
        let rest = trimmed.strip_prefix('[')?;
        (rest.strip_suffix(']')?, false)
    };
    Some((split_dotted(inner)?, array))
}

/// The parts of a dotted name.
fn split_dotted(text: &str) -> Option<Vec<String>> {
    let bytes = text.as_bytes();
    let mut index = skip_space(bytes, 0);
    let mut parts = Vec::new();
    loop {
        if index >= bytes.len() {
            return None;
        }
        parts.push(parse_key_part(bytes, &mut index)?.name);
        let next = skip_space(bytes, index);
        if next < bytes.len() && bytes[next] == b'.' {
            index = skip_space(bytes, next + 1);
            continue;
        }
        if next != bytes.len() {
            return None;
        }
        break;
    }
    Some(parts)
}

/// The key of a `key = value` line, if the line carries one.
fn parse_key_line(line: &str) -> Option<KeyLine> {
    let bytes = line.as_bytes();
    let mut index = skip_space(bytes, 0);
    let start = index;
    let mut parts = Vec::new();
    let mut end;
    loop {
        let part = parse_key_part(bytes, &mut index)?;
        parts.push(part);
        end = index;
        let next = skip_space(bytes, index);
        if next < bytes.len() && bytes[next] == b'.' {
            index = skip_space(bytes, next + 1);
            continue;
        }
        index = next;
        break;
    }
    if index >= bytes.len() || bytes[index] != b'=' {
        return None;
    }
    let value = line[(index + 1).min(line.len())..].to_string();
    Some(KeyLine {
        parts,
        span: (start, end),
        value,
    })
}

/// One part of a key, read from `bytes` at `index`.
fn parse_key_part(bytes: &[u8], index: &mut usize) -> Option<KeyPart> {
    if *index >= bytes.len() {
        return None;
    }
    match bytes[*index] {
        quote @ (b'"' | b'\'') => {
            let start = *index;
            let mut cursor = start + 1;
            while cursor < bytes.len() {
                if quote == b'"' && bytes[cursor] == b'\\' {
                    cursor += 2;
                    continue;
                }
                if bytes[cursor] == quote {
                    break;
                }
                cursor += 1;
            }
            if cursor >= bytes.len() {
                return None;
            }
            let inner = &bytes[start + 1..cursor];
            let source = String::from_utf8_lossy(&bytes[start..=cursor]).into_owned();
            let name = if quote == b'"' {
                unescape(inner)
            } else {
                String::from_utf8_lossy(inner).into_owned()
            };
            *index = cursor + 1;
            Some(KeyPart { source, name })
        }
        _ => {
            let start = *index;
            while *index < bytes.len()
                && (bytes[*index].is_ascii_alphanumeric()
                    || bytes[*index] == b'_'
                    || bytes[*index] == b'-')
            {
                *index += 1;
            }
            if *index == start {
                return None;
            }
            let source = String::from_utf8_lossy(&bytes[start..*index]).into_owned();
            Some(KeyPart {
                name: source.clone(),
                source,
            })
        }
    }
}

/// The text of a basic string without its escapes.
fn unescape(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' || index + 1 >= bytes.len() {
            out.push(bytes[index] as char);
            index += 1;
            continue;
        }
        let escaped = bytes[index + 1];
        index += 2;
        match escaped {
            b'n' => out.push('\n'),
            b't' => out.push('\t'),
            b'r' => out.push('\r'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            _ => {
                out.push('\\');
                out.push(escaped as char);
            }
        }
    }
    out
}

/// The first byte at or after `index` that is not a space or a tab.
fn skip_space(bytes: &[u8], index: usize) -> usize {
    let mut cursor = index;
    while cursor < bytes.len() && (bytes[cursor] == b' ' || bytes[cursor] == b'\t') {
        cursor += 1;
    }
    cursor
}

/// The state of a value that may continue on the next line.
struct ValueScan {
    /// The number of open brackets.
    depth: usize,
    /// The string the value is in.
    state: ValueState,
}

/// The string a value is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueState {
    /// Outside a string.
    Normal,
    /// Inside a `"..."` string.
    Basic,
    /// Inside a `'...'` string.
    Literal,
    /// Inside a `"""..."""` string.
    MultiBasic,
    /// Inside a `'''...'''` string.
    MultiLiteral,
}

impl ValueScan {
    /// A value that starts at the beginning of a line.
    fn new() -> ValueScan {
        ValueScan {
            depth: 0,
            state: ValueState::Normal,
        }
    }

    /// Feeds one line of value text; returns whether the value continues on
    /// the next line.
    fn feed(&mut self, text: &str) -> bool {
        let bytes = text.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            match self.state {
                ValueState::Normal => match byte {
                    b'"' => {
                        if bytes[index..].starts_with(b"\"\"\"") {
                            self.state = ValueState::MultiBasic;
                            index += 3;
                        } else {
                            self.state = ValueState::Basic;
                            index += 1;
                        }
                    }
                    b'\'' => {
                        if bytes[index..].starts_with(b"'''") {
                            self.state = ValueState::MultiLiteral;
                            index += 3;
                        } else {
                            self.state = ValueState::Literal;
                            index += 1;
                        }
                    }
                    b'[' | b'{' => {
                        self.depth += 1;
                        index += 1;
                    }
                    b']' | b'}' => {
                        self.depth = self.depth.saturating_sub(1);
                        index += 1;
                    }
                    // The rest of the line is a comment.
                    b'#' => break,
                    _ => index += 1,
                },
                ValueState::Basic => {
                    if byte == b'\\' {
                        index += 2;
                    } else {
                        if byte == b'"' {
                            self.state = ValueState::Normal;
                        }
                        index += 1;
                    }
                }
                ValueState::Literal => {
                    if byte == b'\'' {
                        self.state = ValueState::Normal;
                    }
                    index += 1;
                }
                ValueState::MultiBasic => {
                    if byte == b'\\' {
                        index += 2;
                    } else if bytes[index..].starts_with(b"\"\"\"") {
                        self.state = ValueState::Normal;
                        index += 3;
                    } else {
                        index += 1;
                    }
                }
                ValueState::MultiLiteral => {
                    if bytes[index..].starts_with(b"'''") {
                        self.state = ValueState::Normal;
                        index += 3;
                    } else {
                        index += 1;
                    }
                }
            }
        }
        // A single-line string that was not closed cannot continue: the file
        // is malformed, and the parser says so.
        if matches!(self.state, ValueState::Basic | ValueState::Literal) {
            self.state = ValueState::Normal;
        }
        self.depth > 0
            || matches!(
                self.state,
                ValueState::MultiBasic | ValueState::MultiLiteral
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The settings of a text, with their path and their text.
    fn settings(text: &str) -> Vec<(Vec<String>, String)> {
        let scan = scan(text);
        scan.entries
            .iter()
            .filter(|entry| !scan.sections[entry.section].array)
            .map(|entry| (entry.path(&scan.sections), entry.lines.join("\n")))
            .collect()
    }

    #[test]
    fn a_key_is_read_with_its_path_and_its_text() {
        let found = settings("[audio]\nbuffer_ms = 300\n");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].0,
            vec!["audio".to_string(), "buffer_ms".to_string()]
        );
        assert_eq!(found[0].1, "buffer_ms = 300");
    }

    #[test]
    fn a_value_that_continues_on_the_next_line_is_one_entry() {
        let found = settings("[plugins]\ndisabled = [\n    \"a.dll\",\n    \"b.dll\",\n]\n");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].1,
            "disabled = [\n    \"a.dll\",\n    \"b.dll\",\n]"
        );
    }

    #[test]
    fn a_comment_on_the_line_belongs_to_the_entry() {
        let found = settings("[playlist]\nshuffle = true # not yet\n");
        assert_eq!(found[0].1, "shuffle = true # not yet");
    }

    #[test]
    fn a_dotted_key_carries_its_parts() {
        let found = settings("audio.buffer_ms = 300\n");
        assert_eq!(
            found[0].0,
            vec!["audio".to_string(), "buffer_ms".to_string()]
        );
    }

    #[test]
    fn a_quoted_key_carries_its_name() {
        let found = settings("\"my key\" = 1\n");
        assert_eq!(found[0].0, vec!["my key".to_string()]);
    }

    #[test]
    fn a_comment_above_a_setting_stands_with_it() {
        let found = settings("# a note\nshuffle = true\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, "# a note\nshuffle = true");
    }

    #[test]
    fn a_header_inside_a_value_is_not_a_header() {
        let found = settings("x = [\n[1],\n]\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, vec!["x".to_string()]);
    }

    #[test]
    fn the_settings_of_a_previous_file_are_kept_verbatim() {
        let previous = "\
version = 1

[audio]
volume = 200
buffer_ms = 300 # not applied

[playlist]
shuffle = true
";
        let merged = merge(
            "version = 1\n\n[audio]\nvolume = 200\n",
            previous,
            &|path| {
                path == ["version".to_string()]
                    || path == ["audio".to_string(), "volume".to_string()]
            },
        );
        assert!(merged.contains("buffer_ms = 300 # not applied"), "{merged}");
        assert!(merged.contains("shuffle = true"), "{merged}");
        assert!(merged.contains("volume = 200"), "{merged}");
    }

    #[test]
    fn a_dotted_key_moves_into_the_table_the_player_defines() {
        let previous = "audio.buffer_ms = 300\n";
        let merged = merge("version = 1\n\n[audio]\nvolume = 200\n", previous, &|_| {
            false
        });
        assert!(merged.contains("[audio]"), "{merged}");
        assert!(merged.contains("buffer_ms = 300"), "{merged}");
        assert!(!merged.contains("audio.buffer_ms"), "{merged}");
    }

    #[test]
    fn an_array_of_tables_is_kept_as_a_whole() {
        let previous = "[[vis]]\nname = \"a\"\n\n[[vis]]\nname = \"b\"\n";
        let merged = merge("version = 1\n", previous, &|_| false);
        assert_eq!(merged.matches("[[vis]]").count(), 2, "{merged}");
    }
}
