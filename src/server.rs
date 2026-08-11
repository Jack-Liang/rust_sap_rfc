//! axum HTTP 服务：路由、handler、共享状态。
//!
//! 共享状态是 `Arc<RfcConnectionPool>`：多连接池 + 自动重连。
//! handler 内通过 `tokio::task::spawn_blocking` 把 FFI 执行丢到阻塞线程池，
//! 不同请求可拿到不同连接并行执行 SAP 调用；同时让非 Send 的裸指针类型
//! 只存在于阻塞闭包内，不跨 await 点，保证 future 干净 Send。

use crate::api::{
    direction_name, rfctype_name, DdicTypeResponse, FieldDef, FieldSemanticsResponse, FixedValueDto,
    FunctionDocResponse, FunctionInterface, FunctionParam, InvokeRequest, InvokeResponse, ParamDoc,
    ScalarValue, SearchFunctionEntry, SearchResponse,
};
use crate::connection::{get_field_infos, RfcConnection};
use crate::error::RfcError;
use crate::executor::execute_collect;
use crate::pool::RfcConnectionPool;
use axum::response::IntoResponse;
use axum::{routing::post, Json, Router};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// 全局共享状态：连接池（内部含连接 + 重连参数）
pub type SharedPool = Arc<RfcConnectionPool>;

/// 全局请求超时（启动期由 [`init_request_timeout`] 写入，默认 60s）。
static REQUEST_TIMEOUT: OnceLock<Duration> = OnceLock::new();

/// 启动期设置全局请求超时（main 调一次）。
pub fn init_request_timeout(d: Duration) {
    let _ = REQUEST_TIMEOUT.set(d);
}

/// 当前全局请求超时；未初始化时回退 60s。
fn request_timeout() -> Duration {
    REQUEST_TIMEOUT
        .get()
        .copied()
        .unwrap_or_else(|| Duration::from_secs(60))
}

/// Prometheus 指标记录器句柄（启动期由 [`init_metrics`] 写入）。
static METRICS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// 启动期初始化 Prometheus 指标记录器（main 调一次）。之后 `counter!`/`gauge!`/`histogram!` 才生效。
pub fn init_metrics() {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("安装 Prometheus 指标记录器失败");
    let _ = METRICS_HANDLE.set(handle);
}

/// 构造超时错误（504 Gateway Timeout，body 走 ErrorResponse）。
fn timeout_error(timeout: Duration) -> RfcError {
    RfcError {
        code: -1,
        status: 504,
        message: format!("SAP 调用超时（{}s）", timeout.as_secs()),
        ..Default::default()
    }
}

/// 用指定超时在阻塞线程池内执行 SAP 调用。
///
/// `spawn_blocking` + `with_connection`（含自动重连）+ `tokio::time::timeout` 收敛到一处。
/// 超时返回 504。注意：超时后 `spawn_blocking` 线程无法取消，FFI 会跑到 SAP 响应才归还
/// 连接（NWRFC 固有限制，靠协议层超时兜底）。FFI 与连接池内部状态都被限制在阻塞闭包中，
/// 绝不跨 await 点，保证 future 干净 Send。
async fn run_blocking_with_timeout<F, R>(
    pool: SharedPool,
    timeout: Duration,
    f: F,
) -> Result<R, RfcError>
where
    F: FnMut(&RfcConnection) -> Result<R, RfcError> + Send + 'static,
    R: Send + 'static,
{
    let pool = Arc::clone(&pool);
    let join = tokio::task::spawn_blocking(move || pool.with_connection(f));
    match tokio::time::timeout(timeout, join).await {
        Ok(inner) => inner.map_err(|e| RfcError {
            code: -1,
            message: format!("阻塞任务失败: {}", e),
            key: String::new(),
            ..Default::default()
        })?,
        Err(_elapsed) => Err(timeout_error(timeout)),
    }
}

/// 用全局默认超时执行（元数据查询等无需 per-request 超时的端点用这个）。
async fn run_blocking<F, R>(pool: SharedPool, f: F) -> Result<R, RfcError>
where
    F: FnMut(&RfcConnection) -> Result<R, RfcError> + Send + 'static,
    R: Send + 'static,
{
    run_blocking_with_timeout(pool, request_timeout(), f).await
}

/// 仅静态路由（不依赖 SAP 连接池）：首页 / Agent 文档 / 健康检查。
/// 供集成测试用，无需构造 pool 即可验证这几个端点。
pub fn static_app<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/", axum::routing::get(index_handler))
        .route("/agents.md", axum::routing::get(agents_handler))
        .route("/health", axum::routing::get(health_handler))
}

/// 构建带共享连接池的 Router（静态路由 + SAP 业务路由）
pub fn app(pool: SharedPool) -> Router {
    // 受认证保护的 /api 业务路由：设置 SAP_API_KEY 后要求 Bearer token
    let api = Router::new()
        .route("/api/rfc", post(invoke_handler))
        .route("/api/functions/search", post(search_functions_handler))
        .route("/api/functions/:name", axum::routing::get(function_interface_handler))
        .route("/api/functions/:name/doc", axum::routing::get(function_doc_handler))
        .route("/api/ddic/type/:name", axum::routing::get(ddic_type_handler))
        .route("/api/ddic/field/:table/:field", axum::routing::get(ddic_field_handler))
        .layer(axum::middleware::from_fn(crate::auth::require_api_key));

    // 探针与公开页免鉴权：编排系统探针不便带 token，且无业务数据泄露
    static_app()
        .route("/ready", axum::routing::get(ready_handler))
        .route("/metrics", axum::routing::get(metrics_handler))
        .merge(api)
        .fallback(fallback_handler)
        .with_state(pool)
}

/// 兜底 404：路由不存在时统一返回 JSON 错误体（而非 axum 默认的空 body）。
async fn fallback_handler() -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error":{"code":404,"message":"Not found","key":"ROUTE_NOT_FOUND"}})),
    )
}

/// `GET /metrics` —— Prometheus 指标（连接池 + RFC 调用计数/耗时）。免鉴权（运维探针）。
async fn metrics_handler() -> impl axum::response::IntoResponse {
    let body = METRICS_HANDLE
        .get()
        .map(|h| h.render())
        .unwrap_or_default();
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
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
    // listen_addr 可能是 0.0.0.0:3000，给用户提示时用 127.0.0.1 更友好（本机访问）
    let display_host = if listen_addr.starts_with("0.0.0.0") {
        listen_addr.replacen("0.0.0.0", "127.0.0.1", 1)
    } else if listen_addr.starts_with("::") {
        listen_addr.replacen("::", "[::1]", 1)
    } else {
        listen_addr.to_string()
    };
    tracing::info!("✅ 服务就绪！");
    tracing::info!("   👉 浏览器打开:         http://{}", display_host);
    tracing::info!("   👉 给 AI/Agent 的文档: http://{}/agents.md", display_host);
    tracing::info!("   端点速览: POST /api/rfc | GET /api/functions/:name | POST /api/functions/search");
    tracing::info!("           GET /api/functions/:name/doc | GET /api/ddic/type/:name | GET /api/ddic/field/:t/:f");
    axum::serve(
        listener,
        app(pool).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

/// GET /health —— 不触碰 SAP，便于外部探活（liveness）
async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// `GET /ready` —— readiness 探针：借连接池调 `RFC_PING` 验证 SAP 可达（带 5s 超时）。
///
/// 与 `/health`（liveness）分离：进程活着但连不上 SAP 时返回 503，编排系统
/// 据此摘流而非重启。失败统一用 503——语义最贴合 readiness（"暂时不可用，
/// 别给我流量"），故不走 `RfcError::IntoResponse` 的 502/504 映射。
///
/// 超时后 `spawn_blocking` 内的 ping 仍会跑完（无法取消），但 ping 本身很快；
/// 最坏占用一个连接几秒，探针频率（默认 10s）下可接受。
async fn ready_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
) -> (
    axum::http::StatusCode,
    Json<serde_json::Value>,
) {
    const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    let ping =
        tokio::time::timeout(READY_TIMEOUT, run_blocking(pool, |conn| conn.ping())).await;

    match ping {
        Ok(Ok(())) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "status": "ready", "sap": "ok" })),
        ),
        Ok(Err(e)) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "unavailable",
                "code": e.code,
                "message": e.message,
            })),
        ),
        Err(_elapsed) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "timeout",
                "timeout_ms": READY_TIMEOUT.as_millis() as u64,
            })),
        ),
    }
}

/// GET /agents.md —— 返回嵌入的 AGENTS.md（供 AI/Agent 读取）
/// 编译期 include_str! 嵌入，预编译包也自带，不依赖磁盘文件。
async fn agents_handler() -> axum::response::Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        include_str!("../AGENTS.md"),
    )
        .into_response()
}

/// GET / —— 浏览器欢迎页（含 Agent 文档入口 + 接口速览）
/// HTML 模板见 src/index.html（编译期 include_str! 嵌入，不依赖磁盘文件）。
/// 从请求 Host 头动态推导访问地址，链接自动匹配用户实际访问的 host:port。
async fn index_handler(req: axum::http::Request<axum::body::Body>) -> axum::response::Html<String> {
    // 从 Host 头取访问地址（如 127.0.0.1:3000 或 192.168.1.5:3000）
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("127.0.0.1:3000");
    let base = format!("http://{host}");
    let agents_url = format!("{base}/agents.md");

    // 认证状态：用于首页条件显示认证提示条
    let auth_visibility = if crate::auth::is_enabled() {
        "block"
    } else {
        "none"
    };
    let html = include_str!("index.html")
        .replace("{{BASE_URL}}", &base)
        .replace("{{AGENTS_URL}}", &agents_url)
        .replace("{{AUTH_BANNER_VISIBILITY}}", auth_visibility);
    axum::response::Html(html)
}

/// 审计用：把请求参数压缩成可读摘要（敏感值脱敏、长值截断、键排序稳定）。
fn summarize_params(req: &InvokeRequest) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !req.inputs.is_empty() {
        let mut keys: Vec<&String> = req.inputs.keys().collect();
        keys.sort();
        let kvs: Vec<String> = keys
            .iter()
            .map(|k| format!("{}={}", k, mask_value(k, &req.inputs[*k])))
            .collect();
        parts.push(format!("inputs{{{}}}", kvs.join(", ")));
    }
    if !req.table_inputs.is_empty() {
        let mut keys: Vec<&String> = req.table_inputs.keys().collect();
        keys.sort();
        let tabs: Vec<String> = keys
            .iter()
            .map(|k| format!("{}[{}行]", k, req.table_inputs[*k].len()))
            .collect();
        parts.push(format!("tables{{{}}}", tabs.join(", ")));
    }
    if !req.struct_inputs.is_empty() {
        let mut keys: Vec<&String> = req.struct_inputs.keys().collect();
        keys.sort();
        let sts: Vec<String> = keys
            .iter()
            .map(|k| format!("{}{{{}}}", k, req.struct_inputs[*k].len()))
            .collect();
        parts.push(format!("structs{{{}}}", sts.join(", ")));
    }
    if parts.is_empty() {
        "(无参数)".into()
    } else {
        parts.join(" ")
    }
}

/// 脱敏 + 截断单个标量值（用于审计摘要）。敏感 key（密码/token 等）→ `***`，长值截断 80 字符。
fn mask_value(key: &str, value: &ScalarValue) -> String {
    const SENSITIVE: &[&str] = &[
        "PASSWD", "PASSWORD", "PASS", "SECRET", "TOKEN", "CREDENTIAL", "KEY",
    ];
    let upper = key.to_uppercase();
    if SENSITIVE.iter().any(|s| upper.contains(s)) {
        return "***".into();
    }
    let s = value.clone().into_chars();
    const MAX: usize = 80;
    if s.chars().count() > MAX {
        format!("{}…", s.chars().take(MAX).collect::<String>())
    } else {
        s
    }
}

/// POST /api/rfc —— 通用 RFC 调用
///
/// 流程：反序列化请求 → spawn_blocking 内通过连接池执行（含自动重连）→ 返回 JSON 结果。
/// FFI 与连接池内部状态都被限制在阻塞闭包中，绝不跨 await 点。
async fn invoke_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    req: Result<Json<InvokeRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<InvokeResponse>, RfcError> {
    let started = std::time::Instant::now();
    let caller_ip = addr.ip().to_string();
    // JSON 解析/反序列化失败 → 统一错误格式（status 取 axum 语义码，body_text 作 message）
    let Json(req) = req.map_err(|r| RfcError {
        code: -1,
        status: r.status().as_u16(),
        message: r.body_text(),
        key: "JSON_INVALID".into(),
    })?;
    let func_name = req.func_name.clone();
    let params = summarize_params(&req);
    let pool_stats = pool.stats(); // 采样当前池状态（pool 即将 move 进闭包）

    // per-request 超时：调用方可对慢接口自主放宽；不传/传 0 → 用全局默认
    let timeout = req
        .timeout_secs
        .filter(|&s| s >= 1)
        .map(Duration::from_secs)
        .unwrap_or_else(request_timeout);
    // 通过 with_connection 执行：遇通信错误自动重连重试一次
    let result =
        run_blocking_with_timeout(pool, timeout, move |conn| execute_collect(conn, &req)).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    // 指标：池状态 gauge（每次调用采样）+ 调用计数/耗时（按 函数×结果 分维）
    gauge!("pool_idle").set(pool_stats.idle as f64);
    gauge!("pool_total").set(pool_stats.total as f64);
    gauge!("pool_max").set(pool_stats.max as f64);

    // 审计日志 + 指标：成功 info / 失败 warn（失败 = 告警信号）
    match result {
        Ok(resp) => {
            counter!("rfc_calls_total", "func" => func_name.clone(), "result" => "ok").increment(1);
            histogram!("rfc_call_duration_ms", "func" => func_name.clone()).record(elapsed_ms as f64);
            tracing::info!(
                func = %func_name,
                caller_ip = %caller_ip,
                elapsed_ms,
                params = %params,
                "RFC 调用成功"
            );
            Ok(Json(resp))
        }
        Err(e) => {
            counter!("rfc_calls_total", "func" => func_name.clone(), "result" => "err").increment(1);
            histogram!("rfc_call_duration_ms", "func" => func_name.clone()).record(elapsed_ms as f64);
            tracing::warn!(
                func = %func_name,
                caller_ip = %caller_ip,
                elapsed_ms,
                params = %params,
                status = e.status,
                code = e.code,
                key = %e.key,
                message = %e.message,
                "RFC 调用失败"
            );
            Err(e)
        }
    }
}

// ========================================================================
// 面向 AI 的元数据查询 handler（端点 ①~⑤）
// ========================================================================

/// 默认语言（从 SAP_LANG 环境变量读，回退 EN）。
/// 端点⑤④可用 ?lang= 覆盖。
fn default_lang() -> String {
    std::env::var("SAP_LANG").unwrap_or_else(|_| "EN".to_string())
}

/// 把 ParamInfo 转成 FieldDef，STRUCTURE/TABLE 类型递归展开子字段（深度上限由 get_field_infos 的句柄链决定）。
fn param_info_to_field_def(
    p: &crate::connection::ParamInfo,
) -> Result<FieldDef, RfcError> {
    let fields = if (p.type_ == crate::ffi::RFCTYPE_TABLE
        || p.type_ == crate::ffi::RFCTYPE_STRUCTURE)
        && p.type_desc_handle.is_some()
    {
        // SAFETY: type_desc_handle 来自刚拉取的有效元数据，连接仍有效
        let subs = unsafe { get_field_infos(p.type_desc_handle.unwrap()) }?;
        let defs: Vec<Box<FieldDef>> = subs
            .iter()
            .map(|sf| {
                Box::new(FieldDef {
                    name: sf.name.clone(),
                    type_name: rfctype_name(sf.type_),
                    length: sf.char_length,
                    decimals: sf.decimals,
                    description: sf.parameter_text.clone(),
                    fields: None, // 深度递归由 metadata 缓存负责；此处仅展开一层供 AI 快速预览
                })
            })
            .collect::<Vec<_>>();
        Some(defs)
    } else {
        None
    };
    Ok(FieldDef {
        name: p.name.clone(),
        type_name: rfctype_name(p.type_),
        length: p.char_length,
        decimals: p.decimals,
        description: p.parameter_text.clone(),
        fields,
    })
}

/// ① GET /api/functions/:name —— 查函数完整接口（参数/类型/方向/嵌套字段）
async fn function_interface_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<FunctionInterface>, RfcError> {
    crate::api::validate_func_name(&name)?;
    let result = run_blocking(pool, move |conn| {
        let param_infos = conn.get_param_infos(&name)?;
        let params: Vec<FunctionParam> = param_infos
            .iter()
            .map(|p| {
                Ok(FunctionParam {
                    name: p.name.clone(),
                    type_name: rfctype_name(p.type_),
                    direction: direction_name(p.direction),
                    length: p.char_length,
                    decimals: p.decimals,
                    optional: p.optional,
                    default: p.default_value.clone(),
                    description: p.parameter_text.clone(),
                    fields: param_info_to_field_def(p)?.fields,
                }) as Result<FunctionParam, RfcError>
            })
            .collect::<Result<_, _>>()?;
        Ok(FunctionInterface {
            name: name.clone(),
            params,
        })
    })
    .await?;
    Ok(Json(result))
}

/// ② POST /api/functions/search —— 搜索函数模块
#[derive(serde::Deserialize)]
pub(crate) struct SearchRequest {
    /// 函数名通配符，如 "BAPI_USER_*"
    #[serde(default)]
    pattern: String,
    /// 函数组过滤（可选）
    #[serde(default)]
    group: String,
    /// 最多返回条数，默认 50
    #[serde(default)]
    max_results: Option<usize>,
}
/// ② POST /api/functions/search —— 搜索函数模块
async fn search_functions_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
    req: Result<Json<SearchRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<SearchResponse>, RfcError> {
    let Json(req) = req.map_err(|r| RfcError {
        code: -1,
        status: r.status().as_u16(),
        message: r.body_text(),
        key: "JSON_INVALID".into(),
    })?;
    // 空 pattern 校验：pattern + group 都空 → 拒绝（防无意义枚举全库）
    if req.pattern.trim().is_empty() && req.group.trim().is_empty() {
        return Err(RfcError {
            code: -1,
            status: 400,
            message: "pattern 和 group 不能同时为空（至少提供一个过滤条件）".into(),
            key: "PATTERN_EMPTY".into(),
        });
    }
    let max = req.max_results.unwrap_or(50).min(500);
    let pattern = req.pattern.clone();
    let functions = run_blocking(pool, move |conn| {
        crate::discovery::search_functions(conn, &req.pattern, &req.group, max)
    })
    .await?;
    let count = functions.len();
    let functions = functions
        .into_iter()
        .map(|f| SearchFunctionEntry {
            name: f.name,
            group: f.group,
            description: f.description,
        })
        .collect();
    Ok(Json(SearchResponse {
        pattern,
        count,
        functions,
    }))
}

/// ③ GET /api/ddic/type/:name —— 查 DDIC 结构/表的字段定义
async fn ddic_type_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<DdicTypeResponse>, RfcError> {
    let req_name = name.clone();
    let result =
        run_blocking(pool, move |conn| crate::metadata::get_type_fields(conn, &name)).await?;
    let fields = result.iter().map(FieldDef::from_type_field).collect();
    Ok(Json(DdicTypeResponse {
        name: req_name,
        fields,
    }))
}

/// ④ GET /api/ddic/field/:table/:field —— 查字段的语义元数据（数据元素/域/固定值）
#[derive(serde::Deserialize)]
struct LangQuery {
    #[serde(default)]
    lang: Option<String>,
}
/// ④ GET /api/ddic/field/:table/:field —— 查字段的语义元数据（数据元素/域/固定值）
async fn ddic_field_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
    axum::extract::Path((table, field)): axum::extract::Path<(String, String)>,
    axum::extract::Query(q): axum::extract::Query<LangQuery>,
) -> Result<Json<FieldSemanticsResponse>, RfcError> {
    let lang = q.lang.unwrap_or_else(default_lang);
    let req_table = table.clone();
    let sem = run_blocking(pool, move |conn| {
        crate::discovery::read_ddic_field_info(conn, &table, &field, &lang)
    })
    .await?;
    Ok(Json(FieldSemanticsResponse {
        table: req_table,
        field: sem.field,
        data_element: sem.data_element,
        domain: sem.domain,
        check_table: sem.check_table,
        description: sem.description,
        medium_label: sem.medium_label,
        fixed_values: sem
            .fixed_values
            .into_iter()
            .map(|fv| FixedValueDto {
                value: fv.value,
                text: fv.text,
            })
            .collect(),
    }))
}

/// ⑤ GET /api/functions/:name/doc —— 查函数文档（短文本 + SE37 长文本 + 参数说明）
async fn function_doc_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<LangQuery>,
) -> Result<Json<FunctionDocResponse>, RfcError> {
    crate::api::validate_func_name(&name)?;
    let lang = q.lang.unwrap_or_else(default_lang);
    let result = run_blocking(pool, move |conn| {
        // 先取参数描述（parameterText 作为参数文档），同时取短文本
        let param_infos = conn.get_param_infos(&name)?;
        let parameter_docs: Vec<ParamDoc> = param_infos
            .iter()
            .filter(|p| !p.parameter_text.is_empty())
            .map(|p| ParamDoc {
                name: p.name.clone(),
                text: p.parameter_text.clone(),
            })
            .collect();
        // 短文本：从任一参数的描述或元数据取（此处用首个参数描述作 fallback）
        let short_text = param_infos
            .first()
            .map(|p| p.parameter_text.clone())
            .unwrap_or_default();
        // 读 SE37 长文档（失败降级为空 + warning）
        let doc = crate::discovery::read_function_doc(conn, &name, &lang, &short_text)?;
        Ok(FunctionDocResponse {
            name: name.clone(),
            short_text: doc.short_text,
            long_text: doc.long_text,
            warning: doc.warning,
            parameter_docs,
        })
    })
    .await?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[test]
    fn timeout_error_is_504() {
        let e = timeout_error(Duration::from_secs(60));
        assert_eq!(e.status, 504, "超时错误应为 504");
        assert!(
            e.message.contains("60"),
            "消息应含超时秒数: {}",
            e.message
        );
    }

    #[test]
    fn mask_value_redacts_sensitive_keys() {
        let secret = ScalarValue::Chars("s3cr3t".into());
        assert_eq!(mask_value("PASSWORD", &secret), "***");
        assert_eq!(mask_value("USER_PASSWD", &secret), "***");
        assert_eq!(mask_value("API_KEY", &secret), "***");
        assert_eq!(mask_value("TOKEN", &secret), "***");
        // 非敏感 key 保留值
        assert_eq!(mask_value("REQUTEXT", &ScalarValue::Chars("hi".into())), "hi");
        assert_eq!(mask_value("MAX_ROWS", &ScalarValue::Int(100)), "100");
    }

    #[test]
    fn mask_value_truncates_long() {
        let long = ScalarValue::Chars("x".repeat(100));
        let m = mask_value("REQUTEXT", &long);
        assert!(m.ends_with('…'), "长值应以省略号结尾: {}", m);
        assert_eq!(m.chars().count(), 81); // 80 个 x + …
    }

    #[test]
    fn summarize_params_redacts_and_structures() {
        let mut req = InvokeRequest::default();
        req.func_name = "STFC_CONNECTION".into();
        req.inputs
            .insert("REQUTEXT".into(), ScalarValue::Chars("hi".into()));
        req.inputs
            .insert("PASSWORD".into(), ScalarValue::Chars("secret".into()));
        let s = summarize_params(&req);
        assert!(s.contains("inputs{"), "应含 inputs 块: {}", s);
        assert!(s.contains("REQUTEXT=hi"), "应含明文值: {}", s);
        assert!(s.contains("PASSWORD=***"), "密码应脱敏: {}", s);
        assert!(!s.contains("secret"), "不应泄露明文密码: {}", s);
    }

    #[test]
    fn summarize_params_empty() {
        let req = InvokeRequest::default();
        assert_eq!(summarize_params(&req), "(无参数)");
    }

    /// 读取 axum 响应 body 为 String
    async fn body_string(body: axum::body::Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let resp = static_app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("\"status\":\"ok\""));
    }

    #[tokio::test]
    async fn agents_md_returns_markdown() {
        let resp = static_app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/agents.md")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let ct = resp.headers().get(axum::http::header::CONTENT_TYPE).unwrap();
        assert!(ct.to_str().unwrap().contains("text/markdown"));
        let body = body_string(resp.into_body()).await;
        assert!(!body.is_empty());
        // 内容应包含项目名（验证编译期嵌入成功）
        assert!(body.contains("rust_sap_rfc") || body.contains("SAP"));
    }

    #[tokio::test]
    async fn index_html_replaces_base_url_from_host_header() {
        let resp = static_app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .header("host", "192.168.1.5:9999")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        // Host 头应被替换进模板（不再含占位符）
        assert!(!body.contains("{{BASE_URL}}"));
        assert!(!body.contains("{{AGENTS_URL}}"));
        assert!(body.contains("http://192.168.1.5:9999"));
    }

    #[tokio::test]
    async fn index_html_defaults_when_no_host_header() {
        let resp = static_app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("http://127.0.0.1:3000"));
    }
}
