//! SAP 连接管理：单连接 + 失败自动重连。
//!
//! 现实中 SAP 连接会因为网络抖动、系统重启、会话超时而失效。
//! 直接把连接句柄包成 `Arc<Mutex<RfcConnection>>` 会让服务在连接断后永久 500，
//! 必须重启。本模块用一个轻量池封装：执行闭包时若遇到通信类错误，
//! 自动重建连接并重试一次，对调用方透明。
//!
//! 并发模型保持不变：内部仍是单连接 + `Mutex`，所有调用串行。

use crate::connection::RfcConnection;
use crate::error::RfcError;
use crate::ffi::RFC_RC;
use std::sync::Mutex;

/// 触发重连的 SAP 错误码集合。
/// 这些都是「连接已不可用」类错误，重建连接是正确的应对。
/// 取自 sapnwrfc.h 中 RFC_RC 枚举的通信/系统失败类。
const RECONNECT_RC: [RFC_RC; 4] = [
    1,  // RFC_COMMUNICATION_FAILURE
    2,  // RFC_SYSTEM_FAILURE
    3,  // RFC_ABAP_EXCEPTION（部分场景连接状态也会变坏）
    22, // RFC_CLOSED（连接已被关闭）
];

fn should_reconnect(err: &RfcError) -> bool {
    RECONNECT_RC.contains(&err.code)
}

/// 单连接池：内部持有一个连接 + 用于重建连接的参数快照。
pub struct RfcConnectionPool {
    /// 连接参数：键为 'static 字面量，值为 owned String（重建连接时复用）
    params: Vec<(&'static str, String)>,
    conn: Mutex<RfcConnection>,
}

impl RfcConnectionPool {
    /// 创建池：立即建立首次连接。
    pub fn new(params: Vec<(&'static str, String)>) -> Result<Self, RfcError> {
        let borrowed: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let conn = RfcConnection::new(&borrowed)?;
        Ok(Self {
            params,
            conn: Mutex::new(conn),
        })
    }

    /// 用当前连接执行闭包。
    ///
    /// - 闭包成功 → 直接返回其结果。
    /// - 闭包失败但错误属于通信类 → 重建连接，再执行一次闭包。
    /// - 闭包失败且非通信类（如参数错误、ABAP 业务异常）→ 直接返回错误，不重连。
    ///
    /// 第二次失败也直接返回，不无限重连。
    pub fn with_connection<R, F>(&self, mut f: F) -> Result<R, RfcError>
    where
        F: FnMut(&RfcConnection) -> Result<R, RfcError>,
    {
        // 第一次尝试
        let first = {
            let guard = self.lock()?;
            f(&guard)
        };
        match first {
            Ok(r) => Ok(r),
            Err(e) if should_reconnect(&e) => {
                // 通信类错误：重建连接后重试一次
                tracing::warn!(code = e.code, key = %e.key, "SAP 连接失败，尝试重连");
                self.reconnect()?;
                let guard = self.lock()?;
                f(&guard)
            }
            Err(e) => Err(e),
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, RfcConnection>, RfcError> {
        self.conn.lock().map_err(|e| RfcError {
            code: -1,
            message: format!("连接锁被毒化: {}", e),
            key: String::new(),
        })
    }

    /// 销毁当前连接并按保存的参数重建。
    /// 持锁期间执行：重建期间所有调用方排队，但不会出现「半新半旧」连接。
    fn reconnect(&self) -> Result<(), RfcError> {
        let mut guard = self.lock()?;
        let borrowed: Vec<(&str, &str)> =
            self.params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let new_conn = RfcConnection::new(&borrowed)?;
        // 旧连接在赋值时 drop，自动 RfcCloseConnection
        *guard = new_conn;
        tracing::info!("SAP 连接已重建");
        Ok(())
    }
}

// SAFETY: 与 RfcConnection 同理。池内部用 Mutex 串行化所有访问，
// 唯一的裸指针（RfcConnection 内部）永远不会跨线程并发使用。
unsafe impl Send for RfcConnectionPool {}
unsafe impl Sync for RfcConnectionPool {}
