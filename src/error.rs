use crate::ffi::{RFC_ERROR_INFO, RFC_OK, RFC_RC};
use crate::string_utils::sap_uc_to_string;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use std::fmt;

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

/// 让 RfcError 可作为 handler 返回值，自动序列化为 500 + JSON 错误体
#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: i32,
    key: String,
    message: String,
}

impl IntoResponse for RfcError {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error: ErrorBody {
                code: self.code,
                key: self.key,
                message: self.message,
            },
        };
        (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_rc_ok_returns_ok() {
        let info: RFC_ERROR_INFO = unsafe { std::mem::zeroed() };
        assert!(check_rc(RFC_OK, &info).is_ok());
    }

    #[test]
    fn check_rc_nonzero_returns_err_with_code() {
        let info: RFC_ERROR_INFO = unsafe { std::mem::zeroed() };
        let err = check_rc(7, &info).unwrap_err();
        assert_eq!(err.code, 7);
        // message/key 缓冲区全 0 → 空字符串
        assert_eq!(err.message, "");
        assert_eq!(err.key, "");
    }

    #[test]
    fn rfc_error_display_includes_code() {
        let e = RfcError {
            code: 42,
            message: "boom".into(),
            key: "K".into(),
        };
        let s = format!("{}", e);
        assert!(s.contains("42"));
        assert!(s.contains("boom"));
    }

    #[test]
    fn into_response_produces_500_with_json_body() {
        let e = RfcError {
            code: 5,
            message: "no auth".into(),
            key: "RFC_AUTHORIZATION".into(),
        };
        let resp = e.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        // body 是异步的，这里只验证状态码；完整 body 验证留给集成测试
    }
}
