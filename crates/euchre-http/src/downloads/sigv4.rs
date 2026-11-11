//! AWS Signature Version 4 query signing ("presigned URLs") for S3-compatible
//! buckets, hand-rolled on `hmac` + `sha2`. Only what the downloads need: a
//! virtual-host URL, `host` as the one signed header, `UNSIGNED-PAYLOAD`.
//!
//! Reference: AWS "Authenticating Requests: Using Query Parameters (AWS
//! Signature Version 4)"; the unit test below is that page's worked example.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// The signing algorithm name.
const ALGORITHM: &str = "AWS4-HMAC-SHA256";
/// The service in the credential scope.
const SERVICE: &str = "s3";

/// One request to presign.
#[derive(Clone, Copy)]
pub(crate) struct Presign<'a> {
    /// `GET` or `HEAD`.
    pub method: &'a str,
    /// `https` or `http`.
    pub scheme: &'a str,
    /// The `Host` header the client will send: `<bucket>.<endpoint host>[:port]`.
    pub host: &'a str,
    /// Object key, without a leading `/`.
    pub key: &'a str,
    /// Region in the credential scope (`auto`, `us-east-1`, ...).
    pub region: &'a str,
    /// Access key ID.
    pub access_key_id: &'a str,
    /// Secret access key.
    pub secret_access_key: &'a str,
    /// Signing time.
    pub at: SystemTime,
    /// `X-Amz-Expires`, in seconds.
    pub expires_secs: u64,
    /// Extra query parameters (unencoded), such as `response-content-disposition`.
    pub query: &'a [(&'a str, &'a str)],
}

/// The presigned URL for `p`.
pub(crate) fn presign(p: &Presign<'_>) -> String {
    let (date, datetime) = amz_date(p.at);
    let scope = format!("{date}/{}/{SERVICE}/aws4_request", p.region);
    let credential = format!("{}/{scope}", p.access_key_id);
    let expires = p.expires_secs.to_string();

    let mut params: Vec<(String, String)> = vec![
        (
            uri_encode("X-Amz-Algorithm", false),
            uri_encode(ALGORITHM, false),
        ),
        (
            uri_encode("X-Amz-Credential", false),
            uri_encode(&credential, false),
        ),
        (
            uri_encode("X-Amz-Date", false),
            uri_encode(&datetime, false),
        ),
        (
            uri_encode("X-Amz-Expires", false),
            uri_encode(&expires, false),
        ),
        (
            uri_encode("X-Amz-SignedHeaders", false),
            uri_encode("host", false),
        ),
    ];
    for (name, value) in p.query {
        params.push((uri_encode(name, false), uri_encode(value, false)));
    }
    params.sort();
    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let path = format!("/{}", uri_encode(p.key, true));
    let canonical_request = format!(
        "{}\n{path}\n{query}\nhost:{}\n\nhost\nUNSIGNED-PAYLOAD",
        p.method, p.host
    );
    let string_to_sign = format!(
        "{ALGORITHM}\n{datetime}\n{scope}\n{:x}",
        Sha256::digest(canonical_request.as_bytes())
    );

    let mut key = hmac(
        format!("AWS4{}", p.secret_access_key).as_bytes(),
        date.as_bytes(),
    );
    for part in [p.region, SERVICE, "aws4_request"] {
        key = hmac(&key, part.as_bytes());
    }
    let signature = hex(&hmac(&key, string_to_sign.as_bytes()));

    format!(
        "{}://{}{path}?{query}&X-Amz-Signature={signature}",
        p.scheme, p.host
    )
}

fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        unreachable!("HMAC takes a key of any length")
    };
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// SigV4 URI encoding: every byte but `A-Z a-z 0-9 - . _ ~` becomes `%XX`
/// (uppercase hex); `/` too, unless `keep_slash` (object key paths).
pub(crate) fn uri_encode(input: &str, keep_slash: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0x0f)]));
        }
    }
    out
}

/// `(YYYYMMDD, YYYYMMDDTHHMMSSZ)` in UTC. A time before 1970 counts as 1970.
pub(crate) fn amz_date(at: SystemTime) -> (String, String) {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, m, d) = civil_from_days(secs / 86_400);
    let rem = secs % 86_400;
    let date = format!("{y:04}{m:02}{d:02}");
    let datetime = format!(
        "{date}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    );
    (date, datetime)
}

/// Days since 1970-01-01 to a proleptic Gregorian `(year, month, day)`
/// (Howard Hinnant's `civil_from_days`, for non-negative days).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// 2013-05-24T00:00:00Z, the AWS example's signing time.
    fn may_24_2013() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_369_353_600)
    }

    /// AWS's published query-string example: GET examplebucket/test.txt,
    /// us-east-1, 20130524T000000Z, X-Amz-Expires=86400.
    #[test]
    fn matches_the_aws_presigned_url_example() {
        let url = presign(&Presign {
            method: "GET",
            scheme: "https",
            host: "examplebucket.s3.amazonaws.com",
            key: "test.txt",
            region: "us-east-1",
            access_key_id: "AKIAIOSFODNN7EXAMPLE",
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            at: may_24_2013(),
            expires_secs: 86_400,
            query: &[],
        });
        assert_eq!(
            url,
            "https://examplebucket.s3.amazonaws.com/test.txt\
             ?X-Amz-Algorithm=AWS4-HMAC-SHA256\
             &X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request\
             &X-Amz-Date=20130524T000000Z\
             &X-Amz-Expires=86400\
             &X-Amz-SignedHeaders=host\
             &X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
    }

    #[test]
    fn extra_parameters_are_encoded_and_sorted_after_the_amz_ones() {
        let url = presign(&Presign {
            method: "GET",
            scheme: "https",
            host: "b.t3.storageapi.dev",
            key: "files/Euchre-0.1.0+b.1-mac.dmg",
            region: "auto",
            access_key_id: "AK",
            secret_access_key: "SK",
            at: may_24_2013(),
            expires_secs: 3600,
            query: &[
                ("response-content-type", "application/x-apple-diskimage"),
                (
                    "response-content-disposition",
                    "attachment; filename=\"Euchre-0.1.0+b.1-mac.dmg\"",
                ),
            ],
        });
        let (path, query) = url.split_once('?').unwrap_or_default();
        assert_eq!(
            path,
            "https://b.t3.storageapi.dev/files/Euchre-0.1.0%2Bb.1-mac.dmg"
        );
        let names: Vec<&str> = query
            .split('&')
            .map(|kv| kv.split_once('=').map_or(kv, |(k, _)| k))
            .collect();
        assert_eq!(
            names,
            [
                "X-Amz-Algorithm",
                "X-Amz-Credential",
                "X-Amz-Date",
                "X-Amz-Expires",
                "X-Amz-SignedHeaders",
                "response-content-disposition",
                "response-content-type",
                "X-Amz-Signature",
            ]
        );
        assert!(query.contains(
            "response-content-disposition=attachment%3B%20filename%3D%22Euchre-0.1.0%2Bb.1-mac.dmg%22"
        ));
        assert!(query.contains("response-content-type=application%2Fx-apple-diskimage"));
        assert!(query.contains("X-Amz-Credential=AK%2F20130524%2Fauto%2Fs3%2Faws4_request"));
    }

    #[test]
    fn uri_encoding_follows_sigv4() {
        assert_eq!(uri_encode("aZ09-._~", false), "aZ09-._~");
        assert_eq!(uri_encode("a b;c=\"d\"", false), "a%20b%3Bc%3D%22d%22");
        assert_eq!(uri_encode("a/b+c", false), "a%2Fb%2Bc");
        assert_eq!(uri_encode("a/b+c", true), "a/b%2Bc");
        assert_eq!(uri_encode("é", false), "%C3%A9");
    }

    #[test]
    fn dates_are_utc() {
        assert_eq!(
            amz_date(may_24_2013()),
            ("20130524".to_owned(), "20130524T000000Z".to_owned())
        );
        assert_eq!(
            amz_date(UNIX_EPOCH),
            ("19700101".to_owned(), "19700101T000000Z".to_owned())
        );
        // 2024-02-29T23:59:59Z (a leap day).
        assert_eq!(
            amz_date(UNIX_EPOCH + Duration::from_secs(1_709_251_199)).1,
            "20240229T235959Z"
        );
        // 2000-03-01T12:34:56Z.
        assert_eq!(
            amz_date(UNIX_EPOCH + Duration::from_secs(951_914_096)).1,
            "20000301T123456Z"
        );
    }
}
