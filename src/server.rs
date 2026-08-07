//! axum HTTP 服务：路由、handler、共享状态。
//!
//! 共享状态是 `Arc<RfcConnectionPool>`：多连接池 + 自动重连。
//! handler 内通过 `tokio::task::spawn_blocking` 把 FFI 执行丢到阻塞线程池，
//! 不同请求可拿到不同连接并行执行 SAP 调用；同时让非 Send 的裸指针类型
//! 只存在于阻塞闭包内，不跨 await 点，保证 future 干净 Send。

use crate::api::{
    direction_name, rfctype_name, DdicTypeResponse, FieldDef, FieldSemanticsResponse, FixedValueDto,
    FunctionDocResponse, FunctionInterface, FunctionParam, InvokeRequest, InvokeResponse, ParamDoc,
    SearchFunctionEntry, SearchResponse,
};
use crate::connection::get_field_infos;
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
        .route("/health", axum::routing::get(health_handler))
        // 通用 RFC 调用
        .route("/api/rfc", post(invoke_handler))
        // 面向 AI 的元数据查询端点（①~⑤）
        .route(
            "/api/functions/search",
            post(search_functions_handler),
        )
        .route(
            "/api/functions/:name",
            axum::routing::get(function_interface_handler),
        )
        .route(
            "/api/functions/:name/doc",
            axum::routing::get(function_doc_handler),
        )
        .route(
            "/api/ddic/type/:name",
            axum::routing::get(ddic_type_handler),
        )
        .route(
            "/api/ddic/field/:table/:field",
            axum::routing::get(ddic_field_handler),
        )
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
    tracing::info!("  - POST /api/rfc                  通用 RFC 调用");
    tracing::info!("  - GET  /api/functions/:name      查函数接口(参数/类型/方向)");
    tracing::info!("  - POST /api/functions/search     搜索函数模块");
    tracing::info!("  - GET  /api/functions/:name/doc  查函数文档");
    tracing::info!("  - GET  /api/ddic/type/:name      查 DDIC 类型字段");
    tracing::info!("  - GET  /api/ddic/field/:t/:f     查字段语义元数据");
    tracing::info!("  - GET  /health                   健康检查");
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
        let defs: Vec<FieldDef> = subs
            .iter()
            .map(|sf| {
                Ok(FieldDef {
                    name: sf.name.clone(),
                    type_name: rfctype_name(sf.type_),
                    length: sf.char_length,
                    decimals: sf.decimals,
                    description: sf.parameter_text.clone(),
                    fields: None, // 深度递归由 metadata 缓存负责；此处仅展开一层供 AI 快速预览
                }) as Result<FieldDef, RfcError>
            })
            .collect::<Result<_, _>>()?;
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
    let pool_clone = Arc::clone(&pool);
    let result = tokio::task::spawn_blocking(move || {
        pool_clone.with_connection(|conn| {
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
            }) as Result<FunctionInterface, RfcError>
        })
    })
    .await
    .map_err(|e| RfcError {
        code: -1,
        message: format!("阻塞任务失败: {}", e),
        key: String::new(),
    })??;
    Ok(Json(result))
}

/// ② POST /api/functions/search —— 搜索函数模块
#[derive(serde::Deserialize)]
struct SearchRequest {
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
async fn search_functions_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, RfcError> {
    let max = req.max_results.unwrap_or(50);
    let pattern = req.pattern.clone();
    let pool_clone = Arc::clone(&pool);
    let functions = tokio::task::spawn_blocking(move || {
        pool_clone.with_connection(|conn| {
            crate::discovery::search_functions(conn, &req.pattern, &req.group, max)
        })
    })
    .await
    .map_err(|e| RfcError {
        code: -1,
        message: format!("阻塞任务失败: {}", e),
        key: String::new(),
    })??;
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
    let pool_clone = Arc::clone(&pool);
    let result = tokio::task::spawn_blocking(move || {
        pool_clone.with_connection(|conn| crate::metadata::get_type_fields(conn, &name))
    })
    .await
    .map_err(|e| RfcError {
        code: -1,
        message: format!("阻塞任务失败: {}", e),
        key: String::new(),
    })??;
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
async fn ddic_field_handler(
    axum::extract::State(pool): axum::extract::State<SharedPool>,
    axum::extract::Path((table, field)): axum::extract::Path<(String, String)>,
    axum::extract::Query(q): axum::extract::Query<LangQuery>,
) -> Result<Json<FieldSemanticsResponse>, RfcError> {
    let lang = q.lang.unwrap_or_else(default_lang);
    let req_table = table.clone();
    let pool_clone = Arc::clone(&pool);
    let sem = tokio::task::spawn_blocking(move || {
        pool_clone
            .with_connection(|conn| crate::discovery::read_ddic_field_info(conn, &table, &field, &lang))
    })
    .await
    .map_err(|e| RfcError {
        code: -1,
        message: format!("阻塞任务失败: {}", e),
        key: String::new(),
    })??;
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
    let lang = q.lang.unwrap_or_else(default_lang);
    let pool_clone = Arc::clone(&pool);
    let result = tokio::task::spawn_blocking(move || {
        pool_clone.with_connection(|conn| {
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
            }) as Result<FunctionDocResponse, RfcError>
        })
    })
    .await
    .map_err(|e| RfcError {
        code: -1,
        message: format!("阻塞任务失败: {}", e),
        key: String::new(),
    })??;
    Ok(Json(result))
}
