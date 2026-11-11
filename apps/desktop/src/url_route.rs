//! The browser's URL space (web plan §3.5): the Svelte site's paths, parsed into the screen to
//! open and written back for the address bar. Pure, on both targets.
//!
//! | Path | [`Deep`] |
//! |---|---|
//! | `/` | `Home` |
//! | `/new` | `New` |
//! | `/play?stick=&natural=` | `Create` (makes the table, then the URL becomes `/play/<id>`) |
//! | `/play/<id>` | `Play` |
//! | `/games` | `Games` |
//! | `/games/<id>` | `Match` |
//! | `/settings` | `Settings` |
//! | anything else | `Unknown` |

use crate::app::Route;

/// Where a URL points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Deep {
    /// `/`.
    Home,
    /// `/new`.
    New,
    /// `/play` with the house rules in the query.
    Create {
        /// `stick=true`.
        stick: bool,
        /// `natural=true`.
        natural: bool,
    },
    /// `/play/<id>`.
    Play(String),
    /// `/games`.
    Games,
    /// `/games/<id>`.
    Match(String),
    /// `/settings`.
    Settings,
    /// A path the app does not own (the path, as given).
    Unknown(String),
}

/// Parses a URL path and query (`query` with or without the leading `?`). Trailing slashes are
/// ignored; ids are percent-decoded and may be any non-empty text (the server decides whether
/// the game exists).
pub fn parse(path: &str, query: &str) -> Deep {
    let trimmed = path.trim_end_matches('/');
    let segments: Vec<&str> = trimmed.split('/').skip(1).collect();
    match segments.as_slice() {
        [] | [""] => Deep::Home,
        ["new"] => Deep::New,
        ["play"] => Deep::Create {
            stick: query_flag(query, "stick"),
            natural: query_flag(query, "natural"),
        },
        ["play", id] if !id.is_empty() => Deep::Play(percent_decode(id)),
        ["games"] => Deep::Games,
        ["games", id] if !id.is_empty() => Deep::Match(percent_decode(id)),
        ["settings"] => Deep::Settings,
        _ => Deep::Unknown(path.to_owned()),
    }
}

/// The path for `route`; `game_id` is the open table's id. The table has no path until it has
/// an id, so that case is `None`.
pub fn path_for(route: &Route, game_id: Option<&str>) -> Option<String> {
    Some(match route {
        Route::Home => "/".to_owned(),
        Route::NewGame => "/new".to_owned(),
        Route::Table => format!("/play/{}", percent_encode(game_id?)),
        Route::Games => "/games".to_owned(),
        Route::MatchDetail(id) => format!("/games/{}", percent_encode(id)),
        Route::Settings => "/settings".to_owned(),
    })
}

fn query_flag(query: &str, name: &str) -> bool {
    query
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .any(|(k, v)| k == name && v == "true")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b9c1d0e-1b6e-4c52-9f0a-3b1e0f6a7c11";

    #[test]
    fn every_svelte_route_parses() {
        assert_eq!(parse("/", ""), Deep::Home);
        assert_eq!(parse("/new", ""), Deep::New);
        assert_eq!(parse("/games", ""), Deep::Games);
        assert_eq!(parse("/settings", ""), Deep::Settings);
        assert_eq!(parse(&format!("/play/{ID}"), ""), Deep::Play(ID.to_owned()));
        assert_eq!(
            parse(&format!("/games/{ID}"), ""),
            Deep::Match(ID.to_owned())
        );
    }

    #[test]
    fn play_without_an_id_creates_with_the_query_rules() {
        assert_eq!(
            parse("/play", ""),
            Deep::Create {
                stick: false,
                natural: false
            }
        );
        assert_eq!(
            parse("/play", "?stick=true&natural=false"),
            Deep::Create {
                stick: true,
                natural: false
            }
        );
        assert_eq!(
            parse("/play/", "natural=true"),
            Deep::Create {
                stick: false,
                natural: true
            }
        );
    }

    #[test]
    fn trailing_slashes_are_ignored() {
        assert_eq!(parse("/games/", ""), Deep::Games);
        assert_eq!(parse("/new//", ""), Deep::New);
        assert_eq!(
            parse(&format!("/play/{ID}/"), ""),
            Deep::Play(ID.to_owned())
        );
        assert_eq!(parse("", ""), Deep::Home);
    }

    #[test]
    fn ids_need_not_be_uuids_and_are_decoded() {
        assert_eq!(parse("/play/abc", ""), Deep::Play("abc".to_owned()));
        assert_eq!(
            parse("/games/a%20b%2Fc", ""),
            Deep::Match("a b/c".to_owned())
        );
        assert_eq!(parse("/play/%zz", ""), Deep::Play("%zz".to_owned()));
        assert_eq!(parse("/play/%", ""), Deep::Play("%".to_owned()));
    }

    #[test]
    fn other_paths_are_unknown() {
        assert_eq!(
            parse("/cardtest", ""),
            Deep::Unknown("/cardtest".to_owned())
        );
        assert_eq!(
            parse("/play/a/b", ""),
            Deep::Unknown("/play/a/b".to_owned())
        );
        assert_eq!(
            parse("/settings/x", ""),
            Deep::Unknown("/settings/x".to_owned())
        );
        assert_eq!(
            parse("/download/x", ""),
            Deep::Unknown("/download/x".to_owned())
        );
    }

    #[test]
    fn paths_round_trip() {
        for (route, id) in [
            (Route::Home, None),
            (Route::NewGame, None),
            (Route::Table, Some(ID)),
            (Route::Games, None),
            (Route::MatchDetail("a b".to_owned()), None),
            (Route::Settings, None),
        ] {
            let path = path_for(&route, id).expect("has a path");
            let back = match parse(&path, "") {
                Deep::Home => Route::Home,
                Deep::New => Route::NewGame,
                Deep::Play(_) => Route::Table,
                Deep::Games => Route::Games,
                Deep::Match(m) => Route::MatchDetail(m),
                Deep::Settings => Route::Settings,
                other => panic!("{other:?}"),
            };
            assert_eq!(back, route, "{path}");
        }
        assert_eq!(path_for(&Route::Table, None), None);
        assert_eq!(
            path_for(&Route::MatchDetail("a b".to_owned()), None).as_deref(),
            Some("/games/a%20b")
        );
    }
}
