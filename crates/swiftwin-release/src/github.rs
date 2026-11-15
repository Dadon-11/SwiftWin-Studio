//! Minimal GitHub REST client for cloud releases: repository secrets
//! (libsodium sealed boxes, as GitHub requires), Git Data API commits
//! (no local git needed) and workflow-run polling.
//! Docs: https://docs.github.com/en/rest (API version 2022-11-28).

use std::time::Duration;

use serde_json::{json, Value};
use swiftwin_core::{base64_bytes, Result, SwiftWinError};

pub const API: &str = "https://api.github.com";

pub struct GitHub {
    base: String,
    token: String,
    pub owner: String,
    pub repo: String,
    agent: ureq::Agent,
}

/// Encrypt a secret for GitHub Actions: crypto_box_seal with the repo public key.
pub fn seal(public_key_b64: &str, plaintext: &[u8]) -> Result<String> {
    let pk = base64_bytes::decode(public_key_b64)
        .filter(|k| k.len() == 32)
        .ok_or_else(|| SwiftWinError::protocol("GitHub returned an invalid public key"))?;
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&pk);
    let pk = crypto_box::PublicKey::from(arr);
    let sealed = pk.seal(&mut rand::rngs::OsRng, plaintext).map_err(|e| SwiftWinError::other(format!("seal: {e}")))?;
    Ok(base64_bytes::encode(&sealed))
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunInfo {
    pub id: u64,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: String,
}

impl GitHub {
    pub fn new(repo_full: &str, token: &str) -> Result<Self> {
        Self::with_base(API, repo_full, token)
    }

    pub fn with_base(base: &str, repo_full: &str, token: &str) -> Result<Self> {
        let (owner, repo) = repo_full
            .split_once('/')
            .filter(|(o, r)| !o.is_empty() && !r.is_empty())
            .ok_or_else(|| SwiftWinError::invalid(format!("repository must be owner/name, got {repo_full:?}")))?;
        Ok(GitHub {
            base: base.trim_end_matches('/').into(),
            token: token.into(),
            owner: owner.into(),
            repo: repo.into(),
            agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build(),
        })
    }

    /// A client for account-level calls (`/user`, `/user/repos`): no repository yet.
    pub fn account(base: &str, token: &str) -> Self {
        GitHub {
            base: base.trim_end_matches('/').into(),
            token: token.into(),
            owner: String::new(),
            repo: String::new(),
            agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build(),
        }
    }

    /// Call a repository endpoint: `path` is relative to `/repos/<owner>/<repo>`.
    pub fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        self.send(method, &format!("/repos/{}/{}{}", self.owner, self.repo, path), body)
    }

    /// Call any endpoint: `path` is relative to the API root (e.g. `/user`).
    pub fn send(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}{}", self.base, path);
        let req = self
            .agent
            .request(method, &url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .set("User-Agent", "swiftwin");
        let res = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match res {
            Ok(r) if r.status() == 204 => Ok(Value::Null),
            Ok(r) => {
                let text = r.into_string().map_err(|e| SwiftWinError::Http(e.to_string()))?;
                if text.trim().is_empty() {
                    Ok(Value::Null)
                } else {
                    serde_json::from_str(&text).map_err(|e| SwiftWinError::Http(format!("GitHub: bad JSON ({e})")))
                }
            }
            Err(ureq::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                let m = v["message"].as_str().unwrap_or("").to_string();
                Err(match code {
                    401 => SwiftWinError::Auth(format!("GitHub rejected the token: {m}")),
                    403 | 404 if !self.repo.is_empty() => SwiftWinError::Auth(format!(
                        "GitHub {method} {path}: HTTP {code} {m} (does the token have `repo` and `workflow` access to {}/{}?)",
                        self.owner, self.repo
                    )),
                    403 | 404 => SwiftWinError::Auth(format!("GitHub {method} {path}: HTTP {code} {m}")),
                    422 => SwiftWinError::InvalidInput(format!("GitHub {method} {path}: {m} {}", errors_text(&v))),
                    409 if m.to_lowercase().contains("empty") => SwiftWinError::invalid(format!(
                        "{}/{} is empty: create it with a README first",
                        self.owner, self.repo
                    )),
                    _ => SwiftWinError::Http(format!("GitHub {method} {path}: HTTP {code} {m}")),
                })
            }
            Err(e) => Err(SwiftWinError::Http(format!("GitHub unreachable: {e}"))),
        }
    }

    /// The status code of a failed call, for callers that treat some codes specially.
    pub fn status_of(e: &SwiftWinError) -> Option<u16> {
        let s = e.to_string();
        s.split("HTTP ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|c| c.parse().ok())
    }

    pub fn set_secret(&self, name: &str, value: &[u8]) -> Result<()> {
        let k = self.call("GET", "/actions/secrets/public-key", None)?;
        let key_id = k["key_id"].as_str().ok_or_else(|| SwiftWinError::protocol("no key_id"))?;
        let enc = seal(k["key"].as_str().unwrap_or(""), value)?;
        self.call("PUT", &format!("/actions/secrets/{name}"), Some(json!({"encrypted_value": enc, "key_id": key_id})))?;
        Ok(())
    }

    /// Commit `files` (path, bytes) as the whole tree of `branch` (force-updating it). Returns the commit sha.
    pub fn commit_files(&self, branch: &str, message: &str, files: &[(String, Vec<u8>)]) -> Result<String> {
        let mut tree = Vec::new();
        for (path, data) in files {
            let b = self.call("POST", "/git/blobs", Some(json!({"content": base64_bytes::encode(data), "encoding": "base64"})))?;
            let sha = b["sha"].as_str().ok_or_else(|| SwiftWinError::protocol("blob has no sha"))?;
            tree.push(json!({"path": path, "mode": "100644", "type": "blob", "sha": sha}));
        }
        let t = self.call("POST", "/git/trees", Some(json!({"tree": tree})))?;
        let tree_sha = t["sha"].as_str().ok_or_else(|| SwiftWinError::protocol("tree has no sha"))?;
        let parent = self
            .call("GET", &format!("/git/ref/heads/{branch}"), None)
            .ok()
            .and_then(|r| r["object"]["sha"].as_str().map(str::to_string));
        let parents: Vec<String> = parent.iter().cloned().collect();
        let c = self.call("POST", "/git/commits", Some(json!({"message": message, "tree": tree_sha, "parents": parents})))?;
        let sha = c["sha"].as_str().ok_or_else(|| SwiftWinError::protocol("commit has no sha"))?.to_string();
        if parent.is_some() {
            self.call("PATCH", &format!("/git/refs/heads/{branch}"), Some(json!({"sha": sha, "force": true})))?;
        } else {
            self.call("POST", "/git/refs", Some(json!({"ref": format!("refs/heads/{branch}"), "sha": sha})))?;
        }
        Ok(sha)
    }

    pub fn run_for_commit(&self, sha: &str) -> Result<Option<RunInfo>> {
        let v = self.call("GET", &format!("/actions/runs?head_sha={sha}&per_page=5"), None)?;
        Ok(v["workflow_runs"].as_array().and_then(|a| a.first()).map(|r| RunInfo {
            id: r["id"].as_u64().unwrap_or(0),
            status: r["status"].as_str().unwrap_or("").into(),
            conclusion: r["conclusion"].as_str().map(str::to_string),
            url: r["html_url"].as_str().unwrap_or("").into(),
        }))
    }
}

/// GitHub's 422 bodies list the reasons under `errors[].message` / `errors[].code`.
fn errors_text(v: &Value) -> String {
    v["errors"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e["message"].as_str().or_else(|| e["code"].as_str())).collect::<Vec<_>>().join("; "))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_box_roundtrip() {
        let sk = crypto_box::SecretKey::generate(&mut rand::rngs::OsRng);
        let pk_b64 = base64_bytes::encode(sk.public_key().as_bytes());
        let sealed = seal(&pk_b64, b"top secret").unwrap();
        let opened = sk.unseal(&base64_bytes::decode(&sealed).unwrap()).unwrap();
        assert_eq!(opened, b"top secret");
        assert!(seal("AAAA", b"x").is_err());
        assert!(GitHub::new("nope", "t").is_err());
    }
}
