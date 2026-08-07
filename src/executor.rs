//! 通用 RFC 执行器：根据 `InvokeRequest` 完成
//! "取函数 → 填参数 → invoke → 读结果" 全过程，返回结构化 `InvokeResponse`。
//!
//! 字段长度与类型自动发现：当请求中长度未指定时，通过 metadata 模块从 SAP
//! 函数描述查询；输入按调用方给出的 ScalarValue 变体选对应 RfcSetXxx。

use crate::api::{execute_invoke, InvokeRequest, InvokeResponse};
use crate::connection::RfcConnection;
use crate::error::RfcError;

/// 通用执行入口。委托给 api::execute_invoke，并注入元数据解析函数。
pub fn execute_collect(
    conn: &RfcConnection,
    req: &InvokeRequest,
) -> Result<InvokeResponse, RfcError> {
    execute_invoke(conn, req, resolve_meta)
}

/// 元数据解析回调：对 string_outputs 查标量参数元数据，对 table_outputs 查表字段元数据。
/// 任何元数据查询失败都静默回退到 DEFAULT_CHAR_LEN（不阻断主流程），
/// 因为 get_chars 本身已支持自适应重试。
fn resolve_meta(conn: &RfcConnection, req: &InvokeRequest) -> crate::api::ResolvedMeta {
    use std::collections::HashMap;

    // 标量输出：参数名 -> (字符长度, 类型)
    let mut scalars: HashMap<String, (usize, i32)> = HashMap::new();
    // string_outputs 和 auto_outputs 都需要元数据（类型+长度）
    for name in req.string_outputs.keys().chain(req.auto_outputs.iter()) {
        let meta = crate::metadata::param_meta(conn, &req.func_name, name);
        let entry = match meta {
            Some(m) => (m.char_length, m.type_),
            None => (crate::api::DEFAULT_CHAR_LEN, crate::ffi::RFCTYPE_CHAR),
        };
        scalars.insert(name.clone(), entry);
    }

    // 表输出：表名 -> {字段名 -> (字符长度, 类型)}
    let mut tables: HashMap<String, HashMap<String, (usize, i32)>> = HashMap::new();
    for table_name in req.table_outputs.keys() {
        let field_map = crate::metadata::table_field_metas(conn, &req.func_name, table_name)
            .unwrap_or_default()
            .into_iter()
            .map(|(k, m)| (k, (m.char_length, m.type_)))
            .collect();
        tables.insert(table_name.clone(), field_map);
    }

    crate::api::ResolvedMeta { scalars, tables }
}
