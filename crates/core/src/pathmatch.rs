//! Minimal gitignore-style path matcher for `deployment_rules.paths`.
//! `**` crosses directory separators, `*` and `?` stay within a segment.

/// Match `pattern` against `path` (`/`-separated).
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let seg: Vec<&str> = path.split('/').collect();
    match_segments(&pat, &seg)
}

fn match_segments(pat: &[&str], seg: &[&str]) -> bool {
    if pat.is_empty() {
        return seg.is_empty();
    }
    if pat[0] == "**" {
        // `**` consumes zero or more segments.
        return (0..=seg.len()).any(|n| match_segments(&pat[1..], &seg[n..]));
    }
    if seg.is_empty() || !match_piece(pat[0], seg[0]) {
        return false;
    }
    match_segments(&pat[1..], &seg[1..])
}

/// Single-segment match — `*` any chars, `?` one char, literal otherwise.
fn match_piece(pat: &str, s: &str) -> bool {
    fn inner(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => (0..=s.len()).any(|n| inner(&p[1..], &s[n..])),
            (Some(b'?'), Some(_)) => inner(&p[1..], &s[1..]),
            (Some(&c), Some(&d)) => c == d && inner(&p[1..], &s[1..]),
            _ => false,
        }
    }
    inner(pat.as_bytes(), s.as_bytes())
}

/// Changed-paths gate: `deployment_rules.paths` is a comma-separated
/// glob list. Deploy when at least one changed file matches; when the
/// webhook carries no file list (`changed` empty), fail open.
pub fn deployable(rules_paths: Option<&str>, changed: &[String]) -> bool {
    let Some(paths) = rules_paths.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    if changed.is_empty() {
        return true;
    }
    changed.iter().any(|f| {
        paths
            .split(',')
            .map(str::trim)
            .any(|p| !p.is_empty() && glob_match(p, f))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matching() {
        assert!(glob_match("apps/web/**", "apps/web/src/main.ts"));
        assert!(glob_match("apps/web/**", "apps/web"));
        assert!(!glob_match("apps/web/**", "apps/api/main.ts"));
        assert!(glob_match("*.md", "README.md"));
        assert!(!glob_match("*.md", "docs/a.md"));
        assert!(glob_match("docs/*.md", "docs/a.md"));
        assert!(glob_match("src/?ain.rs", "src/main.rs"));
    }

    #[test]
    fn deployable_rules() {
        // No rule → always deploy; empty changed list → fail open.
        assert!(deployable(None, &[]));
        assert!(deployable(Some("apps/**"), &[]));
        let touched = vec!["docs/guide.md".to_string()];
        assert!(!deployable(Some("apps/**"), &touched));
        let touched = vec!["docs/guide.md".to_string(), "apps/web/x.ts".to_string()];
        assert!(deployable(Some("apps/**, shared/**"), &touched));
    }
}
