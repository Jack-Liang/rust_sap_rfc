use crate::ffi::{RFC_ERROR_INFO, RFC_OK, RFC_RC};
use crate::string_utils::sap_uc_to_string;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use std::fmt;

/// RFC / SAP 调用错误。
///
/// `status` 是映射后的 HTTP 状态码（由 SAP 错误码派生，见 `status_for_rc`），
/// 让调用方能按状态码区分「调用方错误」(4xx) 与「服务端/上游错误」(5xx)。
#[derive(Debug)]
pub struct RfcError {
    pub code: i32,
    pub message: String,
    pub key: String,
    /// HTTP 状态码（默认 500，check_rc 按 SAP 错误码映射）
    pub status: u16,
}

impl Default for RfcError {
    fn default() -> Self {
        Self {
            code: -1,
            message: String::new(),
            key: String::new(),
            status: 500,
        }
    }
}

impl fmt::Display for RfcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RFC调用错误(代码: {})：{}", self.code, self.message)
    }
}

impl std::error::Error for RfcError {}

/// 让 RfcError 可作为 handler 返回值，按 status 字段映射的状态码 + JSON 错误体返回。
#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[derive(Serialize)]
pub struct ErrorBody {
    /// HTTP 状态码（统一契约：客户端按此码判断，不再暴露 SAP 内部 code）
    pub code: u16,
    pub key: String,
    pub message: String,
}

impl IntoResponse for RfcError {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error: ErrorBody {
                code: self.status,
                key: self.key,
                message: self.message,
            },
        };
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, axum::Json(body)).into_response()
    }
}

/// SAP "未找到"类 key（函数/DDIC 定义不存在）。
/// `code 5`(ABAP_EXCEPTION) 配这些 key → 404；其余 code 5 → 400。
fn is_not_found_key(key: &str) -> bool {
    matches!(key, "FU_NOT_FOUND" | "NOT_FOUND")
}

/// 把 SAP RFC_RC 错误码 + key 映射成 HTTP 状态码。
///
/// 映射依据 sapnwrfc.h 的 _RFC_RC 枚举顺序（从 0 开始）：
/// - 4xx（调用方错误）：ABAP 异常/消息(4,5)→400；但函数/DDIC 不存在(key FU_NOT_FOUND/NOT_FOUND)→404；
///   参数无效/转换失败(20,23)→400；授权失败(25)→403
/// - 5xx（服务端/上游错误）：通信失败(1)、连接关闭(6)→502；
///   运行时失败(3)、内存不足(11)→500；超时(9)→504
fn status_for_rc(rc: RFC_RC, key: &str) -> u16 {
    match rc {
        // 4xx：调用方导致的错误（改请求可恢复）
        4 | 5 => {
            // ABAP_MESSAGE / ABAP_EXCEPTION：按 key 区分"未找到"→404，其余→400
            if is_not_found_key(key) {
                404
            } else {
                400
            }
        }
        17 => 404,              // RFC_NOT_FOUND（函数/结构定义不存在）
        20 | 23 => 400,         // RFC_INVALID_PARAMETER / RFC_CONVERSION_FAILURE
        25 => 403,              // RFC_AUTHORIZATION_FAILURE
        // 5xx：上游/服务端错误
        1 | 6 => 502,           // RFC_COMMUNICATION_FAILURE / RFC_CLOSED（网关/连接问题）
        9 => 504,               // RFC_TIMEOUT
        _ => 500,               // 其余（含 ABAP_RUNTIME_FAILURE、未知码）按 500
    }
}

/// 检查 RFC 返回码，成功返回 Ok，失败转换为 RfcError（status 按 SAP 错误码映射）
pub fn check_rc(rc: RFC_RC, error_info: &RFC_ERROR_INFO) -> Result<(), RfcError> {
    unsafe {
        if rc == RFC_OK {
            Ok(())
        } else {
            let key = sap_uc_to_string(error_info.key.as_ptr(), 64);
            let message = sap_uc_to_string(error_info.message.as_ptr(), 256);
            Err(RfcError {
                code: rc,
                message,
                key: key.clone(),
                status: status_for_rc(rc, &key),
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
            status: 500,
        };
        let s = format!("{}", e);
        assert!(s.contains("42"));
        assert!(s.contains("boom"));
    }

    #[test]
    fn status_mapping_for_known_rc() {
        assert_eq!(status_for_rc(4, ""), 400); // ABAP_MESSAGE
        assert_eq!(status_for_rc(5, ""), 400); // ABAP_EXCEPTION，非 NOT_FOUND key
        assert_eq!(status_for_rc(5, "FU_NOT_FOUND"), 404); // 函数不存在
        assert_eq!(status_for_rc(5, "NOT_FOUND"), 404); // DDIC 不存在
        assert_eq!(status_for_rc(17, ""), 404); // NOT_FOUND
        assert_eq!(status_for_rc(20, ""), 400); // INVALID_PARAMETER
        assert_eq!(status_for_rc(23, ""), 400); // CONVERSION_FAILURE
        assert_eq!(status_for_rc(25, ""), 403); // AUTHORIZATION_FAILURE
        assert_eq!(status_for_rc(1, ""), 502); // COMMUNICATION_FAILURE
        assert_eq!(status_for_rc(6, ""), 502); // CLOSED
        assert_eq!(status_for_rc(9, ""), 504); // TIMEOUT
        assert_eq!(status_for_rc(3, ""), 500); // ABAP_RUNTIME_FAILURE
        assert_eq!(status_for_rc(99, ""), 500); // 未知码
    }

    #[test]
    fn is_not_found_key_recognizes_known_keys() {
        assert!(is_not_found_key("FU_NOT_FOUND"));
        assert!(is_not_found_key("NOT_FOUND"));
        assert!(!is_not_found_key(""));
        assert!(!is_not_found_key("OTHER_ERROR"));
    }

    #[test]
    fn check_rc_maps_abap_exception_to_400() {
        let info: RFC_ERROR_INFO = unsafe { std::mem::zeroed() };
        let err = check_rc(5, &info).unwrap_err(); // RFC_ABAP_EXCEPTION
        assert_eq!(err.status, 400);
    }

    #[test]
    fn check_rc_maps_not_found_to_404() {
        let info: RFC_ERROR_INFO = unsafe { std::mem::zeroed() };
        let err = check_rc(17, &info).unwrap_err(); // RFC_NOT_FOUND
        assert_eq!(err.status, 404);
    }

    #[test]
    fn into_response_uses_status_field() {
        let e = RfcError {
            code: 17,
            message: "not found".into(),
            key: "RFC_NOT_FOUND".into(),
            status: 404,
        };
        let resp = e.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn default_error_is_500() {
        let e = RfcError::default();
        assert_eq!(e.status, 500);
        assert_eq!(e.code, -1);
    }

    #[test]
    fn status_mapping_for_closed_rc() {
        // rc=22 (RFC_CLOSED 在某些 SDK 版本) 不在 status_for_rc 的显式分支里，
        // 但它在 RECONNECT_RC（pool 触发重连）。锁住其 HTTP 语义：走 _=>500。
        // 注：标准 RFC_CLOSED=7（已测），这里测 22 确保不变。
        assert_eq!(status_for_rc(22, ""), 500);
    }

    #[test]
    fn into_response_falls_back_on_invalid_status() {
        // 非法 status（超 HTTP 范围）应回退到 500，不 panic
        let e = RfcError {
            code: -1,
            message: "bad status".into(),
            key: String::new(),
            status: 99, // < 100，非合法 HTTP 状态码
        };
        let resp = e.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }
}
