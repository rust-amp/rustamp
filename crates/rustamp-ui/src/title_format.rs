//! The title-format language of spec **S14**, as a pure engine.
//!
//! The engine is a pure function of two inputs — a format string and the
//! field values a row carries — and its output is a string. It reads no
//! file, opens no window, keeps no clock and never mutates its inputs.
//!
//! **Provenance.** Everything here is written from
//! `specs/S14-title-formatting.md` alone; no reference binary, no Winamp
//! source, no SDK header and no disassembly was read.
//!
//! # What the spec settles
//!
//! * S14 §2.1 fixes the field syntax (`%name%`) and the nine names the
//!   material carries ([`FIELD_NAMES`]); any other tag name is accepted
//!   as well.
//! * S14 §2.2 fixes the twelve function names, `$abbr` in two forms.
//! * S14 §2.3 fixes the conditional group `[ … ]`.
//! * S14 §2.4 carries **four** format strings; they are this spec's own
//!   test vectors and the tests below render exactly those four.
//!
//! # Where the language belongs (S14 §1)
//!
//! The spec states plainly that in the reference this language belongs to
//! the **input plug-ins**, not to the player: the reference has no global
//! title-format setting, and the player's own strings carry no field or
//! function token. We implement the *engine* here and hang it on the
//! playlist row text behind a switch that is **off by default**
//! ([`TitleFormat::off`]), so nothing changes until a user turns it on.
//! That the row text is the place to hang it is **our own choice**: S14 §1
//! offers it as a reading, and open question 7 leaves the player's own
//! post-processing unsettled.
//!
//! # The open questions, and what we chose
//!
//! S14 §4 lists thirteen open questions, and S14 §3 forbids implementing a
//! behaviour §4 lists as if it were settled. Every choice below is
//! therefore marked as ours and names the question it answers.
//!
//! | Question | Our choice |
//! |---|---|
//! | 1 — the exact rule of `$abbr` | the initials of the words: the first ASCII letter of every whitespace-separated word, upper-cased |
//! | 2 — `$num` with a non-numeric field | the empty string: a value that is not a number has no numeric rendering |
//! | 3 — `$caps`/`$caps2` on non-ASCII text | ASCII-only case folding: `a`–`z` and `A`–`Z` change, every other letter stays as it is |
//! | 4 — the escaping of `[`, `]`, `$`, `%` | a backslash escapes the next character; a field *value* is never re-parsed, so a `]` inside a tag is safe |
//! | 5 — an unknown function name | inserted literally, as the call's own source text; the rest of the format is **not** dropped |
//! | 6 — the maximum format-string length | no length limit; the nesting depth is capped at [`MAX_NESTING_DEPTH`] so the parser cannot overflow the stack |
//! | 11 — what counts as a "valid field" | a field whose value is not empty |
//! | 13 — how `$longest` breaks a tie | the first of the tied arguments wins |
//!
//! Questions 7, 8, 9, 10 and 12 are not the engine's business: 7 and 9 ask
//! whether the player post-processes a reported title, 8 asks about the
//! numbered `%1`–`%7` style (this engine does not implement it), 10 asks
//! where the `comment` field name was seen, and 12 asks where the shipped
//! defaults live.
//!
//! # The other choices the spec leaves to the implementer
//!
//! * A field name is matched case-insensitively, so `%TrackNumber%` finds
//!   the same tag as `%tracknumber%`.
//! * `$pad` pads on the right, `$cut` and `$pad` count characters (not
//!   bytes), and a width that is not a number leaves the argument
//!   unchanged.
//! * A malformed format string is literal text: a `%` without its closing
//!   `%`, a `$` that does not start a call and a `]` without its `[` are
//!   characters, not syntax. A `[` without its `]` is a group that runs to
//!   the end of the string.
//! * A group renders when at least one field **inside** it is valid —
//!   that is S14 §2.3's own rule, and S14 §2.3's `$if(A,A,)` restatement
//!   of it. The rule is a property of the group's content, not of the
//!   text the group produces.

use std::collections::BTreeMap;

/// The field names the material carries (S14 §2.1).
///
/// The list is the spec's own: `title`, `filename`, `artist`, `album`,
/// `performer`, `part`, `tracknumber`, `comment` and `streamtitle`. Any
/// other name is accepted as well (S14 §2.1: "any tag name is accepted"),
/// so a row may carry more fields than these.
pub const FIELD_NAMES: [&str; 9] = [
    "title",
    "filename",
    "artist",
    "album",
    "performer",
    "part",
    "tracknumber",
    "comment",
    "streamtitle",
];

/// The depth limit for nested groups and calls (our choice; S14 §4 Q6).
///
/// The reference's own limit is unknown, so this one only has to keep the
/// parser's recursion bounded: a format string nested deeper than this is
/// not interpreted, its brackets and its `$` are literal text.
pub const MAX_NESTING_DEPTH: usize = 64;

/// The default format string: the Vorbis decoder's own default (S14 §2.4).
///
/// It is the one of the spec's four strings that produces the classic
/// "artist - title" row.
pub const DEFAULT_TEMPLATE: &str = "[$if2(%performer%,%artist%) - ]$if2(%title%,%filename%)";

/// The field values a playlist row offers to the format language (S14 §2.1).
///
/// A field the row does not carry yields the empty string (S14 §2.1), and
/// the conditional group of S14 §2.3 is how a format hides that gap. The
/// names are matched case-insensitively.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleFields {
    values: BTreeMap<String, String>,
}

impl TitleFields {
    /// A row that carries no field at all.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a field's value. The name is matched case-insensitively.
    pub fn set(&mut self, name: &str, value: &str) {
        self.values
            .insert(name.to_ascii_lowercase(), value.to_string());
    }

    /// Sets a field's value and returns the fields (the builder form of
    /// [`Self::set`]).
    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.set(name, value);
        self
    }

    /// The field's value, or `None` when the row does not carry the field.
    ///
    /// A field the row carries with an empty value is `Some("")`: the
    /// field is there, but it is not a *valid* field (S14 §4 Q11, our
    /// choice: validity means a non-empty value).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// A title-format string and the engine that renders it (S14 §2).
///
/// The type carries the switch as well: the format is **off** by default
/// ([`Self::off`], [`Default`]), so the row text stays the plain text
/// until a user turns it on. A format built with [`Self::new`] is on,
/// because the caller named a template; the default one is off, because
/// nobody asked for a template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleFormat {
    enabled: bool,
    template: String,
}

impl Default for TitleFormat {
    /// Off, with the default template: today's behaviour until a user
    /// turns the format on.
    fn default() -> Self {
        Self::off()
    }
}

impl TitleFormat {
    /// A format that renders `template`.
    ///
    /// The format is **on**: the caller asked for it by name.
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            enabled: true,
            template: template.into(),
        }
    }

    /// A format that is **off**: the row text stays the plain text.
    pub fn off() -> Self {
        Self {
            enabled: false,
            template: DEFAULT_TEMPLATE.to_string(),
        }
    }

    /// Whether the format is used for the row text.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Turns the format on or off.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// The format string.
    pub fn template(&self) -> &str {
        &self.template
    }

    /// Replaces the format string.
    pub fn set_template(&mut self, template: impl Into<String>) {
        self.template = template.into();
    }

    /// Renders the format string with the row's fields.
    ///
    /// This is the engine itself; the switch belongs to [`Self::row_text`].
    pub fn render(&self, fields: &TitleFields) -> String {
        let mut parser = Parser::new(self.template.as_str());
        let items = parser.parse_items(Stop::End);
        render_items(&items, fields).text
    }

    /// The row text: the plain text while the format is off, the rendered
    /// template once it is on.
    ///
    /// The plain text is what the playlist shows today (the queue entry's
    /// own display name), so an off format changes nothing.
    pub fn row_text(&self, plain: &str, fields: &TitleFields) -> String {
        if self.enabled {
            self.render(fields)
        } else {
            plain.to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// The parsed format string
// ---------------------------------------------------------------------------

/// One item of a parsed format string.
#[derive(Debug, Clone)]
enum Item {
    /// Literal text.
    Text(String),
    /// A field token: `%name%`.
    Field(String),
    /// A conditional group: `[ … ]` (S14 §2.3).
    Group(Vec<Item>),
    /// A function call: `$name( … )` (S14 §2.2).
    Call(Call),
}

/// A parsed function call.
#[derive(Debug, Clone)]
struct Call {
    /// The function's name, without the `$`.
    name: String,
    /// The arguments, each a parsed format string.
    args: Vec<Vec<Item>>,
    /// The call's own source text, for an unknown function name
    /// (S14 §4 Q5, our choice: an unknown name is inserted literally).
    source: String,
}

/// Where a run of items ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The end of the format string.
    End,
    /// The end of a conditional group (`]`).
    Group,
    /// The end of an argument (`,` or `)`).
    Argument,
}

/// The outcome of a `$` that was read as the start of a call.
#[derive(Debug, Clone)]
enum CallOutcome {
    /// A well-formed call.
    Call(Call),
    /// A malformed call: its text is literal.
    Literal(String),
}

/// The parser of a format string.
struct Parser<'a> {
    text: &'a str,
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            pos: 0,
            depth: 0,
        }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn eat(&mut self, ch: char) -> bool {
        if self.peek() == Some(ch) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Parses the items until the stop.
    fn parse_items(&mut self, stop: Stop) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        let mut text = String::new();
        while let Some(ch) = self.peek() {
            match ch {
                '\\' => {
                    // The escape of our own choice (S14 §4 Q4): the next
                    // character is literal.
                    self.bump();
                    match self.bump() {
                        Some(escaped) => text.push(escaped),
                        None => text.push('\\'),
                    }
                }
                '%' => match self.parse_field() {
                    Some(item) => {
                        flush(&mut items, &mut text);
                        items.push(item);
                    }
                    None => {
                        self.bump();
                        text.push('%');
                    }
                },
                '$' => match self.parse_call() {
                    Some(CallOutcome::Call(call)) => {
                        flush(&mut items, &mut text);
                        items.push(Item::Call(call));
                    }
                    Some(CallOutcome::Literal(raw)) => {
                        text.push_str(&raw);
                    }
                    None => {
                        self.bump();
                        text.push('$');
                    }
                },
                '[' => {
                    if self.depth >= MAX_NESTING_DEPTH {
                        // Too deep to nest: the bracket is literal text.
                        self.bump();
                        text.push('[');
                        continue;
                    }
                    flush(&mut items, &mut text);
                    self.bump();
                    self.depth += 1;
                    let inner = self.parse_items(Stop::Group);
                    self.depth -= 1;
                    // A group without its `]` runs to the end of the string.
                    self.eat(']');
                    items.push(Item::Group(inner));
                }
                ']' => {
                    if stop == Stop::Group {
                        break;
                    }
                    self.bump();
                    text.push(']');
                }
                ',' | ')' => {
                    if stop == Stop::Argument {
                        break;
                    }
                    self.bump();
                    text.push(ch);
                }
                _ => {
                    self.bump();
                    text.push(ch);
                }
            }
        }
        flush(&mut items, &mut text);
        items
    }

    /// Parses a `%name%` token.
    ///
    /// `None` when the `%` does not close a field: the `%` is then literal
    /// text.
    fn parse_field(&mut self) -> Option<Item> {
        let open = self.pos;
        self.bump();
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch == '%' {
                break;
            }
            self.bump();
        }
        if self.peek() != Some('%') {
            self.pos = open;
            return None;
        }
        let name = self.text[start..self.pos].trim().to_string();
        self.bump();
        Some(Item::Field(name))
    }

    /// Parses a `$name( … )` call.
    ///
    /// `None` when the `$` does not start a call at all (the `$` is then
    /// literal text); [`CallOutcome::Literal`] when the call is malformed
    /// (its text is literal as well).
    fn parse_call(&mut self) -> Option<CallOutcome> {
        let open = self.pos;
        self.bump();
        let name_start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                self.bump();
            } else {
                break;
            }
        }
        let name = self.text[name_start..self.pos].to_string();
        if name.is_empty() || !self.eat('(') {
            self.pos = open;
            return None;
        }
        if self.depth >= MAX_NESTING_DEPTH {
            // Too deep to nest: the `$` is literal text.
            self.pos = open;
            return None;
        }
        self.depth += 1;
        let mut args = Vec::new();
        loop {
            let arg = self.parse_items(Stop::Argument);
            args.push(arg);
            if self.eat(',') {
                continue;
            }
            break;
        }
        self.depth -= 1;
        if !self.eat(')') {
            // An unterminated call is not a call: its text is literal.
            return Some(CallOutcome::Literal(self.text[open..self.pos].to_string()));
        }
        Some(CallOutcome::Call(Call {
            name,
            args,
            source: self.text[open..self.pos].to_string(),
        }))
    }
}

/// Moves the pending literal text into the item list.
fn flush(items: &mut Vec<Item>, text: &mut String) {
    if !text.is_empty() {
        items.push(Item::Text(std::mem::take(text)));
    }
}

// ---------------------------------------------------------------------------
// The renderer
// ---------------------------------------------------------------------------

/// A rendered item: its text and whether a field inside it was valid.
#[derive(Debug, Clone, Default)]
struct Rendered {
    text: String,
    found: bool,
}

/// Renders the items.
fn render_items(items: &[Item], fields: &TitleFields) -> Rendered {
    let mut out = Rendered::default();
    for item in items {
        let rendered = render_item(item, fields);
        out.text.push_str(&rendered.text);
        out.found |= rendered.found;
    }
    out
}

/// Renders one item.
fn render_item(item: &Item, fields: &TitleFields) -> Rendered {
    match item {
        Item::Text(text) => Rendered {
            text: text.clone(),
            found: false,
        },
        Item::Field(name) => {
            let value = fields.get(name).unwrap_or_default();
            Rendered {
                text: value.to_string(),
                found: !value.is_empty(),
            }
        }
        Item::Group(items) => {
            let inner = render_items(items, fields);
            // S14 §2.3: the block appears only when at least one field
            // inside it was found; otherwise the whole block, brackets
            // included, disappears.
            if inner.found {
                Rendered {
                    text: inner.text,
                    found: true,
                }
            } else {
                Rendered::default()
            }
        }
        Item::Call(call) => render_call(call, fields),
    }
}

/// Renders a function call (S14 §2.2).
fn render_call(call: &Call, fields: &TitleFields) -> Rendered {
    let args: Vec<Rendered> = call
        .args
        .iter()
        .map(|arg| render_items(arg, fields))
        .collect();
    // A call counts as "found" when any of its arguments holds a valid
    // field: that is what an enclosing group looks at.
    let found = args.iter().any(|arg| arg.found);
    let text = match call.name.as_str() {
        "abbr" => abbreviate_args(&args),
        "caps" => caps(arg_text(&args, 0), false),
        "caps2" => caps(arg_text(&args, 0), true),
        "cut" => cut(arg_text(&args, 0), arg_text(&args, 1)),
        "if" => if_args(&args),
        "if2" => if2_args(&args),
        "longest" => longest(&args),
        "lower" => ascii_lower(arg_text(&args, 0)),
        "num" => num(arg_text(&args, 0), arg_text(&args, 1)),
        "pad" => pad(arg_text(&args, 0), arg_text(&args, 1)),
        "padcut" => padcut(arg_text(&args, 0), arg_text(&args, 1)),
        "upper" => ascii_upper(arg_text(&args, 0)),
        // An unknown function name is inserted literally (S14 §4 Q5, our
        // choice): the rest of the format is not dropped.
        _ => call.source.clone(),
    };
    Rendered { text, found }
}

/// The text of the argument at `index`, or the empty string.
fn arg_text(args: &[Rendered], index: usize) -> &str {
    args.get(index).map_or("", |arg| arg.text.as_str())
}

/// `$if(A,B,C)`: `B` when `A` holds a valid field, `C` otherwise.
fn if_args(args: &[Rendered]) -> String {
    if args.first().is_some_and(|arg| arg.found) {
        arg_text(args, 1).to_string()
    } else {
        arg_text(args, 2).to_string()
    }
}

/// `$if2(A,B)`: `A` when `A` holds a valid field, `B` otherwise.
///
/// S14 §2.2: `$if2(A,B)` is `$if(A,A,B)`.
fn if2_args(args: &[Rendered]) -> String {
    if args.first().is_some_and(|arg| arg.found) {
        arg_text(args, 0).to_string()
    } else {
        arg_text(args, 1).to_string()
    }
}

/// `$longest(A,B,…)`: the longest of the arguments' outputs.
///
/// A tie is broken by the first of the tied arguments (S14 §4 Q13, our
/// choice).
fn longest(args: &[Rendered]) -> String {
    let mut best: Option<&Rendered> = None;
    for arg in args {
        let longer = match best {
            None => true,
            Some(current) => arg.text.chars().count() > current.text.chars().count(),
        };
        if longer {
            best = Some(arg);
        }
    }
    best.map_or_else(String::new, |arg| arg.text.clone())
}

/// `$abbr(x)` and `$abbr(x,y)`.
fn abbreviate_args(args: &[Rendered]) -> String {
    let text = arg_text(args, 0);
    match args.get(1) {
        Some(limit) => match parse_count(&limit.text) {
            // Abbreviate only when the text is longer than the limit.
            Some(count) if text.chars().count() > count => abbreviate(text),
            _ => text.to_string(),
        },
        None => abbreviate(text),
    }
}

/// The abbreviation of `text`: the first ASCII letter of every word,
/// upper-cased (S14 §4 Q1, our choice).
fn abbreviate(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if let Some(letter) = word.chars().find(|ch| ch.is_ascii_alphabetic()) {
            out.push(letter.to_ascii_uppercase());
        }
    }
    out
}

/// `$num(x,y)`: the number `x`, padded with leading zeros to `y` characters.
///
/// A value that is not a number has no numeric rendering and yields the
/// empty string (S14 §4 Q2, our choice).
fn num(text: &str, width: &str) -> String {
    let Ok(value) = text.trim().parse::<i64>() else {
        return String::new();
    };
    let digits = value.to_string();
    match parse_count(width) {
        Some(count) => zero_pad(&digits, count),
        None => digits,
    }
}

/// `$pad(x,y)`: `x` padded with spaces on the right up to `y` characters.
fn pad(text: &str, width: &str) -> String {
    match parse_count(width) {
        Some(count) => pad_right(text, count),
        None => text.to_string(),
    }
}

/// `$cut(x,y)`: `x` truncated to `y` characters.
fn cut(text: &str, width: &str) -> String {
    match parse_count(width) {
        Some(count) => take_chars(text, count),
        None => text.to_string(),
    }
}

/// `$padcut(x,y)`: `x` padded to `y` characters and truncated when longer.
fn padcut(text: &str, width: &str) -> String {
    match parse_count(width) {
        Some(count) => {
            if text.chars().count() > count {
                take_chars(text, count)
            } else {
                pad_right(text, count)
            }
        }
        None => text.to_string(),
    }
}

/// `$caps(x)` and `$caps2(x)`: the first ASCII letter of every word
/// upper-cased, the rest of the word lower-cased.
///
/// `$caps2` leaves the letters that are already upper-case alone
/// (S14 §2.2). The folding is ASCII-only (S14 §4 Q3, our choice).
fn caps(text: &str, keep_upper: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_word_start = true;
    for ch in text.chars() {
        if ch.is_whitespace() {
            out.push(ch);
            at_word_start = true;
        } else if at_word_start {
            if ch.is_ascii_alphabetic() {
                out.push(ch.to_ascii_uppercase());
                at_word_start = false;
            } else {
                out.push(ch);
            }
        } else if keep_upper && ch.is_ascii_uppercase() {
            out.push(ch);
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

/// The lower-cased text (ASCII only; S14 §4 Q3, our choice).
fn ascii_lower(text: &str) -> String {
    text.chars().map(|ch| ch.to_ascii_lowercase()).collect()
}

/// The upper-cased text (ASCII only; S14 §4 Q3, our choice).
fn ascii_upper(text: &str) -> String {
    text.chars().map(|ch| ch.to_ascii_uppercase()).collect()
}

/// The number a width argument names, or `None` when it names none.
fn parse_count(text: &str) -> Option<usize> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<usize>().ok()
}

/// The first `count` characters of `text`.
fn take_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

/// `text` padded with spaces on the right up to `count` characters.
fn pad_right(text: &str, count: usize) -> String {
    let len = text.chars().count();
    if len >= count {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + (count - len));
    out.push_str(text);
    for _ in 0..(count - len) {
        out.push(' ');
    }
    out
}

/// `digits` padded with leading zeros up to `count` characters.
fn zero_pad(digits: &str, count: usize) -> String {
    let len = digits.chars().count();
    if len >= count {
        return digits.to_string();
    }
    let (sign, rest) = digits
        .strip_prefix('-')
        .map_or(("", digits), |rest| ("-", rest));
    let mut out = String::with_capacity(count + sign.len());
    out.push_str(sign);
    for _ in 0..(count - len) {
        out.push('0');
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row with the fields the spec's own strings use.
    fn sample() -> TitleFields {
        TitleFields::new()
            .with("title", "The Title")
            .with("filename", "the-title.mp3")
            .with("artist", "The Artist")
            .with("album", "The Album")
            .with("tracknumber", "3")
    }

    /// Renders a format string with the engine.
    fn render(template: &str, fields: &TitleFields) -> String {
        TitleFormat::new(template).render(fields)
    }

    // -- the spec's own vectors (S14 §2.4) ----------------------------------

    /// The module decoder's own default (S14 §2.4).
    #[test]
    fn the_module_decoder_default_renders_the_title_and_the_part() {
        let fields = TitleFields::new()
            .with("title", "Song")
            .with("filename", "song.mp3")
            .with("part", "Intro");
        assert_eq!(
            render("$if2(%title%,%filename%)[ (%part%)]", &fields),
            "Song (Intro)"
        );
        // The group hides the gap of a missing field (S14 §2.3).
        let fields = TitleFields::new()
            .with("title", "Song")
            .with("filename", "song.mp3");
        assert_eq!(
            render("$if2(%title%,%filename%)[ (%part%)]", &fields),
            "Song"
        );
    }

    /// The Vorbis decoder's own default (S14 §2.4).
    #[test]
    fn the_vorbis_decoder_default_renders_the_artist_dash_the_title() {
        let fields = TitleFields::new()
            .with("artist", "The Artist")
            .with("title", "The Title")
            .with("filename", "the-title.mp3");
        assert_eq!(
            render(
                "[$if2(%performer%,%artist%) - ]$if2(%title%,%filename%)",
                &fields
            ),
            "The Artist - The Title"
        );
        // The performer wins over the artist.
        let fields = fields.with("performer", "The Performer");
        assert_eq!(
            render(
                "[$if2(%performer%,%artist%) - ]$if2(%title%,%filename%)",
                &fields
            ),
            "The Performer - The Title"
        );
        // Without a title the filename takes its place.
        let fields = TitleFields::new()
            .with("artist", "The Artist")
            .with("filename", "the-title.mp3");
        assert_eq!(
            render(
                "[$if2(%performer%,%artist%) - ]$if2(%title%,%filename%)",
                &fields
            ),
            "The Artist - the-title.mp3"
        );
    }

    /// The help text's illustration (S14 §2.4).
    #[test]
    fn the_help_text_illustration_abbreviates_the_album() {
        let fields = TitleFields::new()
            .with("artist", "Pink Floyd")
            .with("album", "The Dark Side of the Moon")
            .with("tracknumber", "3")
            .with("title", "Time");
        assert_eq!(
            render(
                "[%artist% - ][$abbr(%album%,10)[ %tracknumber%] / ]%title%[ %streamtitle%]",
                &fields
            ),
            "Pink Floyd - TDSOTM 3 / Time"
        );
    }

    /// The help text's example of the conditional group (S14 §2.4).
    #[test]
    fn the_conditional_group_example_hides_the_missing_album() {
        let fields = TitleFields::new().with("title", "The Title");
        assert_eq!(
            render("%artist% - [%album% / ]%title%", &fields),
            " - The Title"
        );
        let fields = fields.with("album", "The Album");
        assert_eq!(
            render("%artist% - [%album% / ]%title%", &fields),
            " - The Album / The Title"
        );
    }

    // -- the fields (S14 §2.1) ----------------------------------------------

    #[test]
    fn a_field_the_row_does_not_carry_yields_the_empty_string() {
        assert_eq!(render("%artist%", &TitleFields::new()), "");
        assert_eq!(render("[%artist%]", &TitleFields::new()), "");
    }

    #[test]
    fn a_field_name_is_matched_case_insensitively() {
        let fields = sample().with("TrackNumber", "7");
        assert_eq!(render("%tracknumber%", &fields), "7");
        assert_eq!(render("%TrackNumber%", &fields), "7");
    }

    #[test]
    fn the_field_names_are_the_ones_the_spec_lists() {
        assert_eq!(
            FIELD_NAMES,
            [
                "title",
                "filename",
                "artist",
                "album",
                "performer",
                "part",
                "tracknumber",
                "comment",
                "streamtitle",
            ]
        );
    }

    // -- the conditional group (S14 §2.3) -----------------------------------

    #[test]
    fn a_group_with_only_empty_fields_disappears() {
        let fields = TitleFields::new()
            .with("artist", "")
            .with("album", "")
            .with("title", "The Title");
        assert_eq!(render("[%artist% - %album%]", &fields), "");
        // One valid field is enough.
        let fields = fields.with("artist", "The Artist");
        assert_eq!(render("[%artist% - %album%]", &fields), "The Artist - ");
    }

    #[test]
    fn a_group_without_any_field_disappears() {
        assert_eq!(render("[plain text]", &sample()), "");
    }

    #[test]
    fn groups_nest() {
        let fields = sample();
        assert_eq!(render("[[%artist%]]", &fields), "The Artist");
        // The inner group is empty, so the outer one has no valid field.
        assert_eq!(render("[[%nosuchfield%]]", &fields), "");
        // The inner group carries the field, so the outer one renders.
        assert_eq!(render("[[%artist%]x]", &fields), "The Artistx");
    }

    #[test]
    fn an_unterminated_group_runs_to_the_end_of_the_string() {
        let fields = sample();
        assert_eq!(
            render("[%artist% - %album%", &fields),
            "The Artist - The Album"
        );
        // A group without a valid field is still a group: it disappears.
        assert_eq!(render("[%nosuchfield%", &fields), "");
    }

    #[test]
    fn a_stray_closing_bracket_is_literal_text() {
        assert_eq!(render("a ] b", &sample()), "a ] b");
    }

    // -- the functions (S14 §2.2) -------------------------------------------

    #[test]
    fn the_case_functions_fold_ascii_only() {
        // S14 §4 Q3, our choice: only `a`–`z` and `A`–`Z` change.
        let fields = sample().with("title", "Grüße ÄÖÜ");
        assert_eq!(render("$lower(%title%)", &fields), "grüße ÄÖÜ");
        assert_eq!(render("$upper(%title%)", &fields), "GRüßE ÄÖÜ");
    }

    #[test]
    fn caps_upper_cases_the_first_letter_of_every_word() {
        let fields = sample().with("title", "the dark side OF the moon");
        assert_eq!(
            render("$caps(%title%)", &fields),
            "The Dark Side Of The Moon"
        );
    }

    #[test]
    fn caps2_leaves_the_letters_that_are_already_upper_case_alone() {
        let fields = sample().with("title", "iPhone SE");
        assert_eq!(render("$caps2(%title%)", &fields), "IPhone SE");
        assert_eq!(render("$caps(%title%)", &fields), "Iphone Se");
    }

    #[test]
    fn num_pads_the_number_with_leading_zeros() {
        let fields = sample().with("tracknumber", "7");
        assert_eq!(render("$num(%tracknumber%,2)", &fields), "07");
        assert_eq!(render("$num(%tracknumber%,4)", &fields), "0007");
        // A number that is longer than the width is not truncated.
        assert_eq!(render("$num(%tracknumber%,1)", &fields), "7");
    }

    #[test]
    fn num_on_a_non_numeric_value_yields_the_empty_string() {
        // S14 §4 Q2, our choice.
        let fields = sample().with("tracknumber", "3/12");
        assert_eq!(render("$num(%tracknumber%,2)", &fields), "");
        let fields = TitleFields::new();
        assert_eq!(render("$num(%tracknumber%,2)", &fields), "");
    }

    #[test]
    fn pad_pads_on_the_right_and_cut_truncates() {
        let fields = sample().with("title", "The Title");
        assert_eq!(render("$pad(%title%,12)", &fields), "The Title   ");
        assert_eq!(render("$cut(%title%,3)", &fields), "The");
        // A text that is longer than the width is left alone by `$pad`.
        assert_eq!(render("$pad(%title%,3)", &fields), "The Title");
    }

    #[test]
    fn padcut_pads_and_truncates() {
        let fields = sample().with("title", "The Title");
        assert_eq!(render("$padcut(%title%,12)", &fields), "The Title   ");
        assert_eq!(render("$padcut(%title%,3)", &fields), "The");
    }

    #[test]
    fn longest_takes_the_longest_argument() {
        let fields = sample().with("comment", "A much longer comment");
        assert_eq!(
            render("$longest(%title%,%album%,%comment%)", &fields),
            "A much longer comment"
        );
    }

    #[test]
    fn longest_breaks_a_tie_with_the_first_argument() {
        // S14 §4 Q13, our choice.
        let fields = sample();
        assert_eq!(render("$longest(%title%,%album%)", &fields), "The Title");
    }

    #[test]
    fn abbr_takes_the_initials_of_the_words() {
        // S14 §4 Q1, our choice.
        let fields = sample().with("album", "The Dark Side of the Moon");
        assert_eq!(render("$abbr(%album%)", &fields), "TDSOTM");
    }

    #[test]
    fn abbr_with_a_limit_only_abbreviates_a_longer_text() {
        let fields = sample().with("album", "The Dark Side of the Moon");
        assert_eq!(render("$abbr(%album%,10)", &fields), "TDSOTM");
        // Shorter than the limit: the text goes in unchanged.
        let fields = sample().with("album", "Short");
        assert_eq!(render("$abbr(%album%,10)", &fields), "Short");
    }

    #[test]
    fn if_takes_its_second_argument_when_the_first_holds_a_valid_field() {
        let fields = sample();
        assert_eq!(render("$if(%artist%,yes,no)", &fields), "yes".to_string());
        assert_eq!(render("$if(%nosuchfield%,yes,no)", &fields), "no");
    }

    #[test]
    fn if2_is_if_with_the_first_argument_twice() {
        let fields = sample();
        assert_eq!(render("$if2(%title%,%filename%)", &fields), "The Title");
        let fields = TitleFields::new().with("filename", "the-title.mp3");
        assert_eq!(render("$if2(%title%,%filename%)", &fields), "the-title.mp3");
    }

    // -- the open questions we had to answer -------------------------------

    #[test]
    fn an_unknown_function_is_inserted_literally() {
        // S14 §4 Q5, our choice: the call's own source text goes in and the
        // rest of the format survives.
        let fields = sample();
        assert_eq!(render("$nosuch(%title%)", &fields), "$nosuch(%title%)");
        assert_eq!(
            render("a $nosuch(%title%) b", &fields),
            "a $nosuch(%title%) b"
        );
    }

    #[test]
    fn an_unknown_function_inside_a_group_needs_a_valid_field() {
        let fields = sample();
        assert_eq!(render("[$nosuch(%title%)]", &fields), "$nosuch(%title%)");
        assert_eq!(render("[$nosuch(%nosuchfield%)]", &fields), "");
    }

    #[test]
    fn a_backslash_escapes_the_next_character() {
        // S14 §4 Q4, our choice.
        let fields = sample();
        assert_eq!(render(r"\%title\%", &fields), "%title%");
        assert_eq!(render(r"\[%title%\]", &fields), "[The Title]");
        assert_eq!(render(r"\$lower(%title%)", &fields), "$lower(The Title)");
        assert_eq!(render(r"a\\b", &fields), r"a\b");
    }

    #[test]
    fn a_field_value_is_never_re_parsed() {
        // S14 §4 Q4, our choice: the value goes in as it is, so a `]`
        // inside a tag cannot close a group.
        let fields = sample().with("comment", "a ] b");
        assert_eq!(render("%comment%", &fields), "a ] b");
        assert_eq!(render("[%comment%]", &fields), "a ] b");
        assert_eq!(render("[$lower(%comment%)]", &fields), "a ] b");
    }

    #[test]
    fn a_malformed_format_string_is_literal_text() {
        let fields = sample();
        assert_eq!(render("%title", &fields), "%title");
        assert_eq!(render("$lower", &fields), "$lower");
        assert_eq!(render("$ lower(%title%)", &fields), "$ lower(The Title)");
        assert_eq!(render("$lower(%title%", &fields), "$lower(%title%");
    }

    #[test]
    fn the_nesting_depth_is_capped() {
        // S14 §4 Q6, our choice: the depth is bounded, so a pathological
        // format string cannot overflow the stack.
        let deep = format!("{}{}", "[".repeat(500), "%title%");
        let rendered = render(&deep, &sample());
        assert!(rendered.contains("The Title"), "{rendered}");
        let deep = format!("{}{}", "[".repeat(500), "%title%");
        assert!(!render(&deep, &sample()).is_empty());
    }

    #[test]
    fn a_long_input_renders_without_a_limit() {
        // S14 §4 Q6, our choice: no length limit on the format string.
        let long = "x".repeat(50_000);
        let fields = TitleFields::new().with("title", long.as_str());
        assert_eq!(render("%title%", &fields).len(), 50_000);
        let template = "%title%".repeat(5_000);
        assert_eq!(render(&template, &fields).len(), 50_000 * 5_000);
    }

    // -- the switch ---------------------------------------------------------

    #[test]
    fn the_format_is_off_by_default() {
        let format = TitleFormat::default();
        assert!(!format.is_enabled());
        assert_eq!(
            format.row_text("the plain text", &sample()),
            "the plain text"
        );
    }

    #[test]
    fn an_enabled_format_replaces_the_plain_text() {
        let mut format = TitleFormat::default();
        format.set_enabled(true);
        assert_eq!(
            format.row_text("the plain text", &sample()),
            "The Artist - The Title"
        );
    }

    #[test]
    fn the_default_template_is_the_vorbis_decoders_own_default() {
        assert_eq!(TitleFormat::default().template(), DEFAULT_TEMPLATE);
        assert_eq!(
            TitleFormat::default().template(),
            "[$if2(%performer%,%artist%) - ]$if2(%title%,%filename%)"
        );
    }
}
