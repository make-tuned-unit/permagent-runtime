//! Shared RSS/feed-reading primitives — the zero-dependency core that both the
//! Watcher and the Grow audience-listening tool read feeds through.
//!
//! Extracted from the Watcher (`goose-server/src/proactive.rs`, #672), which
//! pioneered parsing RSS by hand against a `reqwest` built with
//! `default-features = false`. That build has **no `.query()` helper**, so query
//! strings are assembled with [`percent_encode`] and full URLs, never
//! `.query(...)`. Keeping these helpers in one place means the two feed readers
//! cannot drift.

use chrono::DateTime;

/// One parsed feed item. Any of `link` / `description` / `pub_date` may be empty
/// when the source omits it; the caller decides what it requires (the Watcher
/// needs a `pub_date` for its freshness gate, the listener tolerates its
/// absence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub link: String,
    pub description: String,
    pub pub_date: String,
}

/// Parse up to `max` `<item>` entries in document order. Only items with a
/// non-empty `<title>` are returned (a title-less item is noise). Never panics —
/// malformed input yields as many items as it can, then stops.
// string_slice: every index below is either a `find()` result (a byte offset on
// a char boundary) or that plus the byte length of an ASCII tag literal (ASCII
// bytes are themselves char boundaries), so no slice can split a UTF-8 sequence.
#[allow(clippy::string_slice)]
pub fn parse_items(xml: &str, max: usize) -> Vec<Item> {
    let mut items = Vec::new();
    let mut rest = xml;
    while items.len() < max {
        let Some(open) = rest.find("<item>") else {
            break;
        };
        let after_open = &rest[open + "<item>".len()..];
        let Some(close) = after_open.find("</item>") else {
            break;
        };
        let block = &after_open[..close];
        rest = &after_open[close + "</item>".len()..];

        let title = between(block, "<title>", "</title>")
            .map(clean_xml)
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let link = between(block, "<link>", "</link>")
            .map(clean_xml)
            .unwrap_or_default();
        let description = between(block, "<description>", "</description>")
            .map(clean_xml)
            .unwrap_or_default();
        let pub_date = between(block, "<pubDate>", "</pubDate>")
            .map(clean_xml)
            .unwrap_or_default();
        items.push(Item {
            title,
            link,
            description,
            pub_date,
        });
    }
    items
}

/// The first `<item>` of a feed, or `None`. Convenience over [`parse_items`] for
/// callers that only want the freshest entry (the Watcher's news check).
pub fn first_item(xml: &str) -> Option<Item> {
    parse_items(xml, 1).into_iter().next()
}

/// Build a Google News RSS search URL for `query` (US English). This is the
/// zero-config default "channel": any topic becomes a live feed of what is being
/// said about it. Assembled by hand — this `reqwest` has no `.query()` helper.
pub fn google_news_search_url(query: &str) -> String {
    format!(
        "https://news.google.com/rss/search?q={}&hl=en-US&gl=US&ceid=US:en",
        percent_encode(query)
    )
}

/// Percent-encode a query value, keeping the RFC 3986 unreserved set literal.
/// Needed because this `reqwest` build lacks the `.query()` helper.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Parse an RFC-2822 date (an RSS `<pubDate>`) to epoch milliseconds.
pub fn parse_rfc2822_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc2822(s.trim())
        .ok()
        .map(|d| d.timestamp_millis())
}

/// The text strictly between the first `start` and the next following `end`.
// string_slice: `find` returns a byte offset on a char boundary and `start.len()`
// advances past an ASCII tag, so both slice indices are valid char boundaries.
#[allow(clippy::string_slice)]
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let rest = &s[i..];
    let j = rest.find(end)?;
    Some(&rest[..j])
}

/// Trim, unwrap a single CDATA wrapper, and decode the handful of XML entities
/// RSS titles and summaries actually use.
fn clean_xml(s: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix("<![CDATA[")
        .and_then(|x| x.strip_suffix("]]>"))
        .unwrap_or(t);
    t.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<rss><channel>
        <item>
            <title><![CDATA[Acme raises $50M &amp; hires]]></title>
            <link>https://news.example/a</link>
            <description>Big &amp; splashy round</description>
            <pubDate>Tue, 07 Jul 2026 12:00:00 GMT</pubDate>
        </item>
        <item>
            <title>Second story</title>
            <link>https://news.example/b</link>
            <description>More detail</description>
            <pubDate>Wed, 08 Jul 2026 12:00:00 GMT</pubDate>
        </item>
        <item>
            <title>Third story</title>
            <link>https://news.example/c</link>
        </item>
    </channel></rss>"#;

    #[test]
    fn parses_multiple_items_in_order() {
        let items = parse_items(FEED, 10);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "Acme raises $50M & hires");
        assert_eq!(items[0].link, "https://news.example/a");
        assert_eq!(items[0].description, "Big & splashy round");
        assert_eq!(items[1].title, "Second story");
        // A missing <description>/<pubDate> is tolerated as empty, not skipped.
        assert_eq!(items[2].title, "Third story");
        assert!(items[2].pub_date.is_empty());
    }

    #[test]
    fn respects_the_max_cap() {
        assert_eq!(parse_items(FEED, 1).len(), 1);
        assert_eq!(parse_items(FEED, 2).len(), 2);
    }

    #[test]
    fn skips_title_less_items_and_survives_garbage() {
        assert!(parse_items("<rss><channel></channel></rss>", 5).is_empty());
        assert!(parse_items("not xml at all", 5).is_empty());
        assert!(parse_items("<item><link>x</link></item>", 5).is_empty());
    }

    #[test]
    fn first_item_is_the_freshest() {
        let it = first_item(FEED).expect("an item");
        assert_eq!(it.title, "Acme raises $50M & hires");
        assert!(parse_rfc2822_ms(&it.pub_date).is_some());
    }

    #[test]
    fn google_news_url_is_hand_built_and_encoded() {
        let url = google_news_search_url("local-first software");
        assert!(url.starts_with("https://news.google.com/rss/search?q="));
        // Space encoded, unreserved hyphen kept literal, fixed locale suffix.
        assert!(url.contains("local-first%20software"));
        assert!(url.ends_with("&hl=en-US&gl=US&ceid=US:en"));
    }

    #[test]
    fn percent_encode_keeps_unreserved_and_escapes_the_rest() {
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(percent_encode("a b&c"), "a%20b%26c");
    }
}
