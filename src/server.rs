//! axum HTTP 服务：路由、handler、共享状态。
//!
//! 共享状态是 `Arc<RfcConnectionPool>`：多连接池 + 自动重连。
//! handler 内通过 `tokio::task::spawn_blocking` 把 FFI 执行丢到阻塞线程池，
//! 不同请求可拿到不同连接并行执行 SAP 调用；同时让非 Send 的裸指针类型
//! 只存在于阻塞闭包内，不跨 await 点，保证 future 干净 Send。

use crate::api::{InvokeRequest, InvokeResponse};
use crate::error::RfcError;
use crate::executor::execute_collect;
use crate::pool::RfcConnectionPool;
use axum::{routing::post, Json, Router};
use std::sync::Arc;

/// 全局共享状态：连接池（内部含连接 + 重连参数）
pub type SharedPool = Arc<RfcConnectionPool>;

/// 构建带共享连接池的 Router
pub fn app(pool: SharedPool) -> Router {
    Router::new()
        .route("/api/rfc", post(invoke_handler))
        .route("/health", axum::routing::get(health_handler))
        .with_state(pool)
}

/// 启动 HTTP 服务（阻塞当前异步任务直到服务器结束）。
/// `shutdown` 是一个 future，完成时触发优雅停机。
pub async fn run(
    pool: SharedPool,
    listen_addr: &str,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), std::io::Error> {
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    tracing::info!(addr = listen_addr, "HTTP 服务监听");
    tracing::info!("  - POST /api/rfc   通用 RFC 调用");
    tracing::info!("  - GET  /health    健康检查");
    axum::serve(listener, app(pool))
        .with_graceful_shutdown(shutdown)
        .await
}

/// GET /health —— 不触碰 SAP，便于外部探活
async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// POST /api/rfc —— 通用 RFC 调用
///
/// 流程：反序列化请求 → spawn_blocking 内通过连接池执行（含自动重连）→ 返回 JSON 结果。
/// FFI 与连接池内部状态都被限制在阻塞闭包中，绝不跨 await 点。
async fn invoke_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
    Json(req): Json<InvokeRequest>,
) -> Result<Json<InvokeResponse>, RfcError> {
    let pool_clone = Arc::clone(&pool);
    let started = std::time::Instant::now();
    let func_name = req.func_name.clone();

    let resp = tokio::task::spawn_blocking(move || {
        // 通过 with_connection 执行：遇通信错误自动重连重试一次
        pool_clone.with_connection(|conn| execute_collect(conn, &req))
    })
    .await
    .map_err(|e| RfcError {
        code: -1,
        message: format!("阻塞任务失败: {}", e),
        key: String::new(),
    })??;

    tracing::info!(
        func = %func_name,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "RFC 调用完成"
    );

    Ok(Json(resp))
}
