//! Sign-up allowlist — port of devpush `utils/access.py`.
//!
//! `allowlist` rows: `type` ∈ email|domain|pattern, `value` the match.
//! Empty table = open sign-up (allow all). Otherwise an email must match
//! at least one rule. devpush caches rules in-process; login paths are
//! cold enough that we query each time — correctness over cache.

use sqlx::PgPool;

use crate::error::Result;

/// True when `email` may sign up. Exact email > domain > regex pattern
/// (case-insensitive search, devpush parity). Malformed emails deny.
pub async fn is_email_allowed(db: &PgPool, email: &str) -> Result<bool> {
    let rules: Vec<(String, String)> = sqlx::query_as("SELECT type, value FROM allowlist")
        .fetch_all(db)
        .await?;
    if rules.is_empty() {
        return Ok(true);
    }

    let email = email.trim().to_lowercase();
    let Some((_, domain)) = email.split_once('@') else {
        return Ok(false);
    };
    if domain.is_empty() {
        return Ok(false);
    }

    for (ty, value) in &rules {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let hit = match ty.as_str() {
            "email" => value.eq_ignore_ascii_case(&email),
            "domain" => value.eq_ignore_ascii_case(domain),
            // Invalid patterns are ignored (devpush swallows re.error).
            "pattern" => regex::RegexBuilder::new(value)
                .case_insensitive(true)
                .build()
                .map(|re| re.is_match(&email))
                .unwrap_or(false),
            _ => false,
        };
        if hit {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    // is_email_allowed needs a DB; the pure matcher is tested via a
    // small harness that mirrors the rule loop.
    fn allowed(rules: &[(&str, &str)], email: &str) -> bool {
        if rules.is_empty() {
            return true;
        }
        let email = email.trim().to_lowercase();
        let Some((_, domain)) = email.split_once('@') else {
            return false;
        };
        if domain.is_empty() {
            return false;
        }
        rules.iter().any(|(ty, value)| {
            let value = value.trim();
            if value.is_empty() {
                return false;
            }
            match *ty {
                "email" => value.eq_ignore_ascii_case(&email),
                "domain" => value.eq_ignore_ascii_case(domain),
                "pattern" => regex::RegexBuilder::new(value)
                    .case_insensitive(true)
                    .build()
                    .map(|re| re.is_match(&email))
                    .unwrap_or(false),
                _ => false,
            }
        })
    }

    #[test]
    fn empty_allowlist_allows_all() {
        assert!(allowed(&[], "anyone@anywhere.com"));
        assert!(allowed(&[], "not-an-email"));
    }

    #[test]
    fn email_and_domain_rules() {
        let rules = [("email", "a@x.com"), ("domain", "corp.example")];
        assert!(allowed(&rules, "A@x.com")); // case-insensitive
        assert!(allowed(&rules, "bob@corp.example"));
        assert!(!allowed(&rules, "mallory@evil.example"));
    }

    #[test]
    fn pattern_rules_search() {
        let rules = [("pattern", "^dev\\d+@")];
        assert!(allowed(&rules, "dev7@anything.io"));
        assert!(!allowed(&rules, "prod@anything.io"));
        // Invalid pattern is ignored, not an error.
        let rules = [("pattern", "[unclosed")];
        assert!(!allowed(&rules, "a@b.c"));
    }

    #[test]
    fn malformed_emails_deny_when_rules_exist() {
        let rules = [("domain", "x.com")];
        assert!(!allowed(&rules, "no-at-sign"));
        assert!(!allowed(&rules, "trailing@"));
    }
}
