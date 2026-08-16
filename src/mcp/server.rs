use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde::Serialize;
use sivtr_core::ai::AgentProvider;
use sivtr_core::workspace;

use crate::commands::memory::{filter, search, show, workset, zoom};
use crate::commands::remote::workspace::workspace_display_name;
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse};

use super::types::{
    memory_result, show_result, to_filter_args, to_search_args, to_show_args, to_zoom_args,
    FilterParams, MountStatus, ProviderStatus, SearchParams, ShowParams, StatusParams,
    StatusResult, VarStatus, WorkspaceOrigin, ZoomParams,
};

#[derive(Clone)]
pub struct SivtrMcp {
    // Used by #[tool_handler] macro-generated ServerHandler methods.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SivtrMcp {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Search local or mounted remote workspace memory (terminal + AI sessions). Uses the same bounds as CLI search: defaults to latest=5 when neither latest nor limit is set. Prefer narrow queries. Returns refs and compact summaries by default."
    )]
    fn sivtr_search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let detail = params.detail.clone();
        let save = params.save.clone();
        let args = to_search_args(&params).map_err(invalid_params)?;
        let set = search::run(&args).map_err(tool_error)?;
        let result = memory_result(&set, detail.as_deref(), save).map_err(tool_error)?;
        ok_json(result)
    }

    #[tool(
        description = "Show exact content for a WorkRef or WorkSet handle (@last, @name, desk:terminal/...). Use after search to expand evidence."
    )]
    fn sivtr_show(
        &self,
        Parameters(params): Parameters<ShowParams>,
    ) -> Result<CallToolResult, McpError> {
        let mode = params.mode.clone();
        let args = to_show_args(&params).map_err(invalid_params)?;
        let set = show::run(&args).map_err(tool_error)?;
        let result = show_result(&set, mode.as_deref()).map_err(tool_error)?;
        ok_json(result)
    }

    #[tool(
        description = "Expand neighboring records around search hits (default source @last). Useful for handoff context."
    )]
    fn sivtr_zoom(
        &self,
        Parameters(params): Parameters<ZoomParams>,
    ) -> Result<CallToolResult, McpError> {
        let detail = params.detail.clone();
        let save = params.save.clone();
        let args = to_zoom_args(&params);
        let set = zoom::run(&args).map_err(tool_error)?;
        let result = memory_result(&set, detail.as_deref(), save).map_err(tool_error)?;
        ok_json(result)
    }

    #[tool(
        description = "Filter/narrow an existing WorkSet or source. Defaults to @last. Prefer this over re-running broad searches."
    )]
    fn sivtr_filter(
        &self,
        Parameters(params): Parameters<FilterParams>,
    ) -> Result<CallToolResult, McpError> {
        let detail = params.detail.clone();
        let save = params.save.clone();
        let args = to_filter_args(&params).map_err(invalid_params)?;
        let set = filter::run(&args).map_err(tool_error)?;
        let result = memory_result(&set, detail.as_deref(), save).map_err(tool_error)?;
        ok_json(result)
    }

    #[tool(
        description = "Environment and origin status: version, hooks, providers, daemon, local workspace origin labels (ws), remote mounts, and saved WorkSet vars."
    )]
    fn sivtr_status(
        &self,
        Parameters(params): Parameters<StatusParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = collect_status(params.cwd.as_deref()).map_err(tool_error)?;
        ok_json(result)
    }
}

#[tool_handler]
impl ServerHandler for SivtrMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "sivtr is local workspace memory for terminal output and AI sessions. \
Search narrowly, expand with show/zoom, and treat results as evidence—verify current files and tests before claiming present state. \
Search defaults to latest=5 when neither latest nor limit is set (same as CLI). \
Use origin-prefixed sources like desk:terminal only after mounts exist (see sivtr_status)."
                .into(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    /// Wraps every tool call with activity tracking so the idle-exit watchdog
    /// sees it. `#[tool_handler]` skips generating this method because it is
    /// already present.
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let _activity = ActivityGuard::begin();
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }
}

pub async fn serve_stdio(idle_exit: Option<Duration>) -> anyhow::Result<()> {
    let server = Arc::new(SivtrMcp::new());
    let service = server.serve(stdio()).await?;
    let Some(idle) = idle_exit else {
        service.waiting().await?;
        return Ok(());
    };

    // Idle-exit watchdog: exit 0 once no tool call has completed for `idle`
    // seconds (and none is in flight). The clock only starts after the first
    // tool call — "exit after use" — so a freshly spawned server that the host
    // is still initializing is never killed. MCP stdio hosts respawn the
    // server on the next tool use, so an idle server never lingers.
    //
    // This uses a hard `process::exit`: returning instead would drop the tokio
    // runtime, whose shutdown joins the blocking pool — and the stdio reader
    // thread is parked on the still-open stdin pipe, so the drop hangs until
    // the host closes stdin. Exiting directly is safe here: no tool call is in
    // flight (BUSY gate) and the last response was flushed seconds ago.
    let wait = service.waiting();
    tokio::pin!(wait);
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            reason = &mut wait => {
                reason?;
                return Ok(());
            }
            _ = ticker.tick() => {
                if HAS_ACTIVITY.load(Ordering::Relaxed)
                    && !BUSY.load(Ordering::Relaxed)
                    && idle_elapsed() >= idle.as_millis() as u64
                {
                    std::process::exit(0);
                }
            }
        }
    }
}

/// Milliseconds since the last completed tool call (0 before any call).
fn idle_elapsed() -> u64 {
    let last = LAST_ACTIVITY_MS.load(Ordering::Relaxed);
    if last == 0 {
        return u64::MAX;
    }
    now_ms().saturating_sub(last)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Last completed tool call, in epoch milliseconds.
static LAST_ACTIVITY_MS: AtomicU64 = AtomicU64::new(0);
/// A tool call is currently executing; the watchdog must not exit mid-call.
static BUSY: AtomicBool = AtomicBool::new(false);
/// Whether any tool call has ever completed (the idle clock starts then).
static HAS_ACTIVITY: AtomicBool = AtomicBool::new(false);

/// Marks activity at the start of a tool call and again when it completes.
/// Dropped on every exit path, including errors.
struct ActivityGuard;

impl ActivityGuard {
    fn begin() -> Self {
        HAS_ACTIVITY.store(true, Ordering::Relaxed);
        BUSY.store(true, Ordering::Relaxed);
        LAST_ACTIVITY_MS.store(now_ms(), Ordering::Relaxed);
        Self
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        LAST_ACTIVITY_MS.store(now_ms(), Ordering::Relaxed);
        BUSY.store(false, Ordering::Relaxed);
    }
}

fn ok_json<T: Serialize>(value: T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn tool_error(error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(error.to_string(), None)
}

fn invalid_params(error: impl std::fmt::Display) -> McpError {
    McpError::invalid_params(error.to_string(), None)
}

fn collect_status(cwd: Option<&str>) -> anyhow::Result<StatusResult> {
    let cwd = cwd.map(PathBuf::from).unwrap_or(std::env::current_dir()?);

    let config_path = dirs::config_dir().map(|dir| dir.join("sivtr").join("config.toml"));
    let config_present = config_path.as_ref().is_some_and(|path| path.exists());

    let session_dir = dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".local").join("state")))
        .map(|dir| dir.join("sivtr"));
    let session_dir_present = session_dir.as_ref().is_some_and(|path| path.exists());

    let shell_hooks_installed = shell_hooks_installed();
    let providers = provider_status();
    let (daemon_running, daemon_node_id) = daemon_status();
    let local_workspaces = local_workspace_origins(&cwd)?;
    let mounts = mount_status(&cwd);
    let vars = workset::list_saved().ok().map(|list| {
        list.into_iter()
            .map(|var| VarStatus {
                name: var.name,
                items: var.items,
                created_at: var.created_at,
            })
            .collect()
    });

    Ok(StatusResult {
        version: env!("CARGO_PKG_VERSION").to_string(),
        config_path: config_path.map(|path| path.display().to_string()),
        config_present,
        session_dir: session_dir.map(|path| path.display().to_string()),
        session_dir_present,
        shell_hooks_installed,
        providers,
        daemon_running,
        daemon_node_id,
        local_workspaces,
        mounts,
        vars,
    })
}

fn shell_hooks_installed() -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let marker = "# >>> sivtr shell integration >>>";
    for rel in [".bashrc", ".zshrc"] {
        let path = home.join(rel);
        if path
            .exists()
            .then(|| std::fs::read_to_string(&path).ok())
            .flatten()
            .is_some_and(|content| content.contains(marker))
        {
            return true;
        }
    }
    if let Some(config_dir) = dirs::config_dir() {
        let nu = config_dir.join("nushell").join("config.nu");
        if nu
            .exists()
            .then(|| std::fs::read_to_string(&nu).ok())
            .flatten()
            .is_some_and(|content| content.contains(marker))
        {
            return true;
        }
    }
    for cmd in ["pwsh", "powershell"] {
        if let Ok(Some(profile)) = crate::commands::terminal::init::shell_printed_path(
            cmd,
            &["-NoProfile", "-Command", "Write-Output $PROFILE"],
        ) {
            let path = Path::new(&profile);
            if path
                .exists()
                .then(|| std::fs::read_to_string(path).ok())
                .flatten()
                .is_some_and(|content| content.contains(marker))
            {
                return true;
            }
        }
    }
    false
}

fn provider_status() -> Vec<ProviderStatus> {
    AgentProvider::all()
        .iter()
        .map(|spec| {
            let provider = spec.provider.session_provider();
            match provider.list_recent_sessions(None) {
                Ok(sessions) => ProviderStatus {
                    name: spec.provider.name().to_string(),
                    sessions: Some(sessions.len()),
                    error: None,
                },
                Err(error) => ProviderStatus {
                    name: spec.provider.name().to_string(),
                    sessions: None,
                    error: Some(error.to_string()),
                },
            }
        })
        .collect()
}

fn daemon_status() -> (bool, Option<String>) {
    match ipc::read_daemon_info() {
        Ok(info) => (true, Some(info.node_id)),
        Err(_) => (false, None),
    }
}

fn local_workspace_origins(cwd: &Path) -> anyhow::Result<Vec<WorkspaceOrigin>> {
    let current = workspace::resolve_current_workspace()?.map(|paths| paths.key);
    // Ensure cwd is registered when possible.
    let _ = workspace::ensure_workspace_for_dir(cwd);
    let mut metas = workspace::list_workspaces()?;
    if let Some(current_key) = current.as_deref() {
        metas.sort_by(|a, b| {
            let a_cur = a.key == current_key;
            let b_cur = b.key == current_key;
            b_cur
                .cmp(&a_cur)
                .then_with(|| b.last_seen_at.cmp(&a.last_seen_at))
        });
    }
    Ok(metas
        .into_iter()
        .map(|meta| {
            let current = current.as_deref() == Some(meta.key.as_str());
            WorkspaceOrigin {
                name: workspace_display_name(&meta),
                root: meta.root,
                key: meta.key,
                current,
            }
        })
        .collect())
}

fn mount_status(cwd: &Path) -> Vec<MountStatus> {
    let key = workspace::resolve_workspace_for_dir(cwd)
        .ok()
        .flatten()
        .map(|paths| paths.key)
        .or_else(|| {
            workspace::resolve_current_workspace()
                .ok()
                .flatten()
                .map(|paths| paths.key)
        });
    let Some(workspace_key) = key.as_deref() else {
        return Vec::new();
    };
    match ipc::call(LocalRequest::RemoteList {
        workspace_key: workspace_key.to_string(),
    }) {
        Ok(LocalResponse::Mounts(mounts)) => mounts
            .into_iter()
            .map(|mount| MountStatus {
                alias: mount.alias,
                peer_name: mount.peer_name,
                share_name: mount.share_name,
            })
            .collect(),
        _ => Vec::new(),
    }
}
