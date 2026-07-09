//! Read-only HTTP-JSON server: the "remote" half of sivtr-to-sivtr peer access.
//!
//! Exposes the workspace query surface (`resolve` / `resolve-part` / `search` /
//! `sessions`) over HTTP so another device's sivtr can read this workspace's
//! structured sessions like reading local. The core does the work; this layer
//! only maps JSON ↔ core types, authenticates the bearer token, and redacts
//! obvious secrets before anything leaves the machine.
//!
//! Security posture: opt-in (`sivtr serve` never auto-starts), default bound to
//! localhost, bearer-token required, read-only (no write endpoints), and
//! workspace-scoped. See `commands::serve` for the CLI entry and bind policy.

pub mod redact;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sivtr_core::ai::AgentProvider;
use sivtr_core::query::load_workspace_records;
use sivtr_core::record::{WorkPart, WorkRecord, WorkRecordIndex, WorkRecordSearchScope, WorkRef};

/// Configuration handed to [`serve`]. All fields are enforced by the caller
/// (`commands::serve`): a missing/empty token must never reach here.
pub struct ServeConfig {
    /// `0.0.0.0:port` for LAN, `127.0.0.1:port` for localhost-only.
    pub addr: SocketAddr,
    /// Required bearer token. Requests without a matching `Authorization:
    /// Bearer <token>` are rejected.
    pub token: String,
    /// Workspace root to serve records for.
    pub workspace: std::path::PathBuf,
    /// When true, pass every record through [`redact::redact_record`] before
    /// serializing.
    pub redact: bool,
}

/// Run the server until interrupted. Blocks the (tokio) caller.
pub async fn serve(cfg: ServeConfig) -> Result<()> {
    let state = Arc::new(AppState {
        token: cfg.token,
        workspace: cfg.workspace,
        redact: cfg.redact,
    });

    let app = Router::new()
        .route("/agent-card", get(agent_card))
        .route("/resolve", post(resolve))
        .route("/resolve-part", post(resolve_part))
        .route("/search", post(search))
        .route("/sessions", post(sessions))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.addr)
        .await
        .with_context(|| format!("Failed to bind {}", cfg.addr))?;
    axum::serve(listener, app.into_make_service())
        .await
        .context("Server stopped with an error")?;
    Ok(())
}

struct AppState {
    token: String,
    workspace: std::path::PathBuf,
    redact: bool,
}

// --- error handling --------------------------------------------------------

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    BadRequest(String),
    NotFound(String),
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

/// Axum middleware-as-extractor: check the bearer token before each request.
fn require_auth(headers: &HeaderMap, expected: &str) -> Result<(), ApiError> {
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        });
    match provided {
        Some(t) if t == expected => Ok(()),
        _ => Err(ApiError::Unauthorized),
    }
}

// --- shared loading --------------------------------------------------------

impl AppState {
    fn index(&self, providers: Vec<AgentProvider>) -> Result<WorkRecordIndex, ApiError> {
        let query = load_workspace_records(&providers, &self.workspace, None)
            .map_err(ApiError::Internal)?;
        Ok(query.into_index())
    }

    fn maybe_redact(&self, record: WorkRecord) -> WorkRecord {
        if self.redact {
            redact::redact_record(&record)
        } else {
            record
        }
    }
}

// --- agent card ------------------------------------------------------------

#[derive(Serialize)]
struct AgentCard {
    name: &'static str,
    version: &'static str,
    protocol: &'static str,
    capabilities: &'static [&'static str],
}

async fn agent_card(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AgentCard>, ApiError> {
    require_auth(&headers, &state.token)?;
    Ok(Json(AgentCard {
        name: "sivtr",
        version: env!("CARGO_PKG_VERSION"),
        protocol: "sivtr/1",
        capabilities: &["resolve", "resolve-part", "search", "sessions"],
    }))
}

// --- resolve ---------------------------------------------------------------

#[derive(Deserialize)]
struct ResolveRequest {
    /// A local-shape ref, e.g. `terminal/session_42/3` or `codex/abc123/5/i/2`.
    /// (Remote clients send the body only; the origin is implied local here.)
    #[serde(rename = "ref")]
    reference: String,
    providers: Option<Vec<String>>,
}

#[derive(Serialize)]
struct ResolveResponse {
    record: WorkRecord,
}

async fn resolve(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ResolveRequest>,
) -> Result<Json<ResolveResponse>, ApiError> {
    require_auth(&headers, &state.token)?;
    let reference = req
        .reference
        .parse::<WorkRef>()
        .map_err(|e| ApiError::BadRequest(format!("invalid ref: {e}")))?;
    let providers = parse_providers(req.providers)?;
    let index = state.index(providers)?;
    let record = index
        .resolve(&reference)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("no record for `{reference}`")))?;
    Ok(Json(ResolveResponse {
        record: state.maybe_redact(record),
    }))
}

// --- resolve-part ----------------------------------------------------------

#[derive(Deserialize)]
struct ResolvePartRequest {
    #[serde(rename = "ref")]
    reference: String,
    providers: Option<Vec<String>>,
}

#[derive(Serialize)]
struct ResolvePartResponse {
    part: WorkPart,
}

async fn resolve_part(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ResolvePartRequest>,
) -> Result<Json<ResolvePartResponse>, ApiError> {
    require_auth(&headers, &state.token)?;
    let reference = req
        .reference
        .parse::<WorkRef>()
        .map_err(|e| ApiError::BadRequest(format!("invalid ref: {e}")))?;
    let providers = parse_providers(req.providers)?;
    let index = state.index(providers)?;
    let mut part = index
        .resolve_part(&reference)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("no part for `{reference}`")))?;
    if state.redact {
        part = redact::redact_part_owned(part);
    }
    Ok(Json(ResolvePartResponse { part }))
}

// --- search ----------------------------------------------------------------

#[derive(Deserialize)]
struct SearchRequest {
    /// Case-insensitive regex matched against record content/title/session.
    regex: String,
    #[serde(default = "default_scope")]
    scope: String,
    #[serde(default = "default_limit")]
    limit: usize,
    /// Restrict to these providers (command names); absent = all providers
    /// plus terminal.
    providers: Option<Vec<String>>,
}

fn default_scope() -> String {
    "content".to_string()
}

fn default_limit() -> usize {
    20
}

#[derive(Serialize)]
struct SearchHit {
    #[serde(rename = "ref")]
    reference: String,
    content: String,
    matched_line: usize,
}

#[derive(Serialize)]
struct SearchResponse {
    hits: Vec<SearchHit>,
}

async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    require_auth(&headers, &state.token)?;
    let regex =
        Regex::new(&req.regex).map_err(|e| ApiError::BadRequest(format!("invalid regex: {e}")))?;
    let scope = match req.scope.as_str() {
        "content" => WorkRecordSearchScope::Content,
        "title" => WorkRecordSearchScope::Title,
        "session" => WorkRecordSearchScope::Session,
        other => return Err(ApiError::BadRequest(format!("unknown scope `{other}`"))),
    };
    let providers = parse_providers(req.providers)?;
    let index = state.index(providers)?;
    // Search spans terminal + the requested agent providers; `include` is true
    // for all records (terminal has no AgentProvider to filter on).
    let matches = index.search(&regex, scope, req.limit, |_| true);
    let hits = matches
        .into_iter()
        .map(|m| {
            let reference = m.record.work_ref.with_target(m.target).to_string();
            SearchHit {
                reference,
                content: m.content,
                matched_line: m.matched_line,
            }
        })
        .collect();
    Ok(Json(SearchResponse { hits }))
}

// --- sessions --------------------------------------------------------------

#[derive(Deserialize)]
struct SessionsRequest {
    providers: Option<Vec<String>>,
    #[serde(default = "default_recent")]
    recent: Option<usize>,
}

fn default_recent() -> Option<usize> {
    Some(50)
}

#[derive(Serialize)]
struct SessionInfo {
    provider: String,
    id: Option<String>,
    title: Option<String>,
    cwd: Option<String>,
    path: String,
}

#[derive(Serialize)]
struct SessionsResponse {
    sessions: Vec<SessionInfo>,
}

async fn sessions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SessionsRequest>,
) -> Result<Json<SessionsResponse>, ApiError> {
    require_auth(&headers, &state.token)?;
    let providers = parse_providers(req.providers)?;
    let mut out = Vec::new();
    for provider in &providers {
        let source = provider.session_provider();
        let mut list = source
            .list_recent_sessions(Some(&state.workspace))
            .map_err(ApiError::Internal)?;
        if let Some(limit) = req.recent {
            list.truncate(limit);
        }
        for info in list {
            out.push(SessionInfo {
                provider: provider.command_name().to_string(),
                id: info.id,
                title: info.title,
                cwd: info.cwd,
                path: info.path.display().to_string(),
            });
        }
    }
    Ok(Json(SessionsResponse { sessions: out }))
}

// --- helpers ---------------------------------------------------------------

/// Parse a list of provider command names. An absent or empty list means "all
/// providers". Terminal records are always included by `load_workspace_records`
/// regardless of the provider list, so callers don't pass "terminal" here.
fn parse_providers(raw: Option<Vec<String>>) -> Result<Vec<AgentProvider>, ApiError> {
    let Some(names) = raw else {
        return Ok(AgentProvider::all().iter().map(|s| s.provider).collect());
    };
    let mut providers = Vec::new();
    for name in names {
        let p = AgentProvider::from_command_name(&name)
            .ok_or_else(|| ApiError::BadRequest(format!("unknown provider `{name}`")))?;
        if !providers.contains(&p) {
            providers.push(p);
        }
    }
    Ok(providers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_providers_absent_yields_all() {
        let providers = parse_providers(None).unwrap();
        assert!(providers.len() >= 2);
    }

    #[test]
    fn parse_providers_resolves_command_names() {
        let providers = parse_providers(Some(vec!["codex".into(), "pi".into()])).unwrap();
        assert!(providers.contains(&AgentProvider::Codex));
        assert!(providers.contains(&AgentProvider::Pi));
    }

    #[test]
    fn parse_providers_rejects_unknown() {
        assert!(parse_providers(Some(vec!["nosuch".into()])).is_err());
    }
}
