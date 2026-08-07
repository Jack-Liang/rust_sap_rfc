//! 函数元数据缓存：首次调用某函数时拉取其参数描述，后续复用。
//!
//! 目的：
//! 1. 让 REST 调用方不必手填 string_outputs 的 max_len——服务端用 SAP 真实字段长度填充。
//! 2. 输出读取时按字段真实类型（INT/FLOAT/BCD/CHAR...）自动选择对应 RfcGetXxx，
//!    保留数值/二进制语义。
//!
//! 缓存设计：只存 Rust 化的纯数据（不含 SAP 句柄），保证可跨线程共享。
//! 表参数的字段信息在拉取时一次性递归解析好。

use crate::connection::{get_field_infos, RfcConnection};
use crate::error::RfcError;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 单个字段的元数据
#[derive(Debug, Clone, Copy)]
pub struct FieldMeta {
    /// 字符长度（ucLength/2，对 CHAR/NUM/DATE/TIME 有意义）
    pub char_length: usize,
    /// RFCTYPE_* 常量值（见 ffi.rs）
    pub type_: i32,
}

/// 单个函数的元数据（纯数据，无裸指针，可跨线程共享）
#[derive(Debug, Clone, Default)]
pub struct FuncMetadata {
    /// 标量参数：参数名 -> 字段元数据
    pub scalars: HashMap<String, FieldMeta>,
    /// 表参数：表名 -> {字段名 -> 字段元数据}
    pub tables: HashMap<String, HashMap<String, FieldMeta>>,
}

/// 全局元数据缓存：函数名 -> 元数据
static METADATA_CACHE: OnceLock<Mutex<HashMap<String, FuncMetadata>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, FuncMetadata>> {
    METADATA_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 拉取并组装某函数的完整元数据（含表字段类型递归解析）。
fn fetch_metadata(conn: &RfcConnection, func_name: &str) -> Result<FuncMetadata, RfcError> {
    let param_infos = conn.get_param_infos(func_name)?;
    let mut meta = FuncMetadata::default();

    for p in &param_infos {
        let fm = FieldMeta {
            char_length: p.char_length,
            type_: p.type_,
        };
        // 表参数和结构体参数：递归读其字段元数据
        // （表和结构体的行类型都用 type_desc_handle 描述，处理方式相同）
        if p.type_ == crate::ffi::RFCTYPE_TABLE || p.type_ == crate::ffi::RFCTYPE_STRUCTURE {
            if let Some(type_handle) = p.type_desc_handle {
                // SAFETY: type_handle 来自刚拉取的有效元数据，连接仍有效
                let fields = unsafe { get_field_infos(type_handle) }?;
                let field_map: HashMap<String, FieldMeta> = fields
                    .iter()
                    .map(|f| {
                        (
                            f.name.to_uppercase(),
                            FieldMeta {
                                char_length: f.char_length,
                                type_: f.type_,
                            },
                        )
                    })
                    .collect();
                meta.tables.insert(p.name.to_uppercase(), field_map);
            }
        }
        meta.scalars.insert(p.name.to_uppercase(), fm);
    }

    Ok(meta)
}

/// 获取某函数的元数据。命中缓存直接返回；未命中则从 SAP 拉取并缓存。
pub fn get_metadata(conn: &RfcConnection, func_name: &str) -> Result<FuncMetadata, RfcError> {
    // 先查缓存（快路径）
    {
        let map = cache().lock().map_err(|e| RfcError {
            code: -1,
            message: format!("元数据缓存锁被毒化: {}", e),
            key: String::new(),
        })?;
        if let Some(meta) = map.get(func_name) {
            return Ok(meta.clone());
        }
    }

    // 未命中：从 SAP 拉取
    let meta = fetch_metadata(conn, func_name)?;
    let mut map = cache().lock().map_err(|e| RfcError {
        code: -1,
        message: format!("元数据缓存锁被毒化: {}", e),
        key: String::new(),
    })?;
    map.insert(func_name.to_string(), meta.clone());
    tracing::debug!(
        func = func_name,
        scalars = meta.scalars.len(),
        tables = meta.tables.len(),
        "元数据已缓存"
    );
    Ok(meta)
}

/// 查询某标量参数的元数据。
pub fn param_meta(conn: &RfcConnection, func_name: &str, param_name: &str) -> Option<FieldMeta> {
    let meta = get_metadata(conn, func_name).ok()?;
    meta.scalars.get(&param_name.to_uppercase()).copied()
}

/// 查询某表参数的字段元数据映射（field_name -> FieldMeta）。
pub fn table_field_metas(
    conn: &RfcConnection,
    func_name: &str,
    table_param: &str,
) -> Option<HashMap<String, FieldMeta>> {
    let meta = get_metadata(conn, func_name).ok()?;
    meta.tables.get(&table_param.to_uppercase()).cloned()
}
