//! Server-side classification for incoming analytics beacons.
//!
//! Three jobs, all done at collect time so the raw signal is never stored:
//! bot detection from the user agent, UTM extraction from an allowlist (never
//! the whole query string), and referrer classification.
//!
//! Everything here is pure so it can be tested against real user-agent strings
//! and real referrers rather than by inspection.

use serde::Serialize;

/// Substrings that mark a request as automated. Matched case-insensitively
/// against the user agent.
///
/// Deliberately broad: on a site with 50 prerendered SEO pages, crawler traffic
/// is a large share of all requests, and counting it makes every figure noise.
/// A false positive costs one under-counted visit; a false negative pollutes
/// the number the user actually reads. `headless` is included because our own
/// verification browser counted itself as a visitor on the first real install.
const BOT_MARKERS: &[&str] = &[
    "bot",
    "crawler",
    "spider",
    "crawling",
    "headless",
    "phantomjs",
    "puppeteer",
    "playwright",
    "selenium",
    "curl/",
    "wget/",
    "python-requests",
    "httpx",
    "axios/",
    "go-http-client",
    "java/",
    "okhttp",
    "libwww",
    "scrapy",
    "slurp",
    "archiver",
    "monitoring",
    "uptime",
    "pingdom",
    "lighthouse",
    "gtmetrix",
    "pagespeed",
    "preview",
    "fetcher",
    "feedfetcher",
    "facebookexternalhit",
    "whatsapp",
    "telegrambot",
    "discordbot",
    "slackbot",
    "twitterbot",
    "linkedinbot",
    "embedly",
    "quora link preview",
    "bitlybot",
    "google-inspectiontool",
    "chrome-lighthouse",
    "ahrefs",
    "semrush",
    "mj12",
    "dotbot",
    "petalbot",
    "yandex",
    "bingpreview",
    "applebot",
    "duckduckbot",
    "baiduspider",
];

/// Is this user agent an automated client?
///
/// An ABSENT user agent counts as a bot: every real browser sends one, and
/// scripted traffic frequently does not.
pub fn is_bot(user_agent: Option<&str>) -> bool {
    let Some(ua) = user_agent else { return true };
    let ua = ua.trim();
    if ua.is_empty() {
        return true;
    }
    let lower = ua.to_ascii_lowercase();
    BOT_MARKERS.iter().any(|m| lower.contains(m))
}

/// Campaign parameters, extracted from an allowlist.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Utm {
    pub source: Option<String>,
    pub medium: Option<String>,
    pub campaign: Option<String>,
}

/// Maximum stored length for any single campaign value.
const UTM_MAX: usize = 128;

fn clamp(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(UTM_MAX).collect())
}

/// Pull campaign parameters out of a path that may carry a query string.
///
/// An ALLOWLIST, never the whole query: arbitrary query params routinely carry
/// emails, tokens and search terms, and storing them wholesale would drag PII
/// into a store that promises not to hold any. `gclid`/`fbclid` map to source
/// because that is the only campaign signal those ad platforms provide.
pub fn extract_utm(path_with_query: &str) -> Utm {
    let Some((_, query)) = path_with_query.split_once('?') else {
        return Utm::default();
    };
    let mut utm = Utm::default();
    for pair in query.split('&') {
        let (key, value) = match pair.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let value = percent_decode(value);
        match key.to_ascii_lowercase().as_str() {
            "utm_source" | "ref" => utm.source = utm.source.or_else(|| clamp(&value)),
            "utm_medium" => utm.medium = clamp(&value),
            "utm_campaign" => utm.campaign = clamp(&value),
            "gclid" => {
                utm.source = utm.source.or_else(|| Some("google".to_string()));
                utm.medium = utm.medium.or_else(|| Some("cpc".to_string()));
            }
            "fbclid" => {
                utm.source = utm.source.or_else(|| Some("facebook".to_string()));
                utm.medium = utm.medium.or_else(|| Some("cpc".to_string()));
            }
            _ => {}
        }
    }
    utm
}

/// Strip the query string from a path so the stored path aggregates.
///
/// Without this every `?utm_source=…` variant is a distinct "page" and the top
/// pages list fragments into near-duplicates.
pub fn normalize_path(path_with_query: &str) -> String {
    let base = path_with_query
        .split_once('?')
        .map(|(p, _)| p)
        .unwrap_or(path_with_query);
    let base = base.split_once('#').map(|(p, _)| p).unwrap_or(base);
    if base.is_empty() {
        "/".to_string()
    } else {
        base.chars().take(512).collect()
    }
}

/// How a visit arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferrerClass {
    Direct,
    Search,
    Social,
    Internal,
    Referral,
}

impl ReferrerClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ReferrerClass::Direct => "direct",
            ReferrerClass::Search => "search",
            ReferrerClass::Social => "social",
            ReferrerClass::Internal => "internal",
            ReferrerClass::Referral => "referral",
        }
    }
}

const SEARCH_HOSTS: &[&str] = &[
    "google.",
    "bing.",
    "duckduckgo.",
    "yahoo.",
    "ecosia.",
    "baidu.",
    "yandex.",
    "brave.",
    "startpage.",
    "qwant.",
];
const SOCIAL_HOSTS: &[&str] = &[
    "facebook.",
    "instagram.",
    "twitter.",
    "x.com",
    "t.co",
    "linkedin.",
    "reddit.",
    "pinterest.",
    "tiktok.",
    "youtube.",
    "threads.",
    "mastodon",
    "bsky.",
    "news.ycombinator.com",
];

/// Host portion of a URL, lowercased and without `www.`.
pub fn referrer_host(referrer: &str) -> Option<String> {
    let trimmed = referrer.trim();
    if trimmed.is_empty() {
        return None;
    }
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split('@')
        .next_back()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    if host.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}

/// Bucket a referrer, given the site's own host so self-referrals are seen as
/// internal rather than inflating the referral list.
pub fn classify_referrer(referrer: Option<&str>, site_host: Option<&str>) -> ReferrerClass {
    let Some(host) = referrer.and_then(referrer_host) else {
        return ReferrerClass::Direct;
    };
    if let Some(own) = site_host.and_then(referrer_host) {
        if host == own {
            return ReferrerClass::Internal;
        }
    }
    if SEARCH_HOSTS
        .iter()
        .any(|s| host.starts_with(s) || host.contains(s))
    {
        return ReferrerClass::Search;
    }
    if SOCIAL_HOSTS
        .iter()
        .any(|s| host.starts_with(s) || host.contains(s))
    {
        return ReferrerClass::Social;
    }
    ReferrerClass::Referral
}

// ── Event properties ────────────────────────────────────────────────────────

/// Clamps on a properties payload. Chosen to be generous enough for real
/// product events and small enough that a runaway client cannot bloat the
/// store.
pub const PROPS_MAX_KEYS: usize = 32;
pub const PROPS_MAX_VALUE_CHARS: usize = 256;
pub const PROPS_MAX_BYTES: usize = 4096;

/// Sanitize a client-supplied properties object.
///
/// TRUNCATES RATHER THAN REJECTS, deliberately: analytics is fire-and-forget,
/// so a rejected event is an event silently lost. Nested objects and arrays are
/// dropped (flat scalars only) because they cannot be aggregated in a
/// breakdown, which is the entire point of storing properties.
///
/// Returns None when nothing usable survives, so the column stays NULL rather
/// than holding `{}`.
pub fn sanitize_properties(raw: &serde_json::Value) -> Option<String> {
    let obj = raw.as_object()?;
    let mut out = serde_json::Map::new();
    for (key, value) in obj.iter() {
        if out.len() >= PROPS_MAX_KEYS {
            break;
        }
        let key: String = key.chars().take(64).collect();
        let kept = match value {
            serde_json::Value::String(s) => {
                serde_json::Value::String(s.chars().take(PROPS_MAX_VALUE_CHARS).collect())
            }
            serde_json::Value::Number(n) => serde_json::Value::Number(n.clone()),
            serde_json::Value::Bool(b) => serde_json::Value::Bool(*b),
            // Null carries no signal for a breakdown; objects and arrays cannot
            // be grouped by. Drop rather than stringify.
            _ => continue,
        };
        out.insert(key, kept);
    }
    if out.is_empty() {
        return None;
    }
    let mut encoded = serde_json::to_string(&out).ok()?;
    if encoded.len() > PROPS_MAX_BYTES {
        // Shed keys until it fits rather than truncating the JSON into
        // something unparseable.
        let mut keys: Vec<String> = out.keys().cloned().collect();
        while encoded.len() > PROPS_MAX_BYTES {
            let drop = keys.pop()?;
            out.remove(&drop);
            if out.is_empty() {
                return None;
            }
            encoded = serde_json::to_string(&out).ok()?;
        }
    }
    Some(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn real_browsers_are_not_bots() {
        for ua in [
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:127.0) Gecko/20100101 Firefox/127.0",
        ] {
            assert!(!is_bot(Some(ua)), "should be human: {ua}");
        }
    }

    #[test]
    fn crawlers_and_tooling_are_bots() {
        for ua in [
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
            "Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)",
            "curl/8.4.0",
            "python-requests/2.31.0",
            "Mozilla/5.0 (X11; Linux x86_64) HeadlessChrome/126.0.0.0 Safari/537.36",
            "facebookexternalhit/1.1",
            "Mozilla/5.0 (compatible; AhrefsBot/7.0; +http://ahrefs.com/robot/)",
        ] {
            assert!(is_bot(Some(ua)), "should be a bot: {ua}");
        }
    }

    /// Our own verification browser counted itself as a visitor on the first
    /// real install — the case that motivated including "headless".
    #[test]
    fn our_own_headless_verifier_is_excluded() {
        assert!(is_bot(Some(
            "Mozilla/5.0 (Macintosh) AppleWebKit/537.36 HeadlessChrome/126.0 Safari/537.36"
        )));
    }

    #[test]
    fn a_missing_user_agent_counts_as_a_bot() {
        assert!(is_bot(None));
        assert!(is_bot(Some("")));
        assert!(is_bot(Some("   ")));
    }

    #[test]
    fn extracts_only_allowlisted_campaign_params() {
        let utm = extract_utm("/deals?utm_source=newsletter&utm_medium=email&utm_campaign=july&email=a@b.com&token=secret");
        assert_eq!(utm.source.as_deref(), Some("newsletter"));
        assert_eq!(utm.medium.as_deref(), Some("email"));
        assert_eq!(utm.campaign.as_deref(), Some("july"));
        // The PII-bearing params are simply not represented anywhere.
        let encoded = serde_json::to_string(&utm).unwrap();
        assert!(!encoded.contains("a@b.com"), "{encoded}");
        assert!(!encoded.contains("secret"), "{encoded}");
    }

    #[test]
    fn maps_ad_click_ids_to_a_source() {
        let utm = extract_utm("/?gclid=abc123");
        assert_eq!(utm.source.as_deref(), Some("google"));
        assert_eq!(utm.medium.as_deref(), Some("cpc"));
        let fb = extract_utm("/?fbclid=xyz");
        assert_eq!(fb.source.as_deref(), Some("facebook"));
    }

    #[test]
    fn an_explicit_utm_source_beats_a_click_id() {
        let utm = extract_utm("/?utm_source=newsletter&gclid=abc");
        assert_eq!(utm.source.as_deref(), Some("newsletter"));
    }

    #[test]
    fn paths_aggregate_by_stripping_query_and_hash() {
        assert_eq!(normalize_path("/deals?utm_source=x"), "/deals");
        assert_eq!(normalize_path("/deals#top"), "/deals");
        assert_eq!(normalize_path("/deals"), "/deals");
        assert_eq!(normalize_path(""), "/");
    }

    #[test]
    fn classifies_referrers_into_useful_buckets() {
        assert_eq!(classify_referrer(None, None), ReferrerClass::Direct);
        assert_eq!(
            classify_referrer(Some("https://www.google.com/search?q=x"), None),
            ReferrerClass::Search
        );
        assert_eq!(
            classify_referrer(Some("https://t.co/abc"), None),
            ReferrerClass::Social
        );
        assert_eq!(
            classify_referrer(Some("https://someblog.dev/post"), None),
            ReferrerClass::Referral
        );
    }

    /// On an SPA, document.referrer still holds the ORIGINAL external referrer
    /// after client-side navigation, so every internal route change re-reports
    /// it. Self-referrals must be seen as internal or the referrer list inflates.
    #[test]
    fn self_referrals_are_internal_not_referral() {
        assert_eq!(
            classify_referrer(
                Some("https://www.grocerysaver.ca/deals"),
                Some("https://grocerysaver.ca")
            ),
            ReferrerClass::Internal
        );
    }

    #[test]
    fn referrer_host_normalizes() {
        assert_eq!(
            referrer_host("https://www.Example.com/x?y=1").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            referrer_host("http://example.com:8080/").as_deref(),
            Some("example.com")
        );
        assert_eq!(referrer_host(""), None);
    }

    // ── properties ──

    #[test]
    fn keeps_flat_scalars() {
        let props = sanitize_properties(&json!({
            "source": "deals", "deal_id": 12, "sale_price": 4.99, "had_original_price": true
        }))
        .unwrap();
        let back: serde_json::Value = serde_json::from_str(&props).unwrap();
        assert_eq!(back["source"], "deals");
        assert_eq!(back["deal_id"], 12);
        assert_eq!(back["had_original_price"], true);
    }

    #[test]
    fn drops_nested_values_that_cannot_be_grouped_by() {
        let props =
            sanitize_properties(&json!({ "ok": "yes", "nested": {"a": 1}, "list": [1, 2] }))
                .unwrap();
        let back: serde_json::Value = serde_json::from_str(&props).unwrap();
        assert_eq!(back["ok"], "yes");
        assert!(back.get("nested").is_none());
        assert!(back.get("list").is_none());
    }

    #[test]
    fn truncates_rather_than_rejecting() {
        // Analytics is fire-and-forget: a rejected event is a lost event.
        let long = "x".repeat(1000);
        let props = sanitize_properties(&json!({ "big": long })).unwrap();
        let back: serde_json::Value = serde_json::from_str(&props).unwrap();
        assert_eq!(back["big"].as_str().unwrap().len(), PROPS_MAX_VALUE_CHARS);
    }

    #[test]
    fn caps_key_count() {
        let mut obj = serde_json::Map::new();
        for i in 0..100 {
            obj.insert(format!("k{i}"), json!(i));
        }
        let props = sanitize_properties(&serde_json::Value::Object(obj)).unwrap();
        let back: serde_json::Value = serde_json::from_str(&props).unwrap();
        assert_eq!(back.as_object().unwrap().len(), PROPS_MAX_KEYS);
    }

    #[test]
    fn stays_under_the_byte_ceiling_and_remains_parseable() {
        let mut obj = serde_json::Map::new();
        for i in 0..PROPS_MAX_KEYS {
            obj.insert(format!("key{i}"), json!("v".repeat(PROPS_MAX_VALUE_CHARS)));
        }
        let props = sanitize_properties(&serde_json::Value::Object(obj)).unwrap();
        assert!(props.len() <= PROPS_MAX_BYTES, "{} bytes", props.len());
        serde_json::from_str::<serde_json::Value>(&props).expect("must stay valid JSON");
    }

    #[test]
    fn empty_or_non_object_becomes_null() {
        assert!(sanitize_properties(&json!({})).is_none());
        assert!(sanitize_properties(&json!("nope")).is_none());
        assert!(sanitize_properties(&json!(null)).is_none());
        // An object of only nested values keeps nothing.
        assert!(sanitize_properties(&json!({ "a": {"b": 1} })).is_none());
    }
}

/// Minimal percent-decoding for query values (`%20`, `+`). Full URL parsing is
/// unnecessary here and would pull a dependency for three characters.
fn percent_decode(value: &str) -> String {
    let bytes = value.replace('+', " ");
    let bytes = bytes.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
