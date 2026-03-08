use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use bitmap_renderer::Block;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

struct AppState {
    rpc_url: String,
    rpc_user: String,
    rpc_pass: String,
    client: Client,
}

#[derive(Deserialize)]
struct BlockResponse {
    result: Block,
}

async fn get_block(state: &AppState, height: u64) -> Result<Block, String> {
    let hash_body = serde_json::json!({
        "jsonrpc": "1.0",
        "id": "bitmap",
        "method": "getblockhash",
        "params": [height]
    });

    let hash_resp: serde_json::Value = state
        .client
        .post(&state.rpc_url)
        .basic_auth(&state.rpc_user, Some(&state.rpc_pass))
        .json(&hash_body)
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse hash response: {e}"))?;

    let hash = hash_resp["result"]
        .as_str()
        .ok_or("No block hash in response")?;

    let block_body = serde_json::json!({
        "jsonrpc": "1.0",
        "id": "bitmap",
        "method": "getblock",
        "params": [hash, 2]
    });

    let block_resp: BlockResponse = state
        .client
        .post(&state.rpc_url)
        .basic_auth(&state.rpc_user, Some(&state.rpc_pass))
        .json(&block_body)
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse block response: {e}"))?;

    Ok(block_resp.result)
}

async fn handle_bitmap(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
) -> impl IntoResponse {
    let height_str = filename.trim_end_matches(".png");
    let height: u64 = match height_str.parse() {
        Ok(h) => h,
        Err(_) => {
            let mut headers = HeaderMap::new();
            headers.insert("cache-control", HeaderValue::from_static("no-store"));
            return (StatusCode::BAD_REQUEST, headers, b"Invalid block number".to_vec());
        }
    };

    let block = match get_block(&state, height).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to fetch block {height}: {e}");
            let mut headers = HeaderMap::new();
            headers.insert("cache-control", HeaderValue::from_static("no-store"));
            return (
                StatusCode::BAD_GATEWAY,
                headers,
                format!("RPC error: {e}").into_bytes(),
            );
        }
    };

    let png = bitmap_renderer::render_bitmap(&block);

    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("image/png"));
    headers.insert(
        "cache-control",
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );

    (StatusCode::OK, headers, png)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let rpc_url = std::env::var("BTC_RPC_URL").unwrap_or_else(|_| "http://localhost:8332".into());
    let rpc_user = std::env::var("BTC_RPC_USER").unwrap_or_else(|_| "bitcoin".into());
    let rpc_pass = std::env::var("BTC_RPC_PASS").unwrap_or_else(|_| "bitcoin".into());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3080);

    let state = Arc::new(AppState {
        rpc_url,
        rpc_user,
        rpc_pass,
        client: Client::new(),
    });

    let app = Router::new()
        .route("/{filename}", axum::routing::get(handle_bitmap))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("bitmap-renderer listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
