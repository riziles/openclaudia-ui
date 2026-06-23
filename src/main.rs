//! OpenClaudia UI — Web GUI launcher for OpenClaudia.
//!
//! Finds an available port, starts the OpenClaudia proxy on an internal port,
//! and serves the web GUI on a public port. All `/v1/*` API requests are
//! proxied to the internal OpenClaudia instance.
//!
//! Usage: `openclaudia-ui` (no arguments needed)

use anyhow::Context;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    response::Response,
    routing::any,
    Router,
};
use reqwest::Client;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use tokio::net::TcpListener as TokioTcpListener;
use tower_http::services::ServeDir;
use tracing::{error, info};

/// Shared state passed to axum handlers.
#[derive(Clone)]
struct AppState {
    /// Base URL of the internal OpenClaudia proxy (e.g. `http://127.0.0.1:61923`)
    proxy_base: Arc<String>,
    /// HTTP client for proxying requests
    client: Client,
}

/// Find a free TCP port on localhost.
fn find_free_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .context("Failed to find a free port")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// Launch the OpenClaudia proxy as a child process on the given port.
fn start_openclaudia_proxy(port: u16) -> anyhow::Result<Child> {
    let openclaudia_bin = find_openclaudia_binary()?;
    info!(
        bin = %openclaudia_bin.display(),
        port,
        "Starting OpenClaudia proxy"
    );
    let child = Command::new(&openclaudia_bin)
        .args(["start", "--port", &port.to_string(), "--host", "127.0.0.1"])
        .spawn()
        .with_context(|| {
            format!(
                "Failed to start openclaudia at {}. Is it installed and on PATH?",
                openclaudia_bin.display()
            )
        })?;
    Ok(child)
}

/// Find the `openclaudia` binary. Checks the directory containing the
/// current executable first, then falls back to PATH.
fn find_openclaudia_binary() -> anyhow::Result<PathBuf> {
    // Check sibling directory first (for portable installation)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            #[cfg(windows)]
            let name = "openclaudia.exe";
            #[cfg(not(windows))]
            let name = "openclaudia";

            let sibling = parent.join(name);
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }
    // Fall back to PATH — Command::new searches PATH natively on spawn
    Ok(PathBuf::from("openclaudia"))
}

/// Wait for the OpenClaudia proxy to be ready by polling `/health`.
async fn wait_for_proxy_ready(port: u16, timeout_secs: u64) -> anyhow::Result<()> {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = Client::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    loop {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!("OpenClaudia proxy is ready");
                return Ok(());
            }
            Ok(resp) => {
                info!(status = %resp.status(), "Proxy not ready yet, retrying...");
            }
            Err(_) => {
                // Connection refused — proxy not listening yet
            }
        }

        if std::time::Instant::now() > deadline {
            anyhow::bail!("OpenClaudia proxy did not start within {timeout_secs}s");
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

/// Proxy handler: forwards requests to the internal OpenClaudia proxy.
async fn proxy_handler(
    State(state): State<AppState>,
    req: Request,
) -> Result<Response, StatusCode> {
    let path = req.uri().path_and_query().map_or("/", |pq| pq.as_str());
    let target_url = format!("{}{}", state.proxy_base, path);

    let method = req.method().clone();
    let headers = req.headers().clone();
    let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;

    let mut proxy_req = state
        .client
        .request(method, &target_url)
        .body(body_bytes.to_vec());

    // Forward relevant headers
    for (key, value) in headers.iter() {
        if key == header::HOST || key == header::CONTENT_LENGTH {
            continue;
        }
        proxy_req = proxy_req.header(key, value);
    }

    match proxy_req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let resp_headers = resp.headers().clone();
            let resp_body = resp.bytes().await.unwrap_or_default();

            let mut builder = Response::builder().status(status);
            for (key, value) in resp_headers.iter() {
                builder = builder.header(key, value);
            }
            builder
                .body(Body::from(resp_body))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(e) => {
            error!(%target_url, error = %e, "Proxy request failed");
            Err(StatusCode::BAD_GATEWAY)
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "openclaudia_ui=info".into()),
        )
        .init();

    // 1. Find a free port for the internal OpenClaudia proxy
    let proxy_port = find_free_port()?;
    info!(proxy_port, "Found free port for OpenClaudia proxy");

    // 2. Find a free port for the UI server
    let ui_port = find_free_port()?;
    info!(ui_port, "Found free port for web UI");

    // 3. Launch OpenClaudia proxy
    let _proxy_child = start_openclaudia_proxy(proxy_port)?;

    // 4. Wait for the proxy to be ready
    wait_for_proxy_ready(proxy_port, 30).await?;

    // 5. Find the static files directory (next to the binary)
    let exe_dir = std::env::current_exe()?
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let static_dir = exe_dir.join("static");
    if !static_dir.exists() {
        // Fallback: look in the current directory
        let cwd_static = std::env::current_dir()?.join("static");
        if cwd_static.exists() {
            anyhow::bail!(
                "Static files directory not found at {}.\n\
                 Place the static/ directory next to the binary: {}",
                static_dir.display(),
                exe_dir.display()
            );
        }
        anyhow::bail!(
            "Static files directory not found.\n\
             Looked at: {}\n\
             Looked at: {}\n\
             Place the static/ directory next to the binary.",
            static_dir.display(),
            cwd_static.display()
        );
    }

    // 6. Start the UI server
    let proxy_base = Arc::new(format!("http://127.0.0.1:{proxy_port}"));
    let state = AppState {
        proxy_base,
        client: Client::new(),
    };

    let app = Router::new()
        // Proxy all /v1/* requests to the internal openclaudia proxy
        .route("/v1/{*path}", any(proxy_handler))
        .route("/health", any(proxy_handler))
        .route("/stats", any(proxy_handler))
        .route("/auth/{*path}", any(proxy_handler))
        // Serve static web GUI files for everything else
        .fallback_service(ServeDir::new(&static_dir))
        .with_state(state);

    let addr = format!("127.0.0.1:{ui_port}");
    let url = format!("http://{addr}");
    info!("OpenClaudia Web UI: {url}");
    println!("OpenClaudia Web UI: {url}");
    println!("Press Ctrl+C to stop.");

    let listener = TokioTcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
