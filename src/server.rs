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
        .route("/", axum::routing::get(index_handler))
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

/// GET / —— 浏览器欢迎页（无 AI 风格：纯文本、内联最简样式）
async fn index_handler() -> axum::response::Html<&'static str> {
    axum::response::Html(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>rust-sap-rfc</title>
<style>
  body { font: 14px/1.5 -apple-system, "Segoe UI", monospace; max-width: 720px; margin: 40px auto; padding: 0 16px; color: #222; }
  h1 { font-size: 18px; margin-bottom: 4px; }
  p.lede { color: #666; margin-top: 0; }
  h2 { font-size: 14px; margin-top: 28px; border-bottom: 1px solid #ddd; padding-bottom: 4px; }
  code { background: #f4f4f4; padding: 1px 4px; }
  pre { background: #f4f4f4; padding: 12px; overflow-x: auto; font-size: 12.5px; }
  table { border-collapse: collapse; }
  td, th { padding: 4px 12px 4px 0; text-align: left; vertical-align: top; }
  th { color: #666; font-weight: normal; }
</style>
</head>
<body>
<h1>rust-sap-rfc</h1>
<p class="lede">SAP NWRFC → REST 网关服务。POST /api/rfc 调用任意 BAPI。</p>

<h2>端点</h2>
<table>
<tr><th>GET&nbsp;&nbsp;</th><td><code>/</code></td><td>本页面</td></tr>
<tr><th></th><td><code>/health</code></td><td>健康检查，返回 <code>{"status":"ok"}</code>（不触碰 SAP）</td></tr>
<tr><th>POST</th><td><code>/api/rfc</code></td><td>通用 RFC 调用，详见 README §5</td></tr>
</table>

<h2>连通测试</h2>
<pre>curl -X POST http://127.0.0.1:3000/api/rfc \
  -H "Content-Type: application/json" \
  -d '{"func_name":"STFC_CONNECTION","inputs":{"REQUTEXT":"hi"},"string_outputs":{"ECHOTEXT":255,"RESPTEXT":255}}'</pre>

<p>更多字段说明、调用示例、BAPI 速查见项目 <code>README.md</code>。</p>
</body>
</html>"#,
    )
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
