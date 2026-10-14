//! Number parsing and formatting shared by the format readers and writers.
//!
//! Playlist formats disagree about how they express a length: M3U `#EXTINF`
//! and PLS `LengthN` use *seconds* (frequently a fraction), XSPF `duration`
//! uses *milliseconds*. All of them are converted to the core unit, `u64`
//! milliseconds, and never panic on garbage.

/// Parses a full string as a number of `scale`-sized units and converts it to
/// whole milliseconds.
///
/// Rejects empty input, non-numeric input, negative values, NaN and infinity.
/// Out-of-range values saturate instead of wrapping.
fn parse_scaled_ms(text: &str, scale: f64) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let value: f64 = text.parse().ok()?;
    to_ms(value, scale)
}

fn to_ms(value: f64, scale: f64) -> Option<u64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let ms = (value * scale).round();
    if !ms.is_finite() {
        return None;
    }
    Some(ms as u64)
}

/// Parses a length in seconds (`"123"`, `"123.456"`) to milliseconds.
pub fn parse_seconds_ms(text: &str) -> Option<u64> {
    parse_scaled_ms(text, 1000.0)
}

/// Parses a length in milliseconds (XSPF `duration`).
pub fn parse_ms(text: &str) -> Option<u64> {
    parse_scaled_ms(text, 1.0)
}

/// Parses the *leading* number of a string as seconds.
///
/// `#EXTINF` values are sometimes followed by attributes
/// (`5 tvg-id="x"`), so parsing stops at the first character that cannot be
/// part of a number. A negative value (the conventional "unknown length"
/// marker, `#EXTINF:-1,Stream`) yields `None`.
pub fn parse_leading_seconds_ms(text: &str) -> Option<u64> {
    let text = text.trim_start();
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(text.len());
    parse_seconds_ms(&text[..end])
}

/// Formats milliseconds as seconds: an integer when the value is a whole number
/// of seconds, otherwise three decimals.
///
/// This is the exact inverse of [`parse_seconds_ms`] for values below 2^53 ms,
/// which is what makes write → read round-trips lossless.
pub fn format_seconds(ms: u64) -> String {
    if ms.is_multiple_of(1000) {
        (ms / 1000).to_string()
    } else {
        format!("{}.{:03}", ms / 1000, ms % 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_seconds() {
        assert_eq!(parse_seconds_ms("123"), Some(123_000));
        assert_eq!(parse_seconds_ms(" 123.456 "), Some(123_456));
        assert_eq!(parse_seconds_ms("0"), Some(0));
        assert_eq!(parse_seconds_ms("-1"), None);
        assert_eq!(parse_seconds_ms(""), None);
        assert_eq!(parse_seconds_ms("abc"), None);
        assert_eq!(parse_seconds_ms("1e3"), Some(1_000_000));
        assert_eq!(parse_seconds_ms("nan"), None);
        assert_eq!(parse_seconds_ms("inf"), None);
        assert_eq!(parse_seconds_ms("1e400"), None);
    }

    #[test]
    fn parses_leading_numbers() {
        assert_eq!(parse_leading_seconds_ms("5"), Some(5_000));
        assert_eq!(parse_leading_seconds_ms("5 tvg-id=\"x\""), Some(5_000));
        assert_eq!(parse_leading_seconds_ms("-1"), None);
        assert_eq!(parse_leading_seconds_ms(""), None);
        assert_eq!(parse_leading_seconds_ms("abc"), None);
        assert_eq!(parse_leading_seconds_ms("  12.5"), Some(12_500));
    }

    #[test]
    fn parses_milliseconds() {
        assert_eq!(parse_ms("123456"), Some(123_456));
        assert_eq!(parse_ms("123456.4"), Some(123_456));
        assert_eq!(parse_ms("-5"), None);
        assert_eq!(parse_ms("x"), None);
    }

    #[test]
    fn formats_seconds_exactly() {
        assert_eq!(format_seconds(123_000), "123");
        assert_eq!(format_seconds(123_456), "123.456");
        assert_eq!(format_seconds(456), "0.456");
        assert_eq!(format_seconds(0), "0");
    }

    #[test]
    fn format_and_parse_are_inverses() {
        for ms in [0u64, 1, 999, 1000, 1001, 123_456, 999_999_999, 86_400_000] {
            assert_eq!(parse_seconds_ms(&format_seconds(ms)), Some(ms), "ms = {ms}");
        }
    }
}
