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
use axum::response::IntoResponse;
use axum::{routing::post, Json, Router};
use std::sync::Arc;

/// 全局共享状态：连接池（内部含连接 + 重连参数）
pub type SharedPool = Arc<RfcConnectionPool>;

/// 构建带共享连接池的 Router
pub fn app(pool: SharedPool) -> Router {
    Router::new()
        .route("/", axum::routing::get(index_handler))
        .route("/agents.md", axum::routing::get(agents_handler))
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
    axum::serve(listener, app(pool))
        .with_graceful_shutdown(shutdown)
        .await
}

/// GET /health —— 不触碰 SAP，便于外部探活
async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
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

    let html = format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>rust-sap-rfc · SAP NWRFC REST 网关</title>
<style>
  * {{ box-sizing: border-box; }}
  body {{ font: 15px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; max-width: 820px; margin: 0 auto; padding: 40px 20px 60px; color: #1f2328; background: #fafbfc; }}
  a {{ color: #0969da; text-decoration: none; }}
  a:hover {{ text-decoration: underline; }}
  code {{ font-family: "SF Mono", Menlo, Consolas, monospace; font-size: 13px; background: #eff1f3; padding: 2px 6px; border-radius: 4px; }}
  pre {{ background: #161b22; color: #e6edf3; padding: 16px; border-radius: 8px; overflow-x: auto; font-size: 13px; line-height: 1.5; }}
  pre code {{ background: none; color: inherit; padding: 0; }}

  /* Hero */
  .hero {{ background: #fff; border: 1px solid #d0d7de; border-radius: 12px; padding: 32px; margin-bottom: 24px; }}
  .hero h1 {{ margin: 0 0 8px; font-size: 26px; font-weight: 700; }}
  .hero .lede {{ margin: 0 0 4px; color: #57606a; font-size: 15px; }}

  /* Agent 卡片 */
  .agent {{ background: linear-gradient(135deg, #ddf4ff 0%, #dafbe1 100%); border: 1px solid #54aeff; border-radius: 12px; padding: 24px; margin-bottom: 32px; }}
  .agent-title {{ display: flex; align-items: center; gap: 10px; font-size: 18px; font-weight: 700; color: #0969da; margin: 0 0 8px; }}
  .agent-title svg {{ flex-shrink: 0; }}
  .agent p {{ margin: 8px 0; color: #1f2328; }}
  .agent-url {{ display: inline-flex; align-items: center; gap: 8px; background: #fff; border: 1px solid #54aeff; padding: 8px 8px 8px 16px; border-radius: 8px; font-family: monospace; font-size: 14px; font-weight: 600; word-break: break-all; }}
  .agent-url a {{ color: #0969da; }}
  .copy-btn {{ flex-shrink: 0; display: inline-flex; align-items: center; gap: 4px; background: #0969da; color: #fff; border: none; border-radius: 6px; padding: 6px 10px; font-size: 12px; font-weight: 600; cursor: pointer; transition: background .15s; }}
  .copy-btn:hover {{ background: #0860c9; }}
  .copy-btn.copied {{ background: #1a7f37; }}
  .copy-btn svg {{ width: 14px; height: 14px; }}
  .agent .hint {{ font-size: 13px; color: #57606a; }}

  /* 章节标题 */
  h2.section {{ font-size: 14px; font-weight: 600; color: #57606a; text-transform: uppercase; letter-spacing: 0.5px; margin: 36px 0 12px; }}

  /* 端点卡片网格 */
  .grid {{ display: grid; grid-template-columns: 1fr; gap: 10px; }}
  .ep {{ display: flex; align-items: center; gap: 12px; background: #fff; border: 1px solid #d0d7de; border-radius: 8px; padding: 12px 16px; transition: box-shadow .15s; }}
  .ep:hover {{ box-shadow: 0 1px 6px rgba(0,0,0,.08); }}
  .method {{ display: inline-block; min-width: 48px; text-align: center; font-size: 11px; font-weight: 700; padding: 3px 8px; border-radius: 4px; flex-shrink: 0; }}
  .m-get {{ background: #dafbe1; color: #1a7f37; border: 1px solid #4ac26b; }}
  .m-post {{ background: #ddf4ff; color: #0969da; border: 1px solid #54aeff; }}
  .ep code {{ background: none; padding: 0; font-size: 13px; font-weight: 500; color: #24292f; }}
  .ep .desc {{ margin-left: auto; font-size: 13px; color: #57606a; text-align: right; }}

  footer {{ margin-top: 40px; padding-top: 20px; border-top: 1px solid #d0d7de; font-size: 13px; color: #57606a; }}
  footer a {{ margin-right: 16px; }}

  @media (max-width: 600px) {{
    .ep {{ flex-wrap: wrap; }}
    .ep .desc {{ margin-left: 0; width: 100%; text-align: left; color: #8c959f; font-size: 12px; }}
    .hero h1 {{ font-size: 22px; }}
  }}
</style>
</head>
<body>

<div class="hero">
  <h1>rust-sap-rfc</h1>
  <p class="lede">SAP NWRFC → REST 网关服务 · 一个端点调用任意 BAPI，5 个元数据端点供 AI 自主探索</p>
</div>

<div class="agent">
  <div class="agent-title">
    <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="10" rx="2"/><circle cx="12" cy="5" r="2"/><path d="M12 7v4"/><line x1="8" y1="16" x2="8" y2="16"/><line x1="16" y1="16" x2="16" y2="16"/></svg>
    给 AI / Agent 用？
  </div>
  <p>把这个链接直接粘贴给 Claude / GPT 等 Agent，它就能自主搜索函数、查参数、调 SAP：</p>
  <p><span class="agent-url"><a href="/agents.md" id="agent-link">{agents_url}</a><button class="copy-btn" onclick="copyLink(this)" title="复制链接"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>复制</button></span></p>
  <p class="hint">Agent 读取这份文档后，就知道有哪些端点、怎么调、操作流程</p>
</div>

<h2 class="section">通用调用</h2>
<div class="grid">
  <div class="ep"><span class="method m-post">POST</span><code>/api/rfc</code><span class="desc">通用 RFC 调用</span></div>
</div>

<h2 class="section">面向 AI 的元数据 API</h2>
<div class="grid">
  <div class="ep"><span class="method m-get">GET</span><code>/api/functions/&#123;name&#125;</code><span class="desc">查函数接口（参数/类型/嵌套字段）</span></div>
  <div class="ep"><span class="method m-post">POST</span><code>/api/functions/search</code><span class="desc">搜索函数模块</span></div>
  <div class="ep"><span class="method m-get">GET</span><code>/api/functions/&#123;name&#125;/doc</code><span class="desc">查函数文档（短文本 + SE37 长文档）</span></div>
  <div class="ep"><span class="method m-get">GET</span><code>/api/ddic/type/&#123;name&#125;</code><span class="desc">查 DDIC 结构/表字段定义</span></div>
  <div class="ep"><span class="method m-get">GET</span><code>/api/ddic/field/&#123;table&#125;/&#123;field&#125;</code><span class="desc">查字段语义（数据元素/域/固定值）</span></div>
</div>

<h2 class="section">其他端点</h2>
<div class="grid">
  <div class="ep"><span class="method m-get">GET</span><code>/</code><span class="desc">本页面</span></div>
  <div class="ep"><span class="method m-get">GET</span><code>/agents.md</code><span class="desc">AI/Agent 操作文档</span></div>
  <div class="ep"><span class="method m-get">GET</span><code>/health</code><span class="desc">健康检查（不触碰 SAP）</span></div>
</div>

<h2 class="section">连通测试</h2>
<pre><code>curl -X POST {base}/api/rfc \
  -H "Content-Type: application/json" \
  -d '{{"func_name":"STFC_CONNECTION","inputs":{{"REQUTEXT":"hi"}},"string_outputs":{{"ECHOTEXT":255,"RESPTEXT":255}}}}'</code></pre>

<footer>
  <a href="https://github.com/Jack-Liang/rust_sap_rfc">项目源码</a>
  <a href="https://github.com/Jack-Liang/rust_sap_rfc/issues">问题反馈</a>
  <span style="float:right">完整文档见 <code>README.md</code> / <code>AGENTS.md</code></span>
</footer>

<script>
function copyLink(btn) {{
  const url = document.getElementById('agent-link').href;
  navigator.clipboard.writeText(url).then(function() {{
    const orig = btn.innerHTML;
    btn.classList.add('copied');
    btn.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>已复制';
    setTimeout(function() {{ btn.classList.remove('copied'); btn.innerHTML = orig; }}, 2000);
  }}).catch(function() {{
    // clipboard API 不可用（如非 HTTPS/localhost），降级选中文本让用户 Cmd+C
    const range = document.createRange(); range.selectNode(document.getElementById('agent-link'));
    window.getSelection().removeAllRanges(); window.getSelection().addRange(range);
    btn.textContent = '已选中，按 Cmd/Ctrl+C';
  }});
}}
</script>

</body>
</html>"#
    );
    axum::response::Html(html)
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
