//! Slug generation and token helpers, ported from devpush models.py.

use rand::Rng;

/// Random hex token of `n` bytes → 2n chars (token_hex(16) = 32 chars).
pub fn token_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::thread_rng().fill(&mut bytes[..]);
    hex::encode(bytes)
}

/// Lowercase slugify: spaces/underscores/dots → '-', strip [^a-z0-9-],
/// collapse repeats, trim, cap at `max`.
pub fn slugify(input: &str, max: usize) -> String {
    let mut out = String::with_capacity(input.len().min(max));
    let mut last_dash = false;
    for c in input.to_lowercase().chars() {
        let c = match c {
            ' ' | '_' | '.' => '-',
            c => c,
        };
        if !c.is_ascii_alphanumeric() && c != '-' {
            continue;
        }
        if c == '-' {
            if last_dash {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        out.push(c);
        if out.len() >= max {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

/// Sanitize a branch name for use in a DNS subdomain.
/// `feat/foo-bar` → `feat-foo-bar` (port of get_alias_domains).
pub fn branch_slug(branch: &str) -> String {
    branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_hex_length() {
        assert_eq!(token_hex(16).len(), 32);
        assert!(token_hex(8).chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn slugify_cases() {
        assert_eq!(slugify("My Cool_App 2.0", 63), "my-cool-app-2-0");
        assert_eq!(slugify("--weird--", 63), "weird");
        assert_eq!(slugify("", 63), "");
        assert_eq!(slugify("aaaaaaaaaaaaaaaaaaaa", 5), "aaaaa");
    }

    #[test]
    fn branch_slug_cases() {
        assert_eq!(branch_slug("feat/foo-bar"), "feat-foo-bar");
        assert_eq!(branch_slug("Main"), "main");
        assert_eq!(branch_slug("release/v1.2.3"), "release-v1-2-3");
    }
}
