//! OpenAPI 3.0 文档定义
//!
//! 通过 `utoipa` 生成 spec，配合 `utoipa-swagger-ui` 在 `/swagger-ui/` 暴露交互式文档。
//! 这里聚合所有 #[utoipa::path] 标注的端点（paths）和响应 DTO（components）。

use utoipa::OpenApi;

use crate::api::{
    DdicTypeResponse, FieldSemanticsResponse, FieldDef, FixedValueDto, FunctionDocResponse,
    FunctionInterface, FunctionParam, InvokeRequest, InvokeResponse, ParamDoc, SearchFunctionEntry,
    SearchResponse,
};
use crate::error::{ErrorBody, ErrorResponse};
use crate::server::SearchRequest;

// ApiDoc 当前未引用（/openapi.json 路由因 utoipa 5.x 在 axum 0.7 下的递归栈溢出 bug 暂时禁用），
// 保留以便 bug 修复后直接复用。
#[allow(dead_code)]
#[derive(OpenApi)]
#[openapi(
    info(
        title = "rust-sap-rfc",
        version = "0.2.0",
        description = "SAP NWRFC → REST 网关服务。POST /api/rfc 调用任意 BAPI，5 个元数据端点供 AI 自主探索。"
    ),
    paths(
        crate::server::invoke_handler,
        crate::server::function_interface_handler,
        crate::server::search_functions_handler,
        crate::server::ddic_type_handler,
        crate::server::ddic_field_handler,
        crate::server::function_doc_handler,
    ),
    components(schemas(
        InvokeRequest,
        InvokeResponse,
        SearchRequest,
        SearchResponse,
        SearchFunctionEntry,
        FunctionInterface,
        FunctionParam,
        FieldDef,
        DdicTypeResponse,
        FieldSemanticsResponse,
        FixedValueDto,
        FunctionDocResponse,
        ParamDoc,
        ErrorResponse,
        ErrorBody,
    )),
    tags(
        (name = "通用调用", description = "POST /api/rfc 调用任意 SAP BAPI/RFC"),
        (name = "元数据查询", description = "面向 AI/Agent 的自服务探索端点")
    )
)]
pub struct ApiDoc;
