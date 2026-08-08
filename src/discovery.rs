//! 面向 AI 的 SAP 元数据发现：封装对 ABAP 标准 RFC 的语义化调用。
//!
//! 与 `metadata.rs`（走 C API）不同，本模块的功能靠调用 ABAP 端的标准 RFC 实现：
//! - 搜索函数：`RFC_FUNCTION_SEARCH`
//! - DDIC 字段语义：`DDIF_FIELDINFO_GET`
//! - 函数长文档：`RFC_READ_TEXT`
//!
//! 这些都是普通 RFC，复用 `api::execute_collect` 即可，无需新 FFI。
//! 每个封装函数把原始表/标量结果映射成语义化的 Rust 结构体。

use crate::api::{FieldSpec, InvokeRequest, ScalarValue};
use crate::connection::RfcConnection;
use crate::error::RfcError;
use crate::executor::execute_collect;
use std::collections::HashMap;

/// 搜索结果条目：一个可远程调用的函数模块
#[derive(Debug, Clone)]
pub struct FunctionEntry {
    pub name: String,
    /// 函数组（可能为空）
    pub group: String,
    /// 短文本描述
    pub description: String,
}

/// 搜索可远程调用的函数模块。
/// 内部调用 ABAP RFC `RFC_FUNCTION_SEARCH`。
///
/// - `pattern`: 函数名通配符，如 `BAPI_USER_*`（空表示 `*`）
/// - `group`: 函数组过滤（空表示不限制）
/// - `max_results`: 最多返回条数（防巨型结果集，默认 50）
pub fn search_functions(
    conn: &RfcConnection,
    pattern: &str,
    group: &str,
    max_results: usize,
) -> Result<Vec<FunctionEntry>, RfcError> {
    let req = InvokeRequest {
        func_name: "RFC_FUNCTION_SEARCH".to_string(),
        inputs: HashMap::from([
            (
                "FUNCNAME".to_string(),
                ScalarValue::Chars(if pattern.is_empty() {
                    "*".to_string()
                } else {
                    pattern.to_uppercase()
                }),
            ),
            (
                "GROUPNAME".to_string(),
                ScalarValue::Chars(group.to_uppercase()),
            ),
        ]),
        table_outputs: HashMap::from([("FUNCTIONS".to_string(), function_table_spec())]),
        ..Default::default()
    };

    let resp = execute_collect(conn, &req)?;
    let table = resp.tables.get("FUNCTIONS").cloned().unwrap_or_default();

    let mut out = Vec::new();
    for row in table.into_iter().take(max_results) {
        out.push(FunctionEntry {
            name: row.get("FUNCNAME").map(|v| v.clone().into_chars()).unwrap_or_default(),
            group: row.get("GROUPNAME").map(|v| v.clone().into_chars()).unwrap_or_default(),
            description: row.get("STEXT").map(|v| v.clone().into_chars()).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// RFC_FUNCTION_SEARCH 结果表的字段规格。
/// SAP 不同版本字段可能略有差异，对缺失字段容错（unwrap_or_default）。
fn function_table_spec() -> Vec<FieldSpec> {
    vec![
        FieldSpec {
            name: "FUNCNAME".to_string(),
            max_len: Some(30),
            auto: false,
        },
        FieldSpec {
            name: "GROUPNAME".to_string(),
            max_len: Some(18),
            auto: false,
        },
        FieldSpec {
            name: "STEXT".to_string(),
            max_len: Some(79),
            auto: false,
        },
    ]
}

/// DDIC 字段的固定值（域的值范围，如状态码 → 描述）
#[derive(Debug, Clone)]
pub struct FixedValue {
    pub value: String,
    pub text: String,
}

/// 单个 DDIC 字段的语义元数据（来自 DDIF_FIELDINFO_GET 的 DFIES 结构）
#[derive(Debug, Clone, Default)]
pub struct FieldSemantics {
    pub field: String,
    /// 数据元素（Roll Name）
    pub data_element: String,
    /// 域（Domain）
    pub domain: String,
    /// 检查表（Check Table）
    pub check_table: String,
    /// 字段描述（短文本）
    pub description: String,
    /// 字段文本（中等长度标签）
    pub medium_label: String,
    /// 固定值列表（域的固定值范围）
    pub fixed_values: Vec<FixedValue>,
}

/// 查询单个 DDIC 字段的语义元数据。
/// 内部调用 ABAP RFC `DDIF_FIELDINFO_GET`，返回 DFIES 结构 + 固定值表。
///
/// - `table`: 表/结构/视图名（如 MARA）
/// - `field`: 字段名（如 MATNR）
/// - `lang`: 语言（如 ZH/EN，影响文本标签语言）
pub fn read_ddic_field_info(
    conn: &RfcConnection,
    table: &str,
    field: &str,
    lang: &str,
) -> Result<FieldSemantics, RfcError> {
    let req = InvokeRequest {
        func_name: "DDIF_FIELDINFO_GET".to_string(),
        inputs: HashMap::from([
            ("TABNAME".to_string(), ScalarValue::Chars(table.to_uppercase())),
            // 注意：查结构单字段必须用 LFIELDNAME 而非 FIELDNAME（后者对结构无效）
            ("LFIELDNAME".to_string(), ScalarValue::Chars(field.to_uppercase())),
            ("LANGU".to_string(), ScalarValue::Chars(lang.to_string())),
            ("ALL_TYPES".to_string(), ScalarValue::Chars("X".to_string())),
        ]),
        // DFIES_WA 是 EXPORT 结构体（单字段语义），FIXED_VALUES 是 TABLES（固定值列表）
        struct_outputs: HashMap::from([("DFIES_WA".to_string(), dfies_field_spec())]),
        table_outputs: HashMap::from([(
            "FIXED_VALUES".to_string(),
            fixed_values_spec(),
        )]),
        ..Default::default()
    };

    let resp = execute_collect(conn, &req)?;
    let dfies = resp.structs.get("DFIES_WA").cloned().unwrap_or_default();

    let fixed_values = resp
        .tables
        .get("FIXED_VALUES")
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|row| FixedValue {
            value: row.get("LOW").map(|v| v.clone().into_chars()).unwrap_or_default(),
            text: row.get("DDTEXT").map(|v| v.clone().into_chars()).unwrap_or_default(),
        })
        .collect();

    Ok(FieldSemantics {
        field: field.to_uppercase(),
        data_element: dfies.get("ROLLNAME").map(|v| v.clone().into_chars()).unwrap_or_default(),
        domain: dfies.get("DOMNAME").map(|v| v.clone().into_chars()).unwrap_or_default(),
        check_table: dfies.get("CHECKTABLE").map(|v| v.clone().into_chars()).unwrap_or_default(),
        description: dfies.get("FIELDTEXT").map(|v| v.clone().into_chars()).unwrap_or_default(),
        medium_label: dfies.get("SCRTEXT_M").map(|v| v.clone().into_chars()).unwrap_or_default(),
        fixed_values,
    })
}

/// DFIES 结构体输出的字段规格（DDIF_FIELDINFO_GET 的 FIELDINFO 参数）。
/// 字段名取自 SAP DFIES 结构定义；对可能缺失的字段容错。
fn dfies_field_spec() -> Vec<FieldSpec> {
    [
        "ROLLNAME",  // 数据元素
        "DOMNAME",   // 域
        "CHECKTABLE", // 检查表
        "FIELDTEXT", // 字段描述
        "SCRTEXT_M", // 中等标签
        "SCRTEXT_L", // 长标签
        "SCRTEXT_S", // 短标签
    ]
    .into_iter()
    .map(|name| FieldSpec {
        name: name.to_string(),
        max_len: Some(79),
        auto: false,
    })
    .collect()
}

/// 固定值表（DD07V）的字段规格
fn fixed_values_spec() -> Vec<FieldSpec> {
    vec![
        FieldSpec {
            name: "LOW".to_string(),
            max_len: Some(10),
            auto: false,
        },
        FieldSpec {
            name: "DDTEXT".to_string(),
            max_len: Some(60),
            auto: false,
        },
    ]
}

/// 函数模块的文档（短文本 + SE37 长文本）
#[derive(Debug, Clone, Default)]
pub struct FunctionDoc {
    #[allow(dead_code)]
    pub name: String,
    /// 短文本（函数模块的 STEXT，来自 FUNCTION_SEARCH 或元数据）
    pub short_text: String,
    /// SE37 长文档（来自 RFC_READ_TEXT，文本对象 FUNC，ID u）
    pub long_text: String,
    /// 读取过程中是否遇到警告（如文档对象不存在）
    pub warning: Option<String>,
}

/// 读取函数模块的 SE37 长文档。
/// 内部调用 ABAP 函数 `DOCU_GET`（SAP 标准文档读取，组 SDOC）。
/// 文档对象约定：
///   - OBJECT = 函数名
///   - ID = "FU"（Function Module 文档类）
///   - LANGU = lang
///
/// 并非所有函数都有文档；无文档时返回空 long_text（不报错）。
pub fn read_function_doc(
    conn: &RfcConnection,
    func_name: &str,
    lang: &str,
    short_text: &str,
) -> Result<FunctionDoc, RfcError> {
    let req = InvokeRequest {
        func_name: "DOCU_GET".to_string(),
        inputs: HashMap::from([
            ("OBJECT".to_string(), ScalarValue::Chars(func_name.to_uppercase())),
            ("ID".to_string(), ScalarValue::Chars("FU".to_string())),
            ("LANGU".to_string(), ScalarValue::Chars(lang.to_string())),
        ]),
        table_outputs: HashMap::from([("LINE".to_string(), text_lines_spec())]),
        ..Default::default()
    };

    match execute_collect(conn, &req) {
        Ok(resp) => {
            let lines = resp.tables.get("LINE").cloned().unwrap_or_default();
            // TLINE 结构：TDFORMAT(2) + TDLINE(132)，拼接所有 TDLINE 成完整文档
            let long_text = lines
                .iter()
                .map(|row| {
                    row.get("TDLINE").map(|v| v.clone().into_chars()).unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(FunctionDoc {
                name: func_name.to_string(),
                short_text: short_text.to_string(),
                long_text,
                warning: None,
            })
        }
        Err(e) => {
            // 文档读取失败不阻断——返回空文档 + 警告
            Ok(FunctionDoc {
                name: func_name.to_string(),
                short_text: short_text.to_string(),
                long_text: String::new(),
                warning: Some(format!("读取长文档失败（可能无文档或 DOCU_GET 不可用）: {}", e.message)),
            })
        }
    }
}

/// DOCU_GET 输出 LINE 表（TLINE 结构）的字段规格
fn text_lines_spec() -> Vec<FieldSpec> {
    vec![
        FieldSpec {
            name: "TDFORMAT".to_string(),
            max_len: Some(2),
            auto: false,
        },
        FieldSpec {
            name: "TDLINE".to_string(),
            max_len: Some(132),
            auto: false,
        },
    ]
}
