use crate::ffi::*;
use crate::error::{check_rc, RfcError};
use crate::string_utils::str_to_sap_uc;
use crate::function::RfcFunction;
use std::collections::HashMap;

/// RFC 连接对象，拥有连接所有权，离开作用域自动关闭
pub struct RfcConnection {
    handle: RFC_CONNECTION_HANDLE,
}

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

    /// 通过 sapnwrfc.ini 中的 DEST 名称建立连接
    pub fn from_dest(dest_name: &str) -> Result<Self, RfcError> {
        Self::new(&[("DEST", dest_name)])
    }

    /// 获取指定函数的调用对象
    pub fn get_function(&self, func_name: &str) -> Result<RfcFunction, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(func_name);

            // 从后端拉取函数元数据描述
            let func_desc = RfcGetFunctionDesc(self.handle, name_uc.as_ptr(), &mut error_info);
            check_rc(error_info.code, &error_info)?;

            // 创建函数调用实例
            let func_handle = RfcCreateFunction(func_desc, &mut error_info);
            check_rc(error_info.code, &error_info)?;

            Ok(RfcFunction {
                handle: func_handle,
                conn_handle: self.handle,
            })
        }
    }

    /// 通用调用入口：传入函数名 + 输入参数，返回按输出名映射的结果
    /// - `func_name`: RFC 函数接口名
    /// - `inputs`: 输入参数名 -> 字符串值
    /// - `outputs`: 需要读取的输出参数名列表及其最大长度
    pub fn call(
        &self,
        func_name: &str,
        inputs: &[(&str, &str)],
        outputs: &[(&str, usize)],
    ) -> Result<HashMap<String, String>, RfcError> {
        let mut func = self.get_function(func_name)?;

        for (name, value) in inputs {
            func.set_chars(name, value)?;
        }

        func.invoke()?;

        let mut result = HashMap::new();
        for (name, max_len) in outputs {
            let value = func.get_chars(name, *max_len)?;
            result.insert(name.to_string(), value);
        }
        Ok(result)
    }
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