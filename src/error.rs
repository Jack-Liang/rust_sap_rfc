use std::fmt;
use crate::ffi::{RFC_ERROR_INFO, RFC_RC, RFC_OK};
use crate::string_utils::sap_uc_to_string;

#[derive(Debug)]
pub struct RfcError {
    pub code: i32,
    pub message: String,
    pub key: String,
}

impl fmt::Display for RfcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RFC调用错误(代码: {})：{}", self.code, self.message)
    }
}

impl std::error::Error for RfcError {}

/// 检查 RFC 返回码，成功返回 Ok，失败转换为 RfcError
pub fn check_rc(rc: RFC_RC, error_info: &RFC_ERROR_INFO) -> Result<(), RfcError> {
    unsafe {
        if rc == RFC_OK {
            Ok(())
        } else {
            Err(RfcError {
                code: rc,
                message: sap_uc_to_string(error_info.message.as_ptr(), 256),
                key: sap_uc_to_string(error_info.key.as_ptr(), 64),
            })
        }
    }
}