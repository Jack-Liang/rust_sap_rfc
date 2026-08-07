use crate::error::{check_rc, RfcError};
use crate::ffi::*;
use crate::function::RfcFunction;
use crate::string_utils::str_to_sap_uc;

/// RFC 连接对象，拥有连接所有权，离开作用域自动关闭
pub struct RfcConnection {
    handle: RFC_CONNECTION_HANDLE,
}

// SAFETY: `RfcConnection` 内部唯一非 Send 成员是裸指针 `RFC_CONNECTION_HANDLE`。
// 在本服务中，所有对连接的访问都被 `Arc<Mutex<RfcConnection>>` 串行化，
// 同一时刻只有一个线程持有连接；SAP NWRFC SDK 允许在「不同线程串行」使用
// 同一连接句柄（不允许并发），因此把它标记为 Send 是 sound 的。
// 注意：仍不可跨 await 点持有 Mutex 守卫，本服务中所有 FFI 调用都通过
// spawn_blocking 在专用阻塞线程执行，从根本上规避跨 await 点问题。
unsafe impl Send for RfcConnection {}

impl RfcConnection {
    /// 通过键值对参数建立直连
    pub fn new(params: &[(&str, &str)]) -> Result<Self, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();

            // 转换所有参数为 SAP 宽字符格式
            let param_owned: Vec<(Vec<u16>, Vec<u16>)> = params
                .iter()
                .map(|(k, v)| (str_to_sap_uc(k), str_to_sap_uc(v)))
                .collect();

            // 构造 SDK 要求的参数数组
            let rfc_params: Vec<RFC_CONNECTION_PARAMETER> = param_owned
                .iter()
                .map(|(k, v)| RFC_CONNECTION_PARAMETER {
                    name: k.as_ptr(),
                    value: v.as_ptr(),
                })
                .collect();

            // 调用 C API 建立连接
            let handle = RfcOpenConnection(
                rfc_params.as_ptr(),
                rfc_params.len() as i32,
                &mut error_info,
            );

            check_rc(error_info.code, &error_info)?;
            Ok(Self { handle })
        }
    }

    /// 获取指定函数的调用对象
    pub fn get_function(&self, func_name: &str) -> Result<RfcFunction, RfcError> {
        unsafe {
            let func_desc = self.get_function_desc_handle(func_name)?;
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let func_handle = RfcCreateFunction(func_desc, &mut error_info);
            check_rc(error_info.code, &error_info)?;

            Ok(RfcFunction {
                handle: func_handle,
                conn_handle: self.handle,
            })
        }
    }

    /// 拉取函数元数据描述句柄（内部用：get_function 和 get_param_infos 共用）
    unsafe fn get_function_desc_handle(
        &self,
        func_name: &str,
    ) -> Result<RFC_FUNCTION_DESC_HANDLE, RfcError> {
        let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
        let name_uc = str_to_sap_uc(func_name);
        let func_desc = RfcGetFunctionDesc(self.handle, name_uc.as_ptr(), &mut error_info);
        check_rc(error_info.code, &error_info)?;
        Ok(func_desc)
    }

    /// 读取函数的所有参数元数据（名称、类型、字符长度、方向、是否表/结构体）。
    /// 用于元数据自动发现，避免调用方手填 max_len。
    pub fn get_param_infos(&self, func_name: &str) -> Result<Vec<ParamInfo>, RfcError> {
        unsafe {
            let func_desc = self.get_function_desc_handle(func_name)?;
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();

            let mut count: u32 = 0;
            let rc = RfcGetParameterCount(func_desc, &mut count, &mut error_info);
            check_rc(rc, &error_info)?;

            let mut out = Vec::with_capacity(count as usize);
            for i in 0..count {
                let mut pdesc: RFC_PARAMETER_DESC = std::mem::zeroed();
                let rc = RfcGetParameterDescByIndex(func_desc, i, &mut pdesc, &mut error_info);
                check_rc(rc, &error_info)?;

                let name = crate::string_utils::sap_uc_to_string(pdesc.name.as_ptr(), 30);
                // ucLength 是 2-byte-per-SAP_CHAR 系统的字节长度，字符长度 = ucLength / 2
                let char_length = pdesc.ucLength / 2;
                out.push(ParamInfo {
                    name,
                    type_: pdesc.type_,
                    char_length: char_length as usize,
                    direction: pdesc.direction,
                    type_desc_handle: if pdesc.typeDescHandle.handle.is_null() {
                        None
                    } else {
                        Some(pdesc.typeDescHandle)
                    },
                });
            }
            Ok(out)
        }
    }
}

/// 单个参数的元数据（Rust 化后的 RFC_PARAMETER_DESC 子集）
#[derive(Clone)]
pub struct ParamInfo {
    pub name: String,
    /// RFCTYPE_* 常量值
    pub type_: i32,
    /// 字符长度（ucLength/2，对 CHAR/NUM/DATE/TIME 等有意义）
    pub char_length: usize,
    /// RFC_DIRECTION 位掩码（import/export/changing/tables）
    pub direction: i32,
    /// 若为结构体/表，持有其子类型句柄（用于进一步查询字段长度）
    pub type_desc_handle: Option<RFC_TYPE_DESC_HANDLE>,
}

impl std::fmt::Debug for ParamInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParamInfo")
            .field("name", &self.name)
            .field("type_", &self.type_)
            .field("char_length", &self.char_length)
            .field("direction", &self.direction)
            .field("type_desc_handle", &self.type_desc_handle.is_some())
            .finish()
    }
}

/// 读取结构体/表类型的所有字段元数据（用于自动发现表行的字段长度）。
///
/// # Safety
/// `type_handle` 必须是从 ParamInfo 取得的有效句柄。
pub unsafe fn get_field_infos(
    type_handle: RFC_TYPE_DESC_HANDLE,
) -> Result<Vec<ParamInfo>, RfcError> {
    let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
    let mut count: u32 = 0;
    let rc = RfcGetFieldCount(type_handle, &mut count, &mut error_info);
    check_rc(rc, &error_info)?;

    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let mut fdesc: RFC_FIELD_DESC = std::mem::zeroed();
        let rc = RfcGetFieldDescByIndex(type_handle, i, &mut fdesc, &mut error_info);
        check_rc(rc, &error_info)?;
        let name = crate::string_utils::sap_uc_to_string(fdesc.name.as_ptr(), 30);
        let char_length = fdesc.ucLength / 2;
        out.push(ParamInfo {
            name,
            type_: fdesc.type_,
            char_length: char_length as usize,
            direction: 0, // 字段没有方向
            type_desc_handle: if fdesc.typeDescHandle.handle.is_null() {
                None
            } else {
                Some(fdesc.typeDescHandle)
            },
        });
    }
    Ok(out)
}

/// 析构自动关闭连接，对应 C 中的 RfcCloseConnection
impl Drop for RfcConnection {
    fn drop(&mut self) {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            // 忽略关闭错误，避免 drop 中 panic
            let _ = RfcCloseConnection(self.handle, &mut error_info);
        }
    }
}
