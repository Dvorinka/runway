//! Open Graph / Twitter meta extraction — powers the deployment OG
//! inspector. Dependency-free: scans `<meta>` and `<title>` tags in the
//! head region. Good enough for tag previews; not a full HTML parser.

use std::collections::BTreeMap;

#[derive(Debug, Default, serde::Serialize)]
pub struct OgReport {
    pub title: Option<String>,
    pub description: Option<String>,
    pub og: BTreeMap<String, String>,
    pub twitter: BTreeMap<String, String>,
    pub favicon: Option<String>,
}

/// Parse OG/twitter/meta tags out of an HTML document.
pub fn parse(html: &str) -> OgReport {
    let head_end = html
        .to_ascii_lowercase()
        .find("</head>")
        .unwrap_or_else(|| html.len().min(64 * 1024));
    let head = &html[..head_end.min(html.len())];
    let mut r = OgReport::default();
    let mut rest = head;
    while let Some(i) = rest.find('<') {
        rest = &rest[i..];
        let tag_end = match rest.find('>') {
            Some(e) => e,
            None => break,
        };
        let tag = &rest[1..tag_end];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("meta") {
            let attrs = attrs_of(&tag[4..]);
            let key = attrs
                .get("property")
                .or_else(|| attrs.get("name"))
                .cloned()
                .unwrap_or_default();
            let content = attrs.get("content").cloned().unwrap_or_default();
            if let Some(og) = key.strip_prefix("og:") {
                r.og.insert(og.to_string(), content);
            } else if let Some(tw) = key.strip_prefix("twitter:") {
                r.twitter.insert(tw.to_string(), content);
            } else if key == "description" {
                r.description = Some(content);
            }
        } else if lower.starts_with("title") {
            if let Some(close) = rest[tag_end + 1..].to_ascii_lowercase().find("</title") {
                let t = rest[tag_end + 1..tag_end + 1 + close].trim().to_string();
                if !t.is_empty() {
                    r.title = Some(t);
                }
            }
        } else if lower.starts_with("link") {
            let attrs = attrs_of(&tag[4..]);
            let rel = attrs.get("rel").cloned().unwrap_or_default();
            if (rel == "icon" || rel == "shortcut icon") && r.favicon.is_none() {
                r.favicon = attrs.get("href").cloned();
            }
        }
        rest = &rest[tag_end + 1..];
    }
    r
}

/// Scan `key="v"` / `key='v'` / `key=v` attributes into a map. Keys are
/// lowercased; values keep their case.
fn attrs_of(tag: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let kstart = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() && b[i] != b'>' {
            i += 1;
        }
        let key = tag[kstart..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                i += 1;
                let vstart = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                val = tag[vstart..i].to_string();
                i += 1;
            } else {
                let vstart = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                val = tag[vstart..i].to_string();
            }
        }
        if !key.is_empty() {
            out.insert(key, val);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_og_twitter_and_title() {
        let html = r#"<html><head>
            <title>demo app</title>
            <meta name="description" content="A test page.">
            <meta property="og:title" content="Demo">
            <meta property="og:image" content="/og.png">
            <meta property='og:type' content=website>
            <meta name="twitter:card" content="summary_large_image">
            <link rel="icon" href="/favicon.svg">
            </head><body>hi</body></html>"#;
        let r = parse(html);
        assert_eq!(r.title.as_deref(), Some("demo app"));
        assert_eq!(r.description.as_deref(), Some("A test page."));
        assert_eq!(r.og.get("title").map(String::as_str), Some("Demo"));
        assert_eq!(r.og.get("image").map(String::as_str), Some("/og.png"));
        assert_eq!(r.og.get("type").map(String::as_str), Some("website"));
        assert_eq!(
            r.twitter.get("card").map(String::as_str),
            Some("summary_large_image")
        );
        assert_eq!(r.favicon.as_deref(), Some("/favicon.svg"));
    }

    #[test]
    fn tolerates_missing_tags() {
        let r = parse("<html><body>nothing</body></html>");
        assert!(r.title.is_none() && r.og.is_empty() && r.twitter.is_empty());
    }
}
