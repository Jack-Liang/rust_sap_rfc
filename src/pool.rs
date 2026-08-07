//! SAP 连接管理：多连接池 + 失败自动重连。
//!
//! 现实中 SAP 连接会因为网络抖动、系统重启、会话超时而失效。
//! 本模块维护一组可复用的连接，每个连接的 FFI 调用各自串行，
//! 但不同连接之间可并行——配合 tokio 的 `spawn_blocking`，handler
//! 并发请求能拿到不同连接并行执行。
//!
//! 连接生命周期：首次按需创建 → 复用 → 遇通信错误丢弃 → 池满后新建补充。
//! 对调用方完全透明（`with_connection` 接口与旧单连接版一致）。

use crate::connection::RfcConnection;
use crate::error::RfcError;
use crate::ffi::RFC_RC;
use std::sync::{Condvar, Mutex};

/// 触发「丢弃该连接」的 SAP 错误码集合。
/// 这些都是「连接已不可用」类错误，复用废连接无意义，应销毁后新建。
/// 取自 sapnwrfc.h 中 RFC_RC 枚举的通信/系统失败类。
const RECONNECT_RC: [RFC_RC; 4] = [
    1,  // RFC_COMMUNICATION_FAILURE
    2,  // RFC_SYSTEM_FAILURE
    3,  // RFC_ABAP_EXCEPTION（部分场景连接状态也会变坏）
    22, // RFC_CLOSED（连接已被关闭）
];

fn should_discard(err: &RfcError) -> bool {
    RECONNECT_RC.contains(&err.code)
}

/// 池的内部可变状态：空闲连接栈 + 当前总连接数。
struct PoolInner {
    idle: Vec<RfcConnection>,
    /// 当前已创建的连接总数（空闲 + 借出中）
    total: usize,
}

/// 多连接池：维护一组可复用的 SAP 连接。
///
/// - 空闲时连接留在池里复用（避免每次调用都握手）
/// - 借出时从空闲栈 pop；无空闲且未达 `max_size` 则新建
/// - 无空闲且已达上限则阻塞等待（Condvar），直到有连接归还
/// - 通信类错误归还时丢弃该连接（不回池），自然降总量
pub struct RfcConnectionPool {
    /// 连接参数：键为 'static 字面量，值为 owned String（新建连接时复用）
    params: Vec<(&'static str, String)>,
    /// 池上限（含空闲 + 借出）。超过则等待，不无限增长。
    max_size: usize,
    inner: Mutex<PoolInner>,
    /// 空闲连接可用时唤醒等待者
    cv: Condvar,
}

impl RfcConnectionPool {
    /// 创建池（默认上限 8）：立即建立首次连接，其余按需创建。
    #[allow(dead_code)]
    pub fn new(params: Vec<(&'static str, String)>) -> Result<Self, RfcError> {
        Self::with_max_size(params, 8)
    }

    /// 指定池上限创建。
    pub fn with_max_size(
        params: Vec<(&'static str, String)>,
        max_size: usize,
    ) -> Result<Self, RfcError> {
        let borrowed: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let conn = RfcConnection::new(&borrowed)?;
        let max_size = max_size.max(1); // 至少 1
        tracing::info!(max_size, "SAP 连接池已创建");
        Ok(Self {
            params,
            max_size,
            inner: Mutex::new(PoolInner {
                idle: vec![conn],
                total: 1,
            }),
            cv: Condvar::new(),
        })
    }

    /// 用一个连接执行闭包。接口与旧单连接版完全一致，调用方无需改动。
    ///
    /// - 闭包成功 → 归还连接，返回结果
    /// - 闭包失败但属通信类 → 丢弃连接，新建一个重试一次
    /// - 闭包失败且非通信类 → 归还连接（连接仍健康），返回错误
    ///
    /// 无空闲连接且未达上限时新建；已达上限则阻塞等待他人归还。
    pub fn with_connection<R, F>(&self, mut f: F) -> Result<R, RfcError>
    where
        F: FnMut(&RfcConnection) -> Result<R, RfcError>,
    {
        // 1. 借出一个连接
        let conn = self.acquire()?;

        // 2. 执行
        let result = f(&conn);

        match result {
            Ok(r) => {
                self.release(conn);
                Ok(r)
            }
            Err(e) if should_discard(&e) => {
                // 通信类错误：丢弃废连接，新建一个重试
                tracing::warn!(code = e.code, key = %e.key, "SAP 连接失败，丢弃并重试");
                self.discard_and_replenish(conn)?;
                // 重新借一个（此时池里至少有刚新建的那个）
                let conn2 = self.acquire()?;
                let r = f(&conn2);
                self.release(conn2);
                r
            }
            Err(e) => {
                // 非通信错误（参数错/ABAP 业务异常）：连接仍健康，归还
                self.release(conn);
                Err(e)
            }
        }
    }

    /// 借出一个连接：优先 pop 空闲；无空闲且未达上限则新建；达上限则等待。
    fn acquire(&self) -> Result<RfcConnection, RfcError> {
        let mut guard = self.inner.lock().map_err(|e| poison_err("连接池锁", e))?;
        loop {
            // 有空闲：直接 pop
            if let Some(conn) = guard.idle.pop() {
                return Ok(conn);
            }
            // 无空闲但未达上限：新建（锁内不建连接——握手慢，会阻塞他人）
            if guard.total < self.max_size {
                guard.total += 1;
                // 释放锁后再建连接（避免持锁期间长时间阻塞其他 acquire）
                drop(guard);
                match self.create_connection() {
                    Ok(c) => return Ok(c),
                    Err(e) => {
                        // 新建失败：回滚计数
                        let mut g = self.inner.lock().map_err(|e2| poison_err("连接池锁", e2))?;
                        g.total -= 1;
                        // 新建失败可能意味着 SAP 挂了，唤醒等待者让他们也重试/失败
                        self.cv.notify_one();
                        return Err(e);
                    }
                }
            }
            // 已达上限且无空闲：等待归还
            guard = self
                .cv
                .wait_timeout(guard, std::time::Duration::from_secs(30))
                .map_err(|e| poison_err("连接池等待", e))?
                .0;
        }
    }

    /// 归还健康连接到空闲栈，唤醒一个等待者。
    fn release(&self, conn: RfcConnection) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.idle.push(conn);
        }
        // 无论锁成功与否都唤醒（锁毒化时等待者会自行报错）
        self.cv.notify_one();
    }

    /// 丢弃废连接并补充一个新建连接（保持池容量）。
    /// 旧连接 drop 时自动 RfcCloseConnection。
    fn discard_and_replenish(&self, _discarded: RfcConnection) -> Result<(), RfcError> {
        // _discarded 在函数结束时 drop，自动关闭。
        // 先减计数（旧连接即将销毁），再建新的（计数加回）。
        {
            let mut guard = self.inner.lock().map_err(|e| poison_err("连接池锁", e))?;
            guard.total -= 1; // 旧连接销毁，腾出配额
        }
        // 新建并放回空闲栈
        let new_conn = self.create_connection()?;
        {
            let mut guard = self.inner.lock().map_err(|e| poison_err("连接池锁", e))?;
            guard.total += 1;
            guard.idle.push(new_conn);
        }
        self.cv.notify_one();
        tracing::info!("SAP 连接已重建（池内补充）");
        Ok(())
    }

    /// 用保存的参数新建一个连接（无锁操作，调用方负责计数管理）。
    fn create_connection(&self) -> Result<RfcConnection, RfcError> {
        let borrowed: Vec<(&str, &str)> =
            self.params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        RfcConnection::new(&borrowed)
    }
}

fn poison_err<T>(ctx: &str, _e: T) -> RfcError {
    RfcError {
        code: -1,
        message: format!("{}被毒化", ctx),
        key: String::new(),
    }
}

// SAFETY: 与 RfcConnection 同理。池内部用 Mutex 串行化对连接 Vec 的访问，
// 每个借出的 RfcConnection 由调用方独占使用（spawn_blocking 闭包内不跨线程共享）。
// NWRFC SDK 允许不同连接对象在不同线程并发使用。
unsafe impl Send for RfcConnectionPool {}
unsafe impl Sync for RfcConnectionPool {}
