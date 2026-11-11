//! The `Range` header, cleaned before `tower-http` sees it.
//!
//! `ServeFile` and `ServeDir` serve one byte range (`206`) and refuse more
//! (`416`). Before refusing, their parser (`http-range-header`) compares every
//! range in the header with every other one, so a long multi-range header costs
//! CPU that grows with the square of its length. [`keep_single_byte_range`]
//! keeps only a request for one byte range, rewritten in the one spelling the
//! parser accepts, and drops every other `Range` header. A dropped header gets
//! the whole file with `200`, which RFC 9110 section 14.2 allows ("a server MAY
//! ignore the Range header field"), and it is what the RFC requires for a range
//! unit the server does not know.

use axum::http::{HeaderMap, HeaderValue, header};

/// A `Range` value longer than this is dropped unread. One byte range with two
/// 20-digit positions is 47 bytes.
pub const MAX_RANGE_BYTES: usize = 100;

/// Keep at most one `Range` header, and only when it names one byte range.
///
/// - Kept: `bytes=<first>-[<last>]` and `bytes=-<suffix>`, with the unit in any
///   letter case and optional spaces around `=` and `-`. It is rewritten as
///   `bytes=<first>-<last>` with no spaces and no leading zeros, the form the
///   parser accepts. A position too large for 64 bits counts as the largest one.
/// - With `len` (the size of the file), a suffix longer than the file becomes
///   `bytes=0-`, so the whole file comes back as `206` (RFC 9110 section
///   14.1.3) rather than `416`.
/// - Dropped: more than one `Range` line, a value over [`MAX_RANGE_BYTES`], a
///   unit other than `bytes`, a list of ranges, and anything that is not one
///   range of digits.
///
/// Reversed (`bytes=5-2`) and unsatisfiable (`bytes=9999-` on a short file)
/// single ranges are kept, so `tower-http` still answers them with `416`.
pub fn keep_single_byte_range(headers: &mut HeaderMap, len: Option<u64>) {
    let mut values = headers.get_all(header::RANGE).iter();
    let Some(first) = values.next() else {
        return;
    };
    let kept = if values.next().is_some() {
        None
    } else {
        canonical(first, len)
    };
    headers.remove(header::RANGE);
    if let Some(kept) = kept {
        headers.insert(header::RANGE, kept);
    }
}

/// One position: decimal digits only. Too large for `u64` saturates.
fn position(digits: &str) -> Option<u64> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(digits.parse::<u64>().unwrap_or(u64::MAX))
}

fn canonical(value: &HeaderValue, len: Option<u64>) -> Option<HeaderValue> {
    if value.len() > MAX_RANGE_BYTES {
        return None;
    }
    let (unit, set) = value.to_str().ok()?.split_once('=')?;
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (first, last) = set.trim().split_once('-')?;
    let (first, last) = (first.trim(), last.trim());
    let spec = if first.is_empty() {
        let suffix = position(last)?;
        match len {
            Some(len) if suffix > len => "bytes=0-".to_owned(),
            _ => format!("bytes=-{suffix}"),
        }
    } else {
        let first = position(first)?;
        if last.is_empty() {
            format!("bytes={first}-")
        } else {
            format!("bytes={first}-{}", position(last)?)
        }
    };
    HeaderValue::from_str(&spec).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleaned(values: &[&str], len: Option<u64>) -> Option<String> {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(
                header::RANGE,
                HeaderValue::from_str(value).unwrap_or(HeaderValue::from_static("x")),
            );
        }
        keep_single_byte_range(&mut headers, len);
        assert!(headers.get_all(header::RANGE).iter().count() <= 1);
        headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    #[test]
    fn one_byte_range_is_kept_in_canonical_form() {
        for (raw, want) in [
            ("bytes=0-9", "bytes=0-9"),
            ("bytes=100-", "bytes=100-"),
            ("bytes=-500", "bytes=-500"),
            ("BYTES=0-1", "bytes=0-1"),
            ("Bytes = 0 - 1", "bytes=0-1"),
            (" bytes=00-015 ", "bytes=0-15"),
            ("bytes=5-2", "bytes=5-2"),
            ("bytes=-0", "bytes=-0"),
            (
                "bytes=99999999999999999999999-",
                "bytes=18446744073709551615-",
            ),
            (
                "bytes=0-99999999999999999999999",
                "bytes=0-18446744073709551615",
            ),
        ] {
            assert_eq!(cleaned(&[raw], Some(4096)).as_deref(), Some(want), "{raw}");
        }
    }

    #[test]
    fn a_suffix_longer_than_the_file_is_the_whole_file() {
        assert_eq!(
            cleaned(&["bytes=-4097"], Some(4096)).as_deref(),
            Some("bytes=0-")
        );
        assert_eq!(
            cleaned(&["bytes=-4096"], Some(4096)).as_deref(),
            Some("bytes=-4096")
        );
        // Without a known size the suffix is left alone.
        assert_eq!(
            cleaned(&["bytes=-4097"], None).as_deref(),
            Some("bytes=-4097")
        );
    }

    #[test]
    fn everything_else_is_dropped() {
        let long = format!("bytes=0-{}", "9".repeat(MAX_RANGE_BYTES));
        for raw in [
            "lines=0-1",
            "items=0-1",
            "bytes=0-1,5-6",
            "bytes=0-1, 5-6",
            "bytes=0-1,",
            "bytes=",
            "bytes=-",
            "bytes=abc",
            "bytes=1-a",
            "bytes=+1-2",
            "bytes=-1-2",
            "bytes 0-1",
            "0-1",
            long.as_str(),
        ] {
            assert_eq!(cleaned(&[raw], Some(4096)), None, "{raw}");
        }
        assert_eq!(cleaned(&["bytes=0-1", "bytes=2-3"], Some(4096)), None);
        assert_eq!(cleaned(&[], Some(4096)), None);
    }

    #[test]
    fn a_huge_multi_range_header_is_dropped_without_parsing() {
        let huge = std::iter::once("bytes=0-0".to_owned())
            .chain((1..4000).map(|i| format!("{}-{}", i * 2, i * 2)))
            .collect::<Vec<_>>()
            .join(",");
        assert!(huge.len() > 30_000);
        let started = std::time::Instant::now();
        assert_eq!(cleaned(&[&huge], Some(1 << 30)), None);
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
    }
}
