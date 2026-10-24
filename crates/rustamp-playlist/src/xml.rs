//! A minimal XML reader, written for this crate because no dependency is
//! allowed.
//!
//! It is a forward-only pull parser: [`Reader::next_event`] yields
//! [`Event::Start`], [`Event::End`] and [`Event::Text`] until the input is
//! exhausted. It understands the XML prologue (`<?…?>`), comments
//! (`<!--…-->`), DOCTYPE declarations with an internal subset, CDATA sections,
//! self-closing elements and quoted or unquoted attribute values.
//!
//! It is deliberately **lenient**: this parser reads untrusted playlist files,
//! so nothing here validates, rejects or panics. Malformed input degrades into
//! text events, unterminated constructs run to the end of the input, and every
//! call advances at least one byte, so a truncated document can never loop
//! forever. Namespace prefixes are kept as part of the name; use
//! [`local_name`] to compare names.

/// One XML token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event<'a> {
    /// An opening tag. Attributes keep their raw (still escaped) values.
    Start {
        /// Element name, with any namespace prefix.
        name: &'a str,
        /// Attributes as `(name, value)` pairs, in document order.
        attrs: Vec<(&'a str, &'a str)>,
    },
    /// A closing tag.
    End {
        /// Element name, with any namespace prefix.
        name: &'a str,
    },
    /// Character data between tags, still entity-escaped. CDATA sections are
    /// reported as text as well (they are already unescaped by definition).
    Text(&'a str),
}

/// A forward-only XML tokenizer.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    input: &'a str,
    pos: usize,
    pending_end: Option<&'a str>,
}

impl<'a> Reader<'a> {
    /// A reader over `input`.
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            pending_end: None,
        }
    }

    /// The next token, or `None` at the end of the input.
    pub fn next_event(&mut self) -> Option<Event<'a>> {
        if let Some(name) = self.pending_end.take() {
            return Some(Event::End { name });
        }
        loop {
            if self.pos >= self.input.len() {
                return None;
            }
            let rest = &self.input[self.pos..];

            if rest.starts_with("<!--") {
                self.pos += 4;
                self.skip_until("-->");
                continue;
            }
            if rest.starts_with("<?") {
                self.pos += 2;
                self.skip_until("?>");
                continue;
            }
            if rest.starts_with("<![CDATA[") {
                let start = self.pos + 9;
                let (text, next) = match self.input[start..].find("]]>") {
                    Some(i) => (&self.input[start..start + i], start + i + 3),
                    None => (&self.input[start..], self.input.len()),
                };
                self.pos = next;
                if text.is_empty() {
                    continue;
                }
                return Some(Event::Text(text));
            }
            if rest.starts_with("<!") {
                self.pos += 2;
                self.skip_declaration();
                continue;
            }
            if rest.starts_with("</") {
                self.pos += 2;
                let name = self.take_name();
                self.skip_until(">");
                return Some(Event::End { name });
            }
            if rest.starts_with('<') {
                self.pos += 1;
                let name = self.take_name();
                if name.is_empty() {
                    // A stray `<`: report it as text so the caller sees the
                    // bytes and the parser always advances.
                    return Some(Event::Text(&self.input[self.pos - 1..self.pos]));
                }
                let attrs = self.take_attrs();
                let self_closing = self.consume_tag_end();
                if self_closing {
                    self.pending_end = Some(name);
                }
                return Some(Event::Start { name, attrs });
            }

            // A run of character data.
            let end = rest
                .find('<')
                .map(|i| self.pos + i)
                .unwrap_or(self.input.len());
            let text = &self.input[self.pos..end];
            self.pos = end;
            if text.is_empty() {
                continue;
            }
            return Some(Event::Text(text));
        }
    }

    /// Advances past `pattern`, or to the end of the input when it is missing.
    fn skip_until(&mut self, pattern: &str) {
        match self.input[self.pos..].find(pattern) {
            Some(i) => self.pos += i + pattern.len(),
            None => self.pos = self.input.len(),
        }
    }

    fn skip_ws(&mut self) {
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    /// Consumes a name: everything up to whitespace, `/`, `>` or `=`.
    fn take_name(&mut self) -> &'a str {
        let start = self.pos;
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if b.is_ascii_whitespace() || b == b'/' || b == b'>' || b == b'=' {
                break;
            }
            self.pos += 1;
        }
        &self.input[start..self.pos]
    }

    fn take_attrs(&mut self) -> Vec<(&'a str, &'a str)> {
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            let bytes = self.input.as_bytes();
            if self.pos >= bytes.len() {
                break;
            }
            let b = bytes[self.pos];
            if b == b'>' || b == b'/' {
                break;
            }
            let name = self.take_name();
            if name.is_empty() {
                // Unexpected character inside the tag: skip it and go on.
                self.pos += 1;
                continue;
            }
            self.skip_ws();
            let value = if self.input.as_bytes().get(self.pos) == Some(&b'=') {
                self.pos += 1;
                self.skip_ws();
                self.take_value()
            } else {
                ""
            };
            attrs.push((name, value));
        }
        attrs
    }

    fn take_value(&mut self) -> &'a str {
        let bytes = self.input.as_bytes();
        if self.pos >= bytes.len() {
            return "";
        }
        let quote = bytes[self.pos];
        if quote == b'"' || quote == b'\'' {
            self.pos += 1;
            let start = self.pos;
            match self.input[start..].find(quote as char) {
                Some(i) => {
                    self.pos = start + i + 1;
                    &self.input[start..start + i]
                }
                None => {
                    self.pos = self.input.len();
                    &self.input[start..]
                }
            }
        } else {
            let start = self.pos;
            while self.pos < bytes.len() {
                let b = bytes[self.pos];
                if b.is_ascii_whitespace() || b == b'>' || b == b'/' {
                    break;
                }
                self.pos += 1;
            }
            &self.input[start..self.pos]
        }
    }

    /// Consumes `/>` or `>`; returns `true` for a self-closing tag.
    fn consume_tag_end(&mut self) -> bool {
        self.skip_ws();
        let rest = &self.input[self.pos..];
        if let Some(after) = rest.strip_prefix("/>") {
            self.pos = self.input.len() - after.len();
            true
        } else if let Some(after) = rest.strip_prefix('>') {
            self.pos = self.input.len() - after.len();
            false
        } else {
            // Malformed tag: leave the position; the caller has already
            // advanced past `<` and the name.
            false
        }
    }

    /// Skips `<!…>`, honouring quoted strings and an internal `[…]` subset.
    fn skip_declaration(&mut self) {
        let bytes = self.input.as_bytes();
        let mut depth: i32 = 0;
        let mut quote: Option<u8> = None;
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            self.pos += 1;
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                }
                continue;
            }
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'[' => depth += 1,
                b']' => depth -= 1,
                b'>' if depth <= 0 => return,
                _ => {}
            }
        }
    }
}

/// The part of a name after the last `:` (the namespace prefix is dropped).
pub fn local_name(name: &str) -> &str {
    match name.rsplit_once(':') {
        Some((_, local)) => local,
        None => name,
    }
}

/// The value of the first attribute whose local name matches `local`
/// (case-insensitive), e.g. `base` for `xml:base`.
pub fn attr<'a>(attrs: &[(&'a str, &'a str)], local: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(name, _)| local_name(name).eq_ignore_ascii_case(local))
        .map(|(_, value)| *value)
}

/// Unescapes the five predefined XML entities and numeric character
/// references.
///
/// Unknown or malformed entities are kept verbatim, and a numeric reference to
/// an invalid code point is kept verbatim as well.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        let tail = &rest[index..];
        // Entity names are short; a `;` further away is not an entity.
        match tail[1..].find(';') {
            Some(end) if end <= 12 => {
                let entity = &tail[1..1 + end];
                match decode_entity(entity) {
                    Some(c) => {
                        out.push(c);
                        rest = &tail[end + 2..];
                    }
                    None => {
                        out.push('&');
                        rest = &tail[1..];
                    }
                }
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = entity.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(input: &str) -> Vec<Event<'_>> {
        let mut reader = Reader::new(input);
        let mut out = Vec::new();
        while let Some(event) = reader.next_event() {
            out.push(event);
            assert!(out.len() < 1000, "the reader must terminate");
        }
        out
    }

    #[test]
    fn reads_elements_attributes_and_text() {
        let evs = events(r#"<a x="1" y='2'><b>hi</b></a>"#);
        assert_eq!(
            evs,
            vec![
                Event::Start {
                    name: "a",
                    attrs: vec![("x", "1"), ("y", "2")]
                },
                Event::Start {
                    name: "b",
                    attrs: vec![]
                },
                Event::Text("hi"),
                Event::End { name: "b" },
                Event::End { name: "a" },
            ]
        );
    }

    #[test]
    fn self_closing_tags_emit_start_and_end() {
        let evs = events("<a/><b />");
        assert_eq!(
            evs,
            vec![
                Event::Start {
                    name: "a",
                    attrs: vec![]
                },
                Event::End { name: "a" },
                Event::Start {
                    name: "b",
                    attrs: vec![]
                },
                Event::End { name: "b" },
            ]
        );
    }

    #[test]
    fn skips_the_prologue_comments_and_doctype() {
        let evs: Vec<_> = events(
            "<?xml version=\"1.0\"?>\n<!-- a > comment -->\n<!DOCTYPE p [ <!ENTITY x \"y\"> ]>\n<playlist/>",
        )
        .into_iter()
        .filter(|e| !matches!(e, Event::Text(t) if t.trim().is_empty()))
        .collect();
        assert_eq!(
            evs,
            vec![
                Event::Start {
                    name: "playlist",
                    attrs: vec![]
                },
                Event::End { name: "playlist" },
            ]
        );
    }

    #[test]
    fn reads_cdata_as_text() {
        let evs = events("<a><![CDATA[x < y & z]]></a>");
        assert_eq!(evs[1], Event::Text("x < y & z"));
    }

    #[test]
    fn namespace_prefixes_and_helpers() {
        let evs = events(r#"<x:track xml:base="http://e/"/>"#);
        let Event::Start { name, attrs } = &evs[0] else {
            panic!("expected a start tag");
        };
        assert_eq!(*name, "x:track");
        assert_eq!(local_name(name), "track");
        assert_eq!(attr(attrs, "base"), Some("http://e/"));
        assert_eq!(attr(attrs, "missing"), None);
    }

    #[test]
    fn unescapes_entities() {
        assert_eq!(
            unescape("a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos;"),
            "a & b <c> \"d\" 'e'"
        );
        assert_eq!(unescape("&#65;&#x42;&#X43;"), "ABC");
        assert_eq!(
            unescape("&unknown; &amp &;&#xZZ;"),
            "&unknown; &amp &;&#xZZ;"
        );
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape("&"), "&");
        assert_eq!(unescape("&#x110000;"), "&#x110000;");
        assert_eq!(unescape("&#xD800;"), "&#xD800;");
    }

    #[test]
    fn malformed_documents_do_not_panic_or_hang() {
        let inputs = [
            "",
            "<",
            "<a",
            "<a ",
            "<a b",
            "<a b=",
            "<a b='",
            "<a b=\"unterminated",
            "</",
            "</a",
            "<a><b></a>",
            "<a><![CDATA[unterminated",
            "<a><!-- unterminated",
            "<a><?pi unterminated",
            "<a><!DOCTYPE unterminated",
            "<a>&",
            ">text<",
            "<a>\u{20AC}\u{FFFD}</a>",
            "<a x=1><b/><c/>tail",
            "<a b=\"\u{00E4}\"/>",
        ];
        for input in inputs {
            let mut reader = Reader::new(input);
            let mut count = 0;
            while reader.next_event().is_some() {
                count += 1;
                assert!(count < 1000, "no progress on {input:?}");
            }
        }
    }

    #[test]
    fn unterminated_text_runs_to_the_end() {
        let evs = events("tail without tags");
        assert_eq!(evs, vec![Event::Text("tail without tags")]);
    }

    #[test]
    fn unquoted_attribute_values_are_read() {
        let evs = events("<a x=1 y=2/>");
        assert_eq!(
            evs[0],
            Event::Start {
                name: "a",
                attrs: vec![("x", "1"), ("y", "2")]
            }
        );
    }
}
