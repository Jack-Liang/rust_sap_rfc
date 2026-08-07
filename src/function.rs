use crate::ffi::*;
use crate::error::{check_rc, RfcError};
use crate::string_utils::{str_to_sap_uc, sap_uc_to_string};
use std::os::raw::c_int;

/// RFC 函数调用对象，离开作用域自动销毁
pub struct RfcFunction {
    pub(crate) handle: RFC_FUNCTION_HANDLE,
    pub(crate) conn_handle: RFC_CONNECTION_HANDLE,
}

/// RFC 表行（结构体句柄）
pub struct RfcRow {
    handle: RFC_STRUCTURE_HANDLE,
}

/// RFC 表参数
pub struct RfcTable {
    handle: RFC_TABLE_HANDLE,
}

impl RfcFunction {
    /// 设置字符串类型输入参数
    pub fn set_chars(&mut self, param_name: &str, value: &str) -> Result<(), RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(param_name);
            let value_uc = str_to_sap_uc(value);

            let rc = RfcSetString(
                self.handle,
                name_uc.as_ptr(),
                value_uc.as_ptr(),
                value.chars().count() as c_int,
                &mut error_info,
            );
            check_rc(rc, &error_info)
        }
    }

    /// 执行远程函数调用
    pub fn invoke(&mut self) -> Result<(), RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let rc = RfcInvoke(self.conn_handle, self.handle, &mut error_info);
            check_rc(rc, &error_info)
        }
    }

    /// 读取字符串类型输出参数
    pub fn get_chars(&mut self, param_name: &str, max_len: usize) -> Result<String, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(param_name);

            let mut buf = vec![0u16; max_len + 1];
            let mut actual_len: c_int = 0;

            let rc = RfcGetString(
                self.handle,
                name_uc.as_ptr(),
                buf.as_mut_ptr(),
                buf.len() as c_int,
                &mut actual_len,
                &mut error_info,
            );
            check_rc(rc, &error_info)?;

            Ok(sap_uc_to_string(buf.as_ptr(), actual_len as usize))
        }
    }

    /// 设置整数类型输入参数
    pub fn set_int(&mut self, param_name: &str, value: i32) -> Result<(), RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(param_name);
            let rc = RfcSetInt(self.handle, name_uc.as_ptr(), value, &mut error_info);
            check_rc(rc, &error_info)
        }
    }

    /// 读取整数类型输出参数
    pub fn get_int(&mut self, param_name: &str) -> Result<i32, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(param_name);
            let mut value: c_int = 0;
            let rc = RfcGetInt(self.handle, name_uc.as_ptr(), &mut value, &mut error_info);
            check_rc(rc, &error_info)?;
            Ok(value)
        }
    }

    /// 获取表参数句柄
    pub fn get_table(&mut self, param_name: &str) -> Result<RfcTable, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(param_name);
            let mut table_handle: RFC_TABLE_HANDLE = std::ptr::null_mut();
            let rc = RfcGetTable(
                self.handle,
                name_uc.as_ptr(),
                &mut table_handle,
                &mut error_info,
            );
            check_rc(rc, &error_info)?;
            Ok(RfcTable { handle: table_handle })
        }
    }
}

impl RfcTable {
    /// 获取表行数
    pub fn row_count(&self) -> Result<u32, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let mut count: u32 = 0;
            let rc = RfcGetRowCount(self.handle, &mut count, &mut error_info);
            check_rc(rc, &error_info)?;
            Ok(count)
        }
    }

    /// 追加一行，返回该行用于设置字段值
    pub fn append_row(&mut self) -> Result<RfcRow, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let row = RfcAppendNewRow(self.handle, &mut error_info);
            check_rc(error_info.code, &error_info)?;
            Ok(RfcRow { handle: row })
        }
    }

    /// 将游标移动到指定行（0-based），然后取该行
    pub fn get_row(&self, index: u32) -> Result<RfcRow, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let rc = RfcMoveTo(self.handle, index, &mut error_info);
            check_rc(rc, &error_info)?;
            let row = RfcGetCurrentRow(self.handle, &mut error_info);
            check_rc(error_info.code, &error_info)?;
            Ok(RfcRow { handle: row })
        }
    }
}

impl RfcRow {
    /// 设置当前行的字符串字段
    pub fn set_chars(&self, field_name: &str, value: &str) -> Result<(), RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(field_name);
            let value_uc = str_to_sap_uc(value);
            let rc = RfcSetString(
                self.handle,
                name_uc.as_ptr(),
                value_uc.as_ptr(),
                value.chars().count() as c_int,
                &mut error_info,
            );
            check_rc(rc, &error_info)
        }
    }

    /// 读取当前行的字符串字段
    pub fn get_chars(&self, field_name: &str, max_len: usize) -> Result<String, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(field_name);
            let mut buf = vec![0u16; max_len + 1];
            let mut actual_len: c_int = 0;
            let rc = RfcGetString(
                self.handle,
                name_uc.as_ptr(),
                buf.as_mut_ptr(),
                buf.len() as c_int,
                &mut actual_len,
                &mut error_info,
            );
            check_rc(rc, &error_info)?;
            Ok(sap_uc_to_string(buf.as_ptr(), actual_len as usize))
        }
    }

    /// 设置当前行的整数字段
    pub fn set_int(&self, field_name: &str, value: i32) -> Result<(), RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(field_name);
            let rc = RfcSetInt(self.handle, name_uc.as_ptr(), value, &mut error_info);
            check_rc(rc, &error_info)
        }
    }

    /// 读取当前行的整数字段
    pub fn get_int(&self, field_name: &str) -> Result<i32, RfcError> {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let name_uc = str_to_sap_uc(field_name);
            let mut value: c_int = 0;
            let rc = RfcGetInt(self.handle, name_uc.as_ptr(), &mut value, &mut error_info);
            check_rc(rc, &error_info)?;
            Ok(value)
        }
    }
}

/// 析构自动销毁函数对象，释放内存
impl Drop for RfcFunction {
    fn drop(&mut self) {
        unsafe {
            let mut error_info = std::mem::zeroed::<RFC_ERROR_INFO>();
            let _ = RfcDestroyFunction(self.handle, &mut error_info);
        }
    }
}