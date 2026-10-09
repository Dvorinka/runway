//! Non-GitHub git provider clients — port of devpush services
//! `gitea.py`, `gitlab.py`, `bitbucket.py`. Auth: gitea `token`,
//! gitlab `PRIVATE-TOKEN`, bitbucket `Bearer`. GitHub Enterprise is a
//! `base_url` away on the GitHub client — no separate type.

use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::crypto::Crypto;
use crate::error::{Error, Result};

/// Minted Bitbucket Bearer tokens, keyed by consumer key. Client
/// credentials last ~2h; we refresh at 90% of `expires_in`.
static BB_TOKENS: LazyLock<Mutex<HashMap<String, (String, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One user's stored connection to a self-hosted/provider account.
#[derive(Debug, Clone)]
pub struct Connection {
    pub id: i64,
    pub base_url: String,
    pub username: String,
    pub token: String, // decrypted
}

/// Which provider family a `repo_provider` string belongs to.
pub fn provider_kind(p: &str) -> Option<&'static str> {
    match p {
        "gitea" => Some("gitea"),
        "gitlab" => Some("gitlab"),
        "bitbucket" => Some("bitbucket"),
        // GHE reuses the GitHub client with a different base URL.
        "github_enterprise" => Some("github_enterprise"),
        _ => None,
    }
}

/// Load + decrypt a user's gitea/gitlab/bitbucket connection row.
/// `provider` is `gitea` | `gitlab` | `bitbucket`.
pub async fn connection(
    db: &PgPool,
    crypto: &Crypto,
    provider: &str,
    conn_id: i64,
) -> Result<Option<Connection>> {
    let table = match provider {
        "gitea" => "gitea_connection",
        "gitlab" => "gitlab_connection",
        "bitbucket" => "bitbucket_connection",
        _ => return Ok(None),
    };
    // Table name is from our own match — safe. Bitbucket stores
    // `workspace` instead of `base_url`/`username` — same slot reused.
    let cols = if provider == "bitbucket" {
        "id, 'https://api.bitbucket.org' AS base_url, workspace AS username, token"
    } else {
        "id, base_url, username, token"
    };
    let row: Option<(i64, String, String, String)> =
        sqlx::query_as(&format!("SELECT {cols} FROM {table} WHERE id = $1"))
            .bind(conn_id)
            .fetch_optional(db)
            .await?;
    row.map(|(id, base_url, username, token)| {
        Ok::<_, Error>(Connection {
            id,
            base_url,
            username,
            token: crypto.decrypt(&token)?,
        })
    })
    .transpose()
}

/// Thin REST client over one connection.
pub struct Client {
    pub provider: String,
    pub conn: Connection,
    http: reqwest::Client,
}

impl Client {
    pub fn new(provider: &str, conn: Connection) -> Self {
        Self {
            provider: provider.to_string(),
            conn,
            http: reqwest::Client::new(),
        }
    }

    fn api_base(&self) -> String {
        let base = self.conn.base_url.trim_end_matches('/');
        match self.provider.as_str() {
            "gitea" => format!("{base}/api/v1"),
            "gitlab" => format!("{base}/api/v4"),
            "bitbucket" => "https://api.bitbucket.org/2.0".into(),
            _ => base.into(),
        }
    }

    /// Bitbucket accepts two credential shapes: a bare token (repo
    /// access token) used as Bearer, or an OAuth consumer `key:secret`
    /// which we exchange for a short-lived Bearer on demand and cache.
    async fn bitbucket_bearer(&self) -> Result<String> {
        let Some((key, secret)) = self.conn.token.split_once(':') else {
            return Ok(self.conn.token.clone());
        };
        if let Some((tok, exp)) = BB_TOKENS.lock().unwrap().get(key) {
            if Instant::now() < *exp {
                return Ok(tok.clone());
            }
        }
        let res: Value = self
            .http
            .post("https://bitbucket.org/site/oauth2/access_token")
            .basic_auth(key, Some(secret))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let tok = res["access_token"]
            .as_str()
            .ok_or_else(|| Error::BadRequest("bitbucket oauth: no access_token".into()))?
            .to_string();
        let ttl = res["expires_in"].as_u64().unwrap_or(7200);
        BB_TOKENS.lock().unwrap().insert(
            key.to_string(),
            (
                tok.clone(),
                Instant::now() + Duration::from_secs(ttl * 9 / 10),
            ),
        );
        Ok(tok)
    }

    async fn auth(&self, req: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        Ok(match self.provider.as_str() {
            "gitea" => req.header("Authorization", format!("token {}", self.conn.token)),
            "gitlab" => req.header("PRIVATE-TOKEN", &self.conn.token),
            "bitbucket" => req.bearer_auth(self.bitbucket_bearer().await?),
            _ => req.bearer_auth(&self.conn.token),
        })
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}{path}", self.api_base());
        let res = self
            .auth(self.http.get(&url))
            .await?
            .send()
            .await?
            .error_for_status()?;
        Ok(res.json().await?)
    }

    /// `GET /user` — verifies the token and returns the account payload.
    pub async fn verify_user(&self) -> Result<Value> {
        self.get("/user").await
    }

    /// Single repo lookup. Gitea `/repos/{o}/{r}`, gitlab
    /// `/projects/{enc}`, bitbucket `/repositories/{ws}/{slug}`.
    /// `full` is `owner/repo` (gitlab accepts nested groups).
    pub async fn repository(&self, full: &str) -> Result<Value> {
        let (owner, repo) = split_full_name(full)?;
        let path = match self.provider.as_str() {
            "gitea" => format!("/repos/{owner}/{repo}"),
            "gitlab" => format!("/projects/{}", path_encode(full)),
            "bitbucket" => format!("/repositories/{owner}/{repo}"),
            _ => return Err(Error::BadRequest("unsupported provider".into())),
        };
        self.get(&path).await
    }

    /// Root directory file names — for preset detection. Gitea
    /// `/contents` items `{name,type}`; gitlab `tree` items;
    /// bitbucket `src/` `values[].path` basenames of `commit_file`s.
    pub async fn list_root_files(&self, full: &str, branch: &str) -> Result<Vec<String>> {
        let (owner, repo) = split_full_name(full)?;
        let path = match self.provider.as_str() {
            "gitea" => format!("/repos/{owner}/{repo}/contents?ref={branch}"),
            "gitlab" => format!(
                "/projects/{}/repository/tree?ref={branch}&per_page=100",
                path_encode(full)
            ),
            "bitbucket" => {
                format!("/repositories/{owner}/{repo}/src/{branch}/?pagelen=100")
            }
            _ => return Ok(vec![]),
        };
        let v = self.get(&path).await?;
        let items = if self.provider == "bitbucket" {
            v["values"].as_array().cloned().unwrap_or_default()
        } else {
            v.as_array().cloned().unwrap_or_default()
        };
        Ok(items
            .iter()
            .filter_map(|i| {
                let is_file = match self.provider.as_str() {
                    "bitbucket" => i["type"].as_str() == Some("commit_file"),
                    _ => i["type"].as_str() != Some("dir"),
                };
                if !is_file {
                    return None;
                }
                let name = if self.provider == "bitbucket" {
                    i["path"].as_str()?.rsplit('/').next()?
                } else {
                    i["name"].as_str()?
                };
                // Only root-level entries count.
                if name.contains('/') {
                    None
                } else {
                    Some(name.to_string())
                }
            })
            .collect())
    }

    /// Repos visible to the token. Gitea: `/user/repos`; gitlab:
    /// `/projects?membership=true`; bitbucket: `/repositories/{workspace}`.
    pub async fn list_repos(&self) -> Result<Vec<Value>> {
        match self.provider.as_str() {
            "gitea" => Ok(self
                .get("/user/repos?limit=50")
                .await?
                .as_array()
                .cloned()
                .unwrap_or_default()),
            "gitlab" => Ok(self
                .get("/projects?membership=true&per_page=50")
                .await?
                .as_array()
                .cloned()
                .unwrap_or_default()),
            "bitbucket" => Ok(self
                .get(&format!("/repositories/{}", self.conn.username))
                .await?["values"]
                .as_array()
                .cloned()
                .unwrap_or_default()),
            _ => Ok(vec![]),
        }
    }

    /// Branches for `owner/repo` (gitlab takes the path URL-encoded).
    pub async fn list_branches(&self, owner: &str, repo: &str) -> Result<Vec<Value>> {
        let path = match self.provider.as_str() {
            "gitea" => format!("/repos/{owner}/{repo}/branches?limit=100"),
            "gitlab" => format!(
                "/projects/{}/repository/branches?per_page=100",
                path_encode(&format!("{owner}/{repo}"))
            ),
            "bitbucket" => format!("/repositories/{}/{}/refs/branches?pagelen=100", owner, repo),
            _ => return Ok(vec![]),
        };
        let v = self.get(&path).await?;
        Ok(if self.provider == "bitbucket" {
            v["values"].as_array().cloned().unwrap_or_default()
        } else {
            v.as_array().cloned().unwrap_or_default()
        })
    }

    /// Latest commit on a branch, normalized to the same shape the
    /// deploy pipeline consumes (sha/message/author/timestamp).
    pub async fn latest_commit(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<Commit>> {
        let path = match self.provider.as_str() {
            "gitea" => format!("/repos/{owner}/{repo}/commits?sha={branch}&limit=1"),
            "gitlab" => format!(
                "/projects/{}/repository/commits?ref_name={branch}&per_page=1",
                path_encode(&format!("{owner}/{repo}"))
            ),
            "bitbucket" => format!(
                "/repositories/{}/{}/commits/{branch}?pagelen=1",
                owner, repo
            ),
            _ => return Ok(None),
        };
        let v = self.get(&path).await?;
        let arr = if self.provider == "bitbucket" {
            v["values"].as_array().cloned()
        } else {
            v.as_array().cloned()
        };
        let Some(first) = arr.and_then(|a| a.into_iter().next()) else {
            return Ok(None);
        };
        Ok(Some(normalize_commit(&self.provider, &first)))
    }

    /// Raw file contents for detection probes (package.json etc.).
    /// Gitea: `/contents/{path}` `content` is base64; gitlab:
    /// `/repository/files/{enc}/raw`; bitbucket: `/src/{branch}/{path}`.
    pub async fn file_text(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        branch: &str,
    ) -> Result<Option<String>> {
        let res = match self.provider.as_str() {
            "gitea" => {
                let v = self
                    .get(&format!(
                        "/repos/{owner}/{repo}/contents/{path}?ref={branch}"
                    ))
                    .await;
                match v {
                    Ok(v) => {
                        let b64 = v["content"].as_str().unwrap_or("").replace('\n', "");
                        use base64::Engine;
                        base64::engine::general_purpose::STANDARD
                            .decode(b64)
                            .ok()
                            .and_then(|b| String::from_utf8(b).ok())
                    }
                    Err(_) => None,
                }
            }
            "gitlab" => {
                let enc = path_encode(path);
                let proj = path_encode(&format!("{owner}/{repo}"));
                let url = format!(
                    "{}/projects/{proj}/repository/files/{enc}/raw?ref={branch}",
                    self.api_base()
                );
                match self.auth(self.http.get(&url)).await {
                    Ok(req) => match req.send().await {
                        Ok(r) if r.status().is_success() => r.text().await.ok(),
                        _ => None,
                    },
                    Err(_) => None,
                }
            }
            "bitbucket" => {
                let url = format!(
                    "{}/repositories/{owner}/{repo}/src/{branch}/{path}",
                    self.api_base()
                );
                match self.auth(self.http.get(&url)).await {
                    Ok(req) => match req.send().await {
                        Ok(r) if r.status().is_success() => r.text().await.ok(),
                        _ => None,
                    },
                    Err(_) => None,
                }
            }
            _ => None,
        };
        Ok(res)
    }
}

/// GitLab path encoding — repo paths only need `/` escaped for the
/// endpoints we call (owner/repo segments can't contain other reserveds).
fn path_encode(s: &str) -> String {
    s.replace('/', "%2F")
}

/// `owner/repo` → `(owner, repo)`; owner may contain `/` for gitlab
/// groups (the split is on the last slash).
fn split_full_name(full: &str) -> Result<(&str, &str)> {
    let (owner, repo) = full
        .rsplit_once('/')
        .ok_or_else(|| Error::BadRequest(format!("invalid repo full name '{full}'")))?;
    if owner.is_empty() || repo.is_empty() {
        return Err(Error::BadRequest(format!(
            "invalid repo full name '{full}'"
        )));
    }
    Ok((owner, repo))
}

/// `X-Gitea-Signature` check — plain hex HMAC-SHA256 of the raw body
/// (no `sha256=` prefix; devpush `_verify_gitea_webhook`).
pub fn verify_gitea_signature(secret: &str, payload: &[u8], signature: &str) -> bool {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let Ok(expected) = hex::decode(signature) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(payload);
    mac.verify_slice(&expected).is_ok()
}

/// `X-Gitlab-Token` check — shared-secret equality, constant-time.
pub fn verify_gitlab_token(secret: &str, token: &str) -> bool {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    // HMAC both sides to compare fixed-size digests.
    let Ok(mut a) = HmacSha256::new_from_slice(b"runway-gitlab") else {
        return false;
    };
    a.update(secret.as_bytes());
    let Ok(mut b) = HmacSha256::new_from_slice(b"runway-gitlab") else {
        return false;
    };
    b.update(token.as_bytes());
    a.finalize().into_bytes() == b.finalize().into_bytes()
}

/// Normalized commit — what `deploy::create` stores in `meta`.
#[derive(Debug, Clone)]
pub struct Commit {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub timestamp: Option<String>,
}

fn normalize_commit(provider: &str, c: &Value) -> Commit {
    match provider {
        "gitlab" => Commit {
            sha: c["id"].as_str().unwrap_or_default().into(),
            message: c["message"].as_str().unwrap_or_default().into(),
            author: c["author_name"].as_str().unwrap_or_default().into(),
            timestamp: c["created_at"].as_str().map(String::from),
        },
        "bitbucket" => Commit {
            sha: c["hash"].as_str().unwrap_or_default().into(),
            message: c["message"].as_str().unwrap_or_default().into(),
            author: c["author"]["user"]["nickname"]
                .as_str()
                .or_else(|| c["author"]["raw"].as_str())
                .unwrap_or_default()
                .into(),
            timestamp: c["date"].as_str().map(String::from),
        },
        _ => Commit {
            // gitea matches the GitHub shape already
            sha: c["sha"].as_str().unwrap_or_default().into(),
            message: c["commit"]["message"].as_str().unwrap_or_default().into(),
            author: c["commit"]["author"]["name"]
                .as_str()
                .or_else(|| c["author"]["login"].as_str())
                .unwrap_or_default()
                .into(),
            timestamp: c["commit"]["author"]["date"].as_str().map(String::from),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitea_signature_roundtrip() {
        use hmac::{Hmac, Mac};
        let payload = br#"{"ref":"refs/heads/main"}"#;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"sec").unwrap();
        mac.update(payload);
        let sig = hex::encode(mac.finalize().into_bytes());
        assert!(verify_gitea_signature("sec", payload, &sig));
        assert!(!verify_gitea_signature("sec", payload, "beef"));
        assert!(!verify_gitea_signature("other", payload, &sig));
        assert!(!verify_gitea_signature("sec", payload, "not-hex!!"));
    }

    #[test]
    fn gitlab_token_constant_time() {
        assert!(verify_gitlab_token("s3cret", "s3cret"));
        assert!(!verify_gitlab_token("s3cret", "nope"));
        assert!(!verify_gitlab_token("s3cret", ""));
    }

    #[test]
    fn split_full_name_edges() {
        assert_eq!(split_full_name("o/r").unwrap(), ("o", "r"));
        assert_eq!(split_full_name("grp/sub/r").unwrap(), ("grp/sub", "r"));
        assert!(split_full_name("noslash").is_err());
        assert!(split_full_name("/r").is_err());
        assert!(split_full_name("o/").is_err());
    }

    #[test]
    fn normalize_commit_shapes() {
        let gl = normalize_commit(
            "gitlab",
            &serde_json::json!({"id":"s","message":"m","author_name":"a",
                                "created_at":"t"}),
        );
        assert_eq!((gl.sha.as_str(), gl.author.as_str()), ("s", "a"));
        let bb = normalize_commit(
            "bitbucket",
            &serde_json::json!({"hash":"s","message":"m",
                                "author":{"user":{"nickname":"a"}},"date":"t"}),
        );
        assert_eq!((bb.sha.as_str(), bb.author.as_str()), ("s", "a"));
        let gt = normalize_commit(
            "gitea",
            &serde_json::json!({"sha":"s",
                                "commit":{"message":"m","author":{"name":"a","date":"t"}}}),
        );
        assert_eq!((gt.sha.as_str(), gt.author.as_str()), ("s", "a"));
    }
}
