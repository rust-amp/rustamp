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
///
/// A plain decimal number is converted with integer arithmetic, so the value
/// is exact for every millisecond count - also the ones a `f64` cannot hold
/// (above 2^53 ms). Everything else (a sign, an exponent, more than three
/// fraction digits) goes through the floating point path.
pub fn parse_seconds_ms(text: &str) -> Option<u64> {
    if let Some(ms) = parse_exact_seconds_ms(text) {
        return Some(ms);
    }
    parse_scaled_ms(text, 1000.0)
}

/// Parses a length in milliseconds (XSPF `duration`).
///
/// A plain integer is parsed exactly, so a duration of more than 2^53 ms
/// survives a write → read round trip; everything else goes through the
/// floating point path.
pub fn parse_ms(text: &str) -> Option<u64> {
    if let Some(ms) = parse_exact_ms(text) {
        return Some(ms);
    }
    parse_scaled_ms(text, 1.0)
}

/// Parses a plain decimal number of seconds into milliseconds, exactly.
///
/// Accepts digits with at most one dot and at most three fraction digits
/// (`"123"`, `"123.4"`, `"123.456"`). The result is computed with integer
/// arithmetic, so it is exact for every millisecond count.
///
/// `None` for anything else, so the caller falls back to the floating point
/// path.
fn parse_exact_seconds_ms(text: &str) -> Option<u64> {
    let (int_part, frac_part) = split_decimal(text.trim())?;
    let int_value: u64 = int_part.parse().ok()?;
    let frac_value: u64 = if frac_part.is_empty() {
        0
    } else {
        if frac_part.len() > 3 {
            return None;
        }
        let digits: u64 = frac_part.parse().ok()?;
        digits * 10u64.pow(3 - frac_part.len() as u32)
    };
    int_value.checked_mul(1000)?.checked_add(frac_value)
}

/// Parses a plain integer number of milliseconds, exactly.
///
/// `None` for anything that is not a sequence of digits, so the caller falls
/// back to the floating point path.
fn parse_exact_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Splits a plain decimal number into its integer and fraction digits.
///
/// `None` when the text is not a non-empty sequence of digits with at most
/// one dot and only digits after it.
fn split_decimal(text: &str) -> Option<(&str, &str)> {
    if text.is_empty() {
        return None;
    }
    let (int_part, frac_part) = match text.split_once('.') {
        Some((int_part, frac_part)) => {
            if frac_part.contains('.') {
                return None;
            }
            (int_part, frac_part)
        }
        None => (text, ""),
    };
    if int_part.is_empty() || !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !frac_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((int_part, frac_part))
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
/// This is the exact inverse of [`parse_seconds_ms`] for every value, which is
/// what makes write → read round-trips lossless.
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

    #[test]
    fn plain_decimals_are_parsed_exactly() {
        assert_eq!(parse_seconds_ms("0.001"), Some(1));
        assert_eq!(parse_seconds_ms("1.5"), Some(1500));
        assert_eq!(parse_seconds_ms("1.50"), Some(1500));
        assert_eq!(parse_seconds_ms("1.500"), Some(1500));
        assert_eq!(parse_seconds_ms("123."), Some(123_000));
        assert_eq!(parse_seconds_ms("  12.5  "), Some(12_500));
        // Not a plain decimal: the floating point path takes over.
        assert_eq!(parse_seconds_ms(".5"), Some(500));
        assert_eq!(parse_seconds_ms("+5"), Some(5000));
    }

    #[test]
    fn huge_values_are_parsed_exactly() {
        // Above 2^53 ms a `f64` cannot hold the value any more, so the
        // floating point path would lose a millisecond.
        let ms = 9_007_199_254_740_991u64;
        assert_eq!(parse_seconds_ms("9007199254740.991"), Some(ms));
        assert_eq!(parse_seconds_ms(&format_seconds(ms)), Some(ms));
        assert_eq!(parse_ms(&ms.to_string()), Some(ms));
        assert_eq!(parse_leading_seconds_ms("9007199254740.991"), Some(ms));
    }
}
