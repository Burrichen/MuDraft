//! Reads a Pitchfork ranked-list story ("The 100 Best … of All Time").
//!
//! Each album is a rank heading followed by `<h2>Artist: <em>Title</em></h2>`, then a
//! caption `<p>Label ✦ Year</p>`. The scan is deliberately narrow: pages in another shape
//! yield too few albums and are reported as unsupported rather than guessed at.

use super::{ImportedEntry, ListImport};
use crate::error::{AppError, AppResult};

const RANK_MARKER: &str = r#"aria-level="3""#;
/// How far before an `<h2>` its rank heading may sit.
const RANK_WINDOW: usize = 400;
const MIN_ALBUMS: usize = 2;

pub fn parse(html: &str, source_url: &str) -> AppResult<ListImport> {
    let mut found: Vec<(i64, ImportedEntry, usize)> = Vec::new();
    let mut from = 0;
    while let Some(start) = html[from..].find("<h2>").map(|i| i + from) {
        let inner_start = start + "<h2>".len();
        let Some(end) = html[inner_start..].find("</h2>").map(|i| i + inner_start) else {
            break;
        };
        from = end;
        let Some(rank) = rank_before(html, start) else {
            continue;
        };
        let Some((artist, title)) = artist_and_title(&html[inner_start..end]) else {
            continue;
        };
        found.push((
            rank,
            ImportedEntry {
                artist_name: artist,
                title,
                year: None,
                position: rank,
            },
            end,
        ));
    }
    if found.len() < MIN_ALBUMS {
        return Err(AppError::validation(
            "address",
            "no ranked album list was found on that page",
        ));
    }
    // A caption belongs to the album whose heading precedes it, up to the next heading.
    let bounds: Vec<usize> = found
        .iter()
        .skip(1)
        .map(|(_, _, end)| *end)
        .chain([html.len()])
        .collect();
    let mut entries: Vec<ImportedEntry> = found
        .into_iter()
        .zip(bounds)
        .map(|((_, mut entry, end), next)| {
            entry.year = caption_year(&html[end..next]);
            entry
        })
        .collect();
    entries.sort_by_key(|e| e.position);
    entries.dedup_by(|a, b| a.position == b.position);
    Ok(ListImport {
        name: page_title(html).unwrap_or_else(|| "Pitchfork list".to_owned()),
        source_url: source_url.to_owned(),
        entries,
    })
}

/// The rank number in the heading just before `at` (`<div …aria-level="3"…>5.</div>`).
fn rank_before(html: &str, at: usize) -> Option<i64> {
    let from = floor_boundary(html, at.saturating_sub(RANK_WINDOW));
    let window = &html[from..at];
    let marker = window.rfind(RANK_MARKER)?;
    let after = &window[marker..];
    let text = after[after.find('>')? + 1..].split('<').next()?;
    let digits = text.trim().strip_suffix('.')?;
    digits
        .parse::<i64>()
        .ok()
        .filter(|r| (1..=9_999).contains(r))
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Splits `Silver Jews: <em>American Water</em>` at the title's emphasis, so artist names
/// that contain a colon ("Songs: Ohia") stay whole. Text after `</em>` ("EP") stays in the
/// title.
fn artist_and_title(inner: &str) -> Option<(String, String)> {
    let open = inner.find("<em>")?;
    let close = inner.find("</em>")?;
    if close < open {
        return None;
    }
    let artist = text(&inner[..open]);
    let artist = artist.trim_end_matches(':').trim().to_owned();
    let title = text(&format!(
        "{} {}",
        &inner[open + "<em>".len()..close],
        &inner[close + "</em>".len()..]
    ));
    (!artist.is_empty() && !title.is_empty()).then_some((artist, title))
}

/// The year in a `Label ✦ 1998` caption, if one appears in `section`.
fn caption_year(section: &str) -> Option<i64> {
    let after = &section[section.find('✦')? + '✦'.len_utf8()..];
    let digits: String = after
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse::<i64>()
        .ok()
        .filter(|y| digits.len() == 4 && (1000..=9999).contains(y))
}

fn page_title(html: &str) -> Option<String> {
    let start = html.find("<title>")? + "<title>".len();
    let raw = text(&html[start..start + html[start..].find("</title>")?]);
    let name = raw.strip_suffix("| Pitchfork").unwrap_or(&raw).trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Plain text of an HTML fragment: tags removed, entities decoded, spacing collapsed.
fn text(fragment: &str) -> String {
    let mut plain = String::with_capacity(fragment.len());
    let mut in_tag = false;
    for c in fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    decode_entities(&plain)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest
            .find(';')
            .filter(|&semi| semi <= 10)
            .and_then(|semi| entity(&rest[1..semi]).map(|c| (c, semi + 1)));
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "rsquo" | "lsquo" => Some('\u{2019}'),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://pitchfork.com/story/the-100-best-indie-rock-albums-of-all-time/";

    /// Mirrors the markup of the live page (attributes and long copy trimmed).
    const PAGE: &str = r#"<html><head><title>The 100 Best Indie Rock Albums of All Time | Pitchfork</title></head><body>
<h2 class="other">Related stories</h2>
<hr/><div role="heading" aria-level="3" class="heading-h3">25.</div><h2>Songs: Ohia: <em>The Magnolia Electric Co.</em></h2>
<figure><div><p>Secretly Canadian ✦ 2003</p></div></figure><p>Copy about the album.</p>
<hr/><div role="heading" aria-level="3" class="heading-h3">24.</div><h2>Yeah Yeah Yeahs: <em>Yeah Yeah Yeahs</em> EP</h2>
<figure><div><p>Shifty ✦ 2001</p></div></figure>
<hr/><div role="heading" aria-level="3" class="heading-h3">2.</div><h2>Liz Phair: <em>Exile in Guyville</em></h2>
<figure><div><p>Matador ✦ 1993</p></div></figure>
<hr/><div role="heading" aria-level="3" class="heading-h3">1.</div><h2>Pavement: <em>Crooked Rain, Crooked Rain</em></h2>
<figure></figure><p>No caption year here.</p>
<hr/><div role="heading" aria-level="3" class="heading-h3">3.</div><h2>Les Savy Fav: <em>Let&#x27;s Stay Friends</em></h2>
<figure><div><p>Frenchkiss ✦ 2007</p></div></figure>
</body></html>"#;

    #[test]
    fn reads_ranked_albums_in_rank_order() {
        let list = parse(PAGE, URL).unwrap();
        assert_eq!(list.name, "The 100 Best Indie Rock Albums of All Time");
        assert_eq!(list.source_url, URL);
        let got: Vec<_> = list
            .entries
            .iter()
            .map(|e| (e.position, e.artist_name.as_str(), e.title.as_str(), e.year))
            .collect();
        assert_eq!(
            got,
            vec![
                (1, "Pavement", "Crooked Rain, Crooked Rain", None),
                (2, "Liz Phair", "Exile in Guyville", Some(1993)),
                (3, "Les Savy Fav", "Let's Stay Friends", Some(2007)),
                (24, "Yeah Yeah Yeahs", "Yeah Yeah Yeahs EP", Some(2001)),
                (25, "Songs: Ohia", "The Magnolia Electric Co.", Some(2003)),
            ]
        );
    }

    #[test]
    fn a_page_without_a_ranked_list_is_unsupported() {
        let err = parse("<html><h2>Just an article</h2></html>", URL).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn decodes_entities_and_ignores_unknown_ones() {
        assert_eq!(
            text("Mates &amp; Dates &#8217;n&#x27; &unknown; &"),
            "Mates & Dates \u{2019}n' &unknown; &"
        );
    }
}
