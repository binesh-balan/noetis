//! Keyless access to Azure AI Foundry with Entra ID (work account) sign-in.
//!
//! Desktop OAuth 2.0 authorization-code flow with PKCE and a loopback redirect
//! (RFC 8252): the system browser shows the Microsoft sign-in page (single sign-on on
//! Entra-joined PCs), the app receives the code on http://localhost:<random port>, and
//! exchanges it for a short-lived access token for the Foundry resource. No client secret
//! and no API key exist on the device.
//!
//! The access token is kept in memory only. The refresh token is stored in the app data
//! folder, encrypted with the current Windows user's DPAPI key (`secure_storage`), so later
//! summaries don't prompt again; signing out deletes it.

use anyhow::{anyhow, bail, Context, Result};
use once_cell::sync::Lazy;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

const DEFAULT_AUTHORITY: &str = "https://login.microsoftonline.com";
const DEFAULT_SCOPE: &str = "https://cognitiveservices.azure.com/.default";
const SESSION_FILE: &str = "entra-session.json";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
/// Refresh the access token this long before it expires.
const EXPIRY_MARGIN: Duration = Duration::from_secs(300);

/// `entra` section of the managed policy.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntraConfig {
    /// Directory (tenant) ID.
    pub tenant_id: String,
    /// Application (client) ID of the app registration (public client, no secret).
    pub client_id: String,
    /// Resource scope; defaults to Azure AI services (Cognitive Services).
    pub scope: Option<String>,
    /// Sign-in host; defaults to the Azure public cloud. Override for sovereign clouds.
    pub authority: Option<String>,
}

impl EntraConfig {
    fn endpoint(&self, path: &str) -> String {
        let host = self.authority.as_deref().unwrap_or(DEFAULT_AUTHORITY).trim_end_matches('/');
        format!("{host}/{}/oauth2/v2.0/{path}", self.tenant_id)
    }

    fn scopes(&self) -> String {
        format!("{} openid profile offline_access", self.scope.as_deref().unwrap_or(DEFAULT_SCOPE))
    }
}

struct Session {
    access_token: String,
    expires_at: Instant,
}

/// In-memory access token. The mutex also serialises sign-in so two summaries never open
/// two browser windows.
static SESSION: Lazy<Mutex<Option<Session>>> = Lazy::new(|| Mutex::new(None));

#[derive(Serialize, Deserialize)]
struct StoredSession {
    /// DPAPI-protected refresh token (`secure_storage::protect`).
    refresh_token: String,
    account: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url without padding.
pub(crate) fn b64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 2);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(B64URL[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let vals: Vec<u32> = s
        .trim_end_matches('=')
        .bytes()
        .map(|c| B64URL.iter().position(|&x| x == c).map(|p| p as u32))
        .collect::<Option<_>>()?;
    let mut out = Vec::with_capacity(vals.len() * 3 / 4);
    for chunk in vals.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, v)| acc | v << (18 - 6 * i));
        for i in 0..chunk.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

fn random_token(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len).map(|_| B64URL[rng.gen_range(0..64)] as char).collect()
}

/// PKCE S256 challenge for a verifier (RFC 7636).
pub(crate) fn pkce_challenge(verifier: &str) -> String {
    b64url(&Sha256::digest(verifier.as_bytes()))
}

fn authorize_url(cfg: &EntraConfig, redirect_uri: &str, state: &str, challenge: &str) -> String {
    let mut url = url::Url::parse(&cfg.endpoint("authorize")).expect("valid authority URL");
    url.query_pairs_mut()
        .append_pair("client_id", &cfg.client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_mode", "query")
        .append_pair("scope", &cfg.scopes())
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    url.to_string()
}

/// The authorization code (and state) from the browser's redirect request line, e.g.
/// `GET /?code=...&state=... HTTP/1.1`.
fn parse_redirect(request_line: &str) -> Result<(String, String)> {
    let target = request_line.split_whitespace().nth(1).ok_or_else(|| anyhow!("malformed redirect"))?;
    let url = url::Url::parse(&format!("http://localhost{target}"))?;
    let get = |k: &str| url.query_pairs().find(|(key, _)| key == k).map(|(_, v)| v.into_owned());
    if let Some(error) = get("error") {
        bail!("Sign-in failed: {} {}", error, get("error_description").unwrap_or_default());
    }
    Ok((get("code").ok_or_else(|| anyhow!("no authorization code"))?, get("state").unwrap_or_default()))
}

/// Signed-in account name from the ID token (display only; the token is not trusted for
/// anything else, so its signature isn't checked).
fn account_from_id_token(id_token: &str) -> Option<String> {
    let payload = b64url_decode(id_token.split('.').nth(1)?)?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    ["preferred_username", "upn", "email", "name"]
        .iter()
        .find_map(|k| claims.get(*k)?.as_str().map(str::to_string))
}

async fn token_request(cfg: &EntraConfig, form: &[(&str, &str)]) -> Result<TokenResponse> {
    let response = reqwest::Client::new().post(cfg.endpoint("token")).form(form).send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        let detail: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        bail!(
            "Sign-in token request failed ({status}): {}",
            detail["error_description"].as_str().or(detail["error"].as_str()).unwrap_or(&body)
        );
    }
    serde_json::from_str(&body).context("unexpected token response")
}

fn session_path(app_data: &Path) -> PathBuf {
    app_data.join(SESSION_FILE)
}

fn load_stored(app_data: &Path) -> Option<StoredSession> {
    serde_json::from_str(&std::fs::read_to_string(session_path(app_data)).ok()?).ok()
}

fn store(app_data: &Path, tokens: &TokenResponse, previous: Option<&StoredSession>) -> Result<()> {
    let refresh = match (&tokens.refresh_token, previous) {
        (Some(r), _) => crate::secure_storage::protect(r),
        (None, Some(p)) => p.refresh_token.clone(),
        (None, None) => return Ok(()),
    };
    let account = tokens
        .id_token
        .as_deref()
        .and_then(account_from_id_token)
        .or_else(|| previous.and_then(|p| p.account.clone()));
    std::fs::create_dir_all(app_data)?;
    std::fs::write(session_path(app_data), serde_json::to_string(&StoredSession { refresh_token: refresh, account })?)?;
    Ok(())
}

fn into_session(tokens: &TokenResponse) -> Session {
    Session {
        access_token: tokens.access_token.clone(),
        expires_at: Instant::now() + Duration::from_secs(tokens.expires_in),
    }
}

/// Browser sign-in: listen on a random loopback port, open the authorize URL with `open`,
/// wait for the redirect, then redeem the code with the PKCE verifier.
async fn interactive<F, Fut>(cfg: &EntraConfig, open: F) -> Result<TokenResponse>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let redirect_uri = format!("http://localhost:{}", listener.local_addr()?.port());
    let verifier = random_token(64);
    let state = random_token(32);
    open(authorize_url(cfg, &redirect_uri, &state, &pkce_challenge(&verifier))).await?;

    let (code, returned_state) = tokio::time::timeout(SIGN_IN_TIMEOUT, async {
        loop {
            let (mut stream, _) = listener.accept().await?;
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).await?;
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let first = request.lines().next().unwrap_or_default().to_string();
            // Browsers may also ask for /favicon.ico; only the root carries the result.
            if !first.starts_with("GET /?") {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n").await;
                continue;
            }
            let result = parse_redirect(&first);
            let message = if result.is_ok() {
                "Signed in to Noetis. You can close this tab."
            } else {
                "Noetis sign-in failed. You can close this tab and try again."
            };
            let body = format!("<!doctype html><title>Noetis</title><p style=\"font-family:sans-serif\">{message}</p>");
            let _ = stream
                .write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes())
                .await;
            return result;
        }
    })
    .await
    .map_err(|_| anyhow!("Sign-in timed out"))??;

    if returned_state != state {
        bail!("Sign-in response did not match this request (state mismatch)");
    }
    token_request(
        cfg,
        &[
            ("client_id", cfg.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_verifier", verifier.as_str()),
            ("scope", cfg.scopes().as_str()),
        ],
    )
    .await
}

/// A valid access token: cached, refreshed silently, or via browser sign-in (`open`).
async fn access_token_with<F, Fut>(app_data: &Path, cfg: &EntraConfig, open: F) -> Result<String>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut session = SESSION.lock().await;
    if let Some(s) = session.as_ref().filter(|s| s.expires_at > Instant::now() + EXPIRY_MARGIN) {
        return Ok(s.access_token.clone());
    }
    let stored = load_stored(app_data);
    if let Some(prev) = &stored {
        let refresh = crate::secure_storage::unprotect(&prev.refresh_token);
        if !refresh.is_empty() {
            let scopes = cfg.scopes();
            match token_request(
                cfg,
                &[("client_id", cfg.client_id.as_str()), ("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("scope", scopes.as_str())],
            )
            .await
            {
                Ok(tokens) => {
                    store(app_data, &tokens, stored.as_ref())?;
                    let token = tokens.access_token.clone();
                    *session = Some(into_session(&tokens));
                    return Ok(token);
                }
                Err(e) => log::warn!("Silent Entra ID token refresh failed, signing in again: {e:#}"),
            }
        }
    }
    let tokens = interactive(cfg, open).await?;
    store(app_data, &tokens, None)?;
    let token = tokens.access_token.clone();
    *session = Some(into_session(&tokens));
    Ok(token)
}

async fn open_in_browser(url: String) -> Result<()> {
    crate::api::api::open_external_url(url).await.map_err(|e| anyhow!(e))
}

/// Access token for the org's Foundry resource, signing in through the browser if needed.
pub async fn access_token(app_data: &Path, cfg: &EntraConfig) -> Result<String> {
    if crate::network_policy::is_strict_offline() {
        bail!("Strict Offline Mode is on, so the app can't sign in to your organization");
    }
    access_token_with(app_data, cfg, open_in_browser).await
}

// ---------------------------------------------------------------------------
// Tauri commands (Settings → Organization account)
// ---------------------------------------------------------------------------

fn app_data<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, String> {
    use tauri::Manager;
    app.path().app_data_dir().map_err(|e| e.to_string())
}

/// `{ enabled, signedIn, account }` for the organization account card.
#[tauri::command]
pub async fn api_entra_status<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<serde_json::Value, String> {
    let stored = load_stored(&app_data(&app)?);
    Ok(serde_json::json!({
        "enabled": crate::policy::managed_entra().is_some(),
        "signedIn": stored.is_some(),
        "account": stored.and_then(|s| s.account),
    }))
}

#[tauri::command]
pub async fn api_entra_sign_in<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    let cfg = crate::policy::managed_entra().ok_or("Organization sign-in is not configured")?;
    access_token(&app_data(&app)?, &cfg).await.map(|_| ()).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn api_entra_sign_out<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    *SESSION.lock().await = None;
    match std::fs::remove_file(session_path(&app_data(&app)?)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn base64url_roundtrip_and_known_values() {
        assert_eq!(b64url(b"f"), "Zg");
        assert_eq!(b64url(b"fo"), "Zm8");
        assert_eq!(b64url(b"foo"), "Zm9v");
        assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
        for data in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0u8, 255, 128, 7, 9]] {
            assert_eq!(b64url_decode(&b64url(data)).unwrap(), data);
        }
    }

    #[test]
    fn parses_redirects_and_account() {
        assert_eq!(parse_redirect("GET /?code=abc&state=xyz HTTP/1.1").unwrap(), ("abc".into(), "xyz".into()));
        let err = parse_redirect("GET /?error=access_denied&error_description=User+cancelled HTTP/1.1").unwrap_err();
        assert!(err.to_string().contains("User cancelled"), "{err}");
        assert!(parse_redirect("GET /?state=x HTTP/1.1").is_err());

        let payload = b64url(br#"{"preferred_username":"ana@contoso.com","name":"Ana"}"#);
        assert_eq!(account_from_id_token(&format!("h.{payload}.s")).as_deref(), Some("ana@contoso.com"));
        assert_eq!(account_from_id_token("garbage"), None);
    }

    #[test]
    fn authorize_url_has_pkce_and_scopes() {
        let cfg = EntraConfig { tenant_id: "t1".into(), client_id: "c1".into(), scope: None, authority: None };
        let url = url::Url::parse(&authorize_url(&cfg, "http://localhost:5000", "st", "ch")).unwrap();
        assert_eq!(url.as_str().split('?').next(), Some("https://login.microsoftonline.com/t1/oauth2/v2.0/authorize"));
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["redirect_uri"], "http://localhost:5000");
        assert!(q["scope"].starts_with("https://cognitiveservices.azure.com/.default") && q["scope"].contains("offline_access"));
    }

    /// Whole flow against a local mock of the Entra token endpoint and a scripted
    /// "browser": sign-in, cached token, silent refresh after expiry, no second prompt.
    #[tokio::test]
    async fn full_sign_in_flow_against_mock_authority() {
        use std::sync::{Arc, Mutex as StdMutex};

        // Mock authority: records form bodies, answers every token request.
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let authority = format!("http://127.0.0.1:{}", server.local_addr().unwrap().port());
        let bodies = Arc::new(StdMutex::new(Vec::<String>::new()));
        let recorded = bodies.clone();
        tokio::spawn(async move {
            let mut n = 0;
            loop {
                let (mut s, _) = server.accept().await.unwrap();
                let mut buf = vec![0u8; 16384];
                let len = s.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..len]).to_string();
                recorded.lock().unwrap().push(req.split("\r\n\r\n").nth(1).unwrap_or_default().to_string());
                n += 1;
                let id = b64url(br#"{"preferred_username":"ana@contoso.com"}"#);
                // First token expires immediately so the next call must refresh.
                let body = format!(r#"{{"access_token":"at{n}","expires_in":{},"refresh_token":"rt{n}","id_token":"h.{id}.s"}}"#, if n == 1 { 1 } else { 3600 });
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
            }
        });

        let cfg = EntraConfig { tenant_id: "tenant".into(), client_id: "client".into(), scope: None, authority: Some(authority) };
        let dir = tempfile::tempdir().unwrap();
        // Scripted browser: follow the authorize URL's redirect_uri with a code + the state.
        let browser = |url: String| async move {
            let u = url::Url::parse(&url).unwrap();
            let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
            assert!(q.contains_key("code_challenge"));
            let redirect = format!("{}/?code=the-code&state={}", q["redirect_uri"], q["state"]);
            tokio::spawn(async move { let _ = reqwest::get(redirect).await; });
            Ok(())
        };
        let never = |_: String| async { panic!("must not open the browser again") };

        *SESSION.lock().await = None;
        assert_eq!(access_token_with(dir.path(), &cfg, browser).await.unwrap(), "at1");
        let first = bodies.lock().unwrap()[0].clone();
        assert!(first.contains("grant_type=authorization_code") && first.contains("code=the-code") && first.contains("code_verifier="), "{first}");

        // Stored session: account visible, refresh token not in clear text on Windows.
        let stored = load_stored(dir.path()).unwrap();
        assert_eq!(stored.account.as_deref(), Some("ana@contoso.com"));
        #[cfg(target_os = "windows")]
        assert!(stored.refresh_token.starts_with("dpapi:v1:"));

        // at1 expired (1 s lifetime < margin) -> silent refresh with rt1, no browser.
        assert_eq!(access_token_with(dir.path(), &cfg, never).await.unwrap(), "at2");
        assert!(bodies.lock().unwrap()[1].contains("grant_type=refresh_token") && bodies.lock().unwrap()[1].contains("refresh_token=rt1"));
        // at2 is valid -> served from memory, no request.
        assert_eq!(access_token_with(dir.path(), &cfg, never).await.unwrap(), "at2");
        assert_eq!(bodies.lock().unwrap().len(), 2);
        *SESSION.lock().await = None;
    }
}
