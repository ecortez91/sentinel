use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::constants::*;

/// Anthropic OAuth token refresh endpoint (same as OpenCode/OpenClaw use).
const ANTHROPIC_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";

/// How we authenticate with the Anthropic API.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// Standard API key (x-api-key header)
    ApiKey(String),
    /// OAuth access token from OpenCode/Claude Code (Bearer token)
    OAuthToken(String),
}

impl AuthMethod {
    pub fn display_name(&self) -> &str {
        match self {
            AuthMethod::ApiKey(_) => "API Key",
            AuthMethod::OAuthToken(_) => "OAuth (Max Sub)",
        }
    }
}

/// Async Claude API client.
///
/// Supports both standard API keys and OAuth tokens (from OpenCode).
/// Uses streaming to send response chunks back in real-time.
pub struct ClaudeClient {
    client: Client,
    auth: AuthMethod,
    model: String,
}

/// Events sent from the AI task back to the main loop.
#[derive(Debug)]
pub enum AiEvent {
    /// A chunk of text from the streaming response.
    Chunk(String),
    /// The response is complete.
    Done,
    /// An error occurred.
    Error(String),
}

/// Structure of OpenCode's auth.json file.
#[derive(Deserialize)]
struct OpenCodeAuth {
    anthropic: Option<OpenCodeAnthropicAuth>,
}

#[derive(Deserialize)]
struct OpenCodeAnthropicAuth {
    access: Option<String>,
    refresh: Option<String>,
    expires: Option<u64>,
}

impl ClaudeClient {
    pub fn new(auth: AuthMethod) -> Self {
        Self {
            client: Client::new(),
            auth,
            model: CLAUDE_MODEL.to_string(),
        }
    }

    /// Whether we're using OAuth authentication (requires special headers/prompts).
    fn is_oauth(&self) -> bool {
        matches!(self.auth, AuthMethod::OAuthToken(_))
    }

    /// Try to discover auth credentials automatically.
    /// Priority:
    /// 1. ANTHROPIC_API_KEY env var
    /// 2. OpenCode auth.json (~/.local/share/opencode/auth.json) — with auto-refresh
    /// 3. Claude Code plaintext credentials
    pub async fn discover_auth() -> Option<AuthMethod> {
        // 1. Check env var
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                // Detect if it's an OAuth token vs API key
                if key.starts_with("sk-ant-oat") {
                    return Some(AuthMethod::OAuthToken(key));
                }
                return Some(AuthMethod::ApiKey(key));
            }
        }

        // 2. Check OpenCode auth.json (with automatic token refresh)
        if let Some(auth) = Self::read_opencode_auth().await {
            return Some(auth);
        }

        // 3. Check Claude Code plaintext credentials
        if let Some(auth) = Self::read_claude_code_auth() {
            return Some(auth);
        }

        None
    }

    /// Read OAuth token from OpenCode's auth storage.
    /// If the token is expired (or near expiry), automatically refreshes it
    /// using the Anthropic OAuth endpoint and writes the new tokens back to
    /// auth.json so OpenCode/OpenClaw stay in sync.
    async fn read_opencode_auth() -> Option<AuthMethod> {
        let auth_path = dirs_opencode_auth();
        let content = std::fs::read_to_string(&auth_path).ok()?;
        let auth: OpenCodeAuth = serde_json::from_str(&content).ok()?;
        let anthropic = auth.anthropic?;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as u64;

        // Check if token is expired or near expiry
        let needs_refresh = match anthropic.expires {
            Some(expires) => now_ms + TOKEN_EXPIRY_BUFFER_MS as u64 >= expires,
            None => false, // No expiry info — assume it's valid
        };

        if needs_refresh {
            if let Some(refresh_token) = &anthropic.refresh {
                if !refresh_token.is_empty() {
                    // Attempt to refresh the token
                    if let Some((new_access, new_refresh, new_expires)) =
                        refresh_opencode_token(refresh_token).await
                    {
                        // Write the refreshed tokens back to auth.json
                        write_opencode_auth(&new_access, &new_refresh, new_expires);
                        return Some(AuthMethod::OAuthToken(new_access));
                    }
                    // Refresh failed — fall through and try the old token anyway
                }
            }
        }

        let access = anthropic.access?;
        if access.is_empty() {
            return None;
        }

        Some(AuthMethod::OAuthToken(access))
    }

    /// Try reading from Claude Code's plaintext credential store (Linux).
    fn read_claude_code_auth() -> Option<AuthMethod> {
        let cred_path = crate::constants::home_dir().join(".claude").join(".credentials.json");
        let content = std::fs::read_to_string(&cred_path).ok()?;
        let creds: Value = serde_json::from_str(&content).ok()?;

        // Claude Code stores {accessToken, refreshToken, expiresAt, ...}
        if let Some(token) = creds.get("accessToken").and_then(|v| v.as_str()) {
            if !token.is_empty() {
                return Some(AuthMethod::OAuthToken(token.to_string()));
            }
        }

        // Or it might store an API key directly
        if let Some(key) = creds.get("apiKey").and_then(|v| v.as_str()) {
            if !key.is_empty() {
                return Some(AuthMethod::ApiKey(key.to_string()));
            }
        }

        None
    }

    /// Send a question with system context, streaming chunks back via channel.
    pub async fn ask_streaming(
        &self,
        system_prompt: &str,
        messages: Vec<Value>,
        tx: mpsc::UnboundedSender<AiEvent>,
    ) -> Result<()> {
        // OAuth tokens require the system prompt to start with Claude Code identity
        let system_value = if self.is_oauth() {
            serde_json::json!([
                {"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude."},
                {"type": "text", "text": system_prompt}
            ])
        } else {
            serde_json::json!(system_prompt)
        };

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": CLAUDE_MAX_TOKENS,
            "stream": true,
            "system": system_value,
            "messages": messages,
        });

        let mut request = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("anthropic-version", CLAUDE_API_VERSION)
            .header("content-type", "application/json");

        // OAuth tokens require beta feature flags
        if self.is_oauth() {
            request = request.header(
                "anthropic-beta",
                CLAUDE_BETA_FLAGS,
            );
        }

        // Apply the right auth header based on method
        request = match &self.auth {
            AuthMethod::ApiKey(key) => request.header("x-api-key", key),
            AuthMethod::OAuthToken(token) => {
                request.header("Authorization", format!("Bearer {}", token))
            }
        };

        let response = request
            .json(&body)
            .send()
            .await
            .context("Failed to connect to Claude API")?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            let err_msg = if status.as_u16() == 401 {
                format!(
                    "Authentication failed (401). Auth method: {}. \
                     Token refresh was attempted but the API still rejected the request. \
                     Try: export ANTHROPIC_API_KEY=sk-ant-... or re-authenticate in OpenCode.",
                    self.auth.display_name()
                )
            } else if status.as_u16() == 403 {
                "Access forbidden (403). Your subscription may not have API access.".to_string()
            } else if status.as_u16() == 429 {
                "Rate limited. Wait a moment and try again.".to_string()
            } else {
                format!("API error {}: {}", status, crate::utils::truncate_str(&body_text, 300))
            };
            let _ = tx.send(AiEvent::Error(err_msg));
            return Ok(());
        }

        // Stream SSE events
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    buffer.push_str(&text);

                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 1..].to_string();

                        if let Some(data) = line.strip_prefix("data: ") {
                            if data.trim() == "[DONE]" {
                                let _ = tx.send(AiEvent::Done);
                                return Ok(());
                            }

                            if let Ok(event) = serde_json::from_str::<Value>(data) {
                                if event.get("type").and_then(|t| t.as_str())
                                    == Some("content_block_delta")
                                {
                                    if let Some(delta) = event.get("delta") {
                                        if let Some(text) =
                                            delta.get("text").and_then(|t| t.as_str())
                                        {
                                            let _ = tx.send(AiEvent::Chunk(text.to_string()));
                                        }
                                    }
                                }

                                if event.get("type").and_then(|t| t.as_str())
                                    == Some("message_stop")
                                {
                                    let _ = tx.send(AiEvent::Done);
                                    return Ok(());
                                }

                                if event.get("type").and_then(|t| t.as_str()) == Some("error") {
                                    let err_msg = event
                                        .get("error")
                                        .and_then(|e| e.get("message"))
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("Unknown streaming error");
                                    let _ = tx.send(AiEvent::Error(err_msg.to_string()));
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(AiEvent::Error(format!("Stream error: {}", e)));
                    return Ok(());
                }
            }
        }

        let _ = tx.send(AiEvent::Done);
        Ok(())
    }

}

/// Path to OpenCode's auth.json
fn dirs_opencode_auth() -> PathBuf {
    crate::constants::home_dir()
        .join(".local")
        .join("share")
        .join("opencode")
        .join("auth.json")
}

/// Refresh an expired OAuth token using Anthropic's token endpoint.
/// Returns (new_access_token, new_refresh_token, expires_at_ms) on success.
async fn refresh_opencode_token(refresh_token: &str) -> Option<(String, String, u64)> {
    let client = Client::new();
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "client_id": OAUTH_CLIENT_ID,
        "refresh_token": refresh_token,
    });

    let response = client
        .post(ANTHROPIC_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let data: Value = response.json().await.ok()?;

    let access_token = data.get("access_token")?.as_str()?.to_string();
    let new_refresh = data.get("refresh_token")?.as_str()?.to_string();
    let expires_in = data.get("expires_in")?.as_u64()?;

    // Calculate expiry with 5-minute buffer (same as OpenClaw)
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64
        + expires_in * 1000
        - TOKEN_EXPIRY_BUFFER_MS as u64;

    Some((access_token, new_refresh, expires_at))
}

/// Write refreshed tokens back to OpenCode's auth.json.
/// Preserves any other keys in the file (e.g., GitLab tokens).
fn write_opencode_auth(access: &str, refresh: &str, expires: u64) {
    let auth_path = dirs_opencode_auth();

    // Read existing file to preserve other provider entries
    let mut auth_data: Value = match std::fs::read_to_string(&auth_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };

    // Update only the anthropic section
    auth_data["anthropic"] = serde_json::json!({
        "type": "oauth",
        "access": access,
        "refresh": refresh,
        "expires": expires,
    });

    // Write back — ignore errors (non-critical, worst case the old token stays)
    let _ = std::fs::write(&auth_path, serde_json::to_string_pretty(&auth_data).unwrap_or_default());
}


