//! Optional KERN build executor (`project.config.build_executor = "kern"`).
//!
//! Generates a GitHub-Actions-shaped workflow, dispatches it to a KERN
//! control plane via `POST /api/v1/projects/{project}/runs` (inline
//! `workflow_yaml`), polls to conclusion, and downloads the uploaded
//! `runway-out` artifact. Static-mode only — the artifact lands in the
//! deployment's local static dir and the normal SWS serve phase takes over.

use crate::error::{Error, Result};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

pub struct KernClient {
    base: String,
    token: String,
    pub project: String,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
pub struct Run {
    pub id: String,
    pub status: Option<String>,
    pub conclusion: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub name: String,
}

impl KernClient {
    pub fn new(base: &str, token: &str, project: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            project: project.to_string(),
            http: reqwest::Client::new(),
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
    }

    /// Upload a source bundle (tar.gz) content-addressed — resolves the
    /// digest first and skips the PUT on a hit. Returns the `bundle_ref`
    /// the run trigger expects; a bundle replaces the repo checkout.
    pub async fn upload_source(&self, body: &[u8]) -> Result<String> {
        let digest = hex::encode(Sha256::digest(body));
        let path = format!(
            "/api/v1/projects/{}/source/resolve?sha256={digest}",
            self.project
        );
        let resp = self.req(reqwest::Method::GET, &path).send().await?;
        if resp.status().is_success() {
            let v: serde_json::Value = resp.json().await?;
            if let Some(r) = v.get("bundle_ref").and_then(|r| r.as_str()) {
                return Ok(r.to_string());
            }
        }
        let resp = self
            .req(
                reqwest::Method::PUT,
                &format!("/api/v1/projects/{}/source?sha256={digest}", self.project),
            )
            .header("content-type", "application/gzip")
            .body(body.to_vec())
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Validation(format!(
                "kern source upload failed ({status}): {}",
                &body[..body.len().min(300)]
            )));
        }
        let v: serde_json::Value = resp.json().await?;
        v.get("bundle_ref")
            .and_then(|r| r.as_str())
            .map(str::to_string)
            .ok_or_else(|| Error::Validation("kern source: no bundle_ref".into()))
    }

    pub async fn dispatch(
        &self,
        workflow_yaml: &str,
        git_ref: &str,
        sha: &str,
        bundle_ref: &str,
    ) -> Result<String> {
        let resp = self
            .req(
                reqwest::Method::POST,
                &format!("/api/v1/projects/{}/runs", self.project),
            )
            .json(&json!({
                "workflow_path": ".runway/build.yml",
                "workflow_name": "runway-build",
                "workflow_yaml": workflow_yaml,
                "ref": git_ref,
                "sha": sha,
                "event": "workflow_dispatch",
                "trust_level": "trusted",
                "bundle_ref": bundle_ref,
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Validation(format!(
                "kern dispatch failed ({status}): {}",
                &body[..body.len().min(300)]
            )));
        }
        let v: serde_json::Value = resp.json().await?;
        v.get("run")
            .and_then(|r| r.get("id"))
            .or_else(|| v.get("id"))
            .or_else(|| v.get("run_id"))
            .and_then(|i| i.as_str())
            .map(str::to_string)
            .ok_or_else(|| Error::Validation("kern dispatch: no run id in response".into()))
    }

    pub async fn run(&self, run_id: &str) -> Result<Run> {
        let v: serde_json::Value = self
            .req(reqwest::Method::GET, &format!("/api/v1/runs/{run_id}"))
            .send()
            .await?
            .json()
            .await?;
        let r = v.get("run").unwrap_or(&v);
        Ok(Run {
            id: r
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or(run_id)
                .to_string(),
            status: r.get("status").and_then(|s| s.as_str()).map(Into::into),
            conclusion: r.get("conclusion").and_then(|s| s.as_str()).map(Into::into),
        })
    }

    pub async fn cancel(&self, run_id: &str) -> Result<()> {
        self.req(
            reqwest::Method::POST,
            &format!("/api/v1/runs/{run_id}/cancel"),
        )
        .send()
        .await?;
        Ok(())
    }

    /// Artifacts for a run, newest first.
    pub async fn artifacts(&self, run_id: &str) -> Result<Vec<Artifact>> {
        let v: serde_json::Value = self
            .req(reqwest::Method::GET, "/api/v1/artifacts")
            .query(&[("run_id", run_id)])
            .send()
            .await?
            .json()
            .await?;
        let list = v
            .get("artifacts")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(list
            .iter()
            .filter_map(|a| {
                Some(Artifact {
                    id: a.get("id")?.as_str()?.to_string(),
                    name: a.get("name")?.as_str()?.to_string(),
                })
            })
            .collect())
    }

    /// Raw gzip bytes of an artifact archive.
    pub async fn download(&self, artifact_id: &str) -> Result<Vec<u8>> {
        let resp = self
            .req(
                reqwest::Method::GET,
                &format!("/api/v1/artifacts/{artifact_id}/download"),
            )
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Validation(format!(
                "kern download failed ({})",
                resp.status()
            )));
        }
        Ok(resp.bytes().await?.to_vec())
    }
}

/// Generate the workflow. `build_script` is the same `commands.join(" && ")`
/// the local runner executes, with the container-absolute paths rewritten
/// to workspace-relative ones by the caller.
pub fn workflow_yaml(
    build_script: &str,
    image: &str,
    labels: &[String],
    env: &[(String, String)],
) -> String {
    let mut y = String::from(
        "name: runway-build\non: [workflow_dispatch]\njobs:\n  build:\n    runs-on:\n",
    );
    for l in labels {
        y.push_str(&format!("      - {}\n", yaml_scalar(l)));
    }
    y.push_str(&format!(
        "    container:\n      image: {}\n",
        yaml_scalar(image)
    ));
    if !env.is_empty() {
        y.push_str("    env:\n");
        for (k, v) in env {
            y.push_str(&format!("      {}: {}\n", k, yaml_quote(v)));
        }
    }
    y.push_str("    steps:\n      - name: Build\n        run: |\n");
    for line in build_script.lines() {
        y.push_str(&format!("          {line}\n"));
    }
    // `runway-out/**` — a bare dir path globs only the dir entry itself;
    // `**` (doublestar) descends and includes dotfiles (.runway.json).
    y.push_str(
        "      - name: Upload output\n        uses: actions/upload-artifact@v4\n        with:\n          name: runway-out\n          path: runway-out/**\n          if-no-files-found: error\n",
    );
    y
}

fn yaml_scalar(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-/:".contains(c))
    {
        s.to_string()
    } else {
        yaml_quote(s)
    }
}

fn yaml_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_shape() {
        let y = workflow_yaml(
            "cd app && npm run build",
            "node:22-bookworm",
            &["self-hosted".into(), "linux".into()],
            &[("SOURCE_URL".into(), "https://x.test/s?t=1".into())],
        );
        assert!(y.contains("runs-on:\n      - self-hosted\n      - linux\n"));
        assert!(y.contains("image: node:22-bookworm"));
        assert!(y.contains("SOURCE_URL: 'https://x.test/s?t=1'"));
        assert!(y.contains("          cd app && npm run build\n"));
        assert!(y.contains("name: runway-out"));
    }

    #[test]
    fn yaml_escapes_quotes() {
        assert_eq!(yaml_quote("a'b"), "'a''b'");
        assert_eq!(yaml_scalar("node:20-bookworm"), "node:20-bookworm");
    }
}
