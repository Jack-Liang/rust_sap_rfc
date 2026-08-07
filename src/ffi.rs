//! 底层 C FFI 最小绑定，仅覆盖基础客户端调用
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::os::raw::{c_int, c_void};

// 基础类型映射
pub type SAP_UC = u16;          // SAP 宽字符（UTF-16）
pub type RFC_RC = c_int;        // 函数返回码
pub type RFC_INT = c_int;       // 4 字节整型
pub type RFC_CONNECTION_HANDLE = *mut c_void;
pub type RFC_FUNCTION_HANDLE = *mut c_void;
pub type RFC_FUNCTION_DESC_HANDLE = *mut c_void;
pub type RFC_TABLE_HANDLE = *mut c_void;
pub type RFC_STRUCTURE_HANDLE = *mut c_void;

pub const RFC_OK: RFC_RC = 0;

// 连接参数结构体
#[repr(C)]
pub struct RFC_CONNECTION_PARAMETER {
    pub name: *const SAP_UC,
    pub value: *const SAP_UC,
}

// 错误信息结构体（对齐官方原生内存布局）
#[repr(C)]
pub struct RFC_ERROR_INFO {
    pub code: RFC_RC,
    pub group: c_int,
    pub key: [SAP_UC; 128],
    pub message: [SAP_UC; 512],
    pub abap_msg_type: [SAP_UC; 1],
    pub abap_msg_class: [SAP_UC; 20],
    pub abap_msg_number: [SAP_UC; 3],
    pub abap_msg_v1: [SAP_UC; 50],
    pub abap_msg_v2: [SAP_UC; 50],
    pub abap_msg_v3: [SAP_UC; 50],
    pub abap_msg_v4: [SAP_UC; 50],
}

// Windows NWRFC SDK 使用 _stdcall 调用约定，等同于 "system"（Windows 上 stdcall = system）
extern "system" {
    // 连接管理
    pub fn RfcOpenConnection(
        connectionParams: *const RFC_CONNECTION_PARAMETER,
        paramCount: c_int,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_CONNECTION_HANDLE;

    pub fn RfcCloseConnection(
        connection: RFC_CONNECTION_HANDLE,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    // 函数元数据与实例
    pub fn RfcGetFunctionDesc(
        connection: RFC_CONNECTION_HANDLE,
        funcName: *const SAP_UC,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_FUNCTION_DESC_HANDLE;

    pub fn RfcCreateFunction(
        funcDesc: RFC_FUNCTION_DESC_HANDLE,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_FUNCTION_HANDLE;

    pub fn RfcDestroyFunction(
        funcHandle: RFC_FUNCTION_HANDLE,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    // 字符串 / 整数读写（dataHandle 可为函数或结构体）
    pub fn RfcSetString(
        dataHandle: *mut c_void,
        name: *const SAP_UC,
        value: *const SAP_UC,
        valueLen: c_int,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    pub fn RfcGetString(
        dataHandle: *mut c_void,
        name: *const SAP_UC,
        value: *mut SAP_UC,
        maxLen: c_int,
        actualLen: *mut c_int,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    pub fn RfcSetInt(
        dataHandle: *mut c_void,
        name: *const SAP_UC,
        value: c_int,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    pub fn RfcGetInt(
        dataHandle: *mut c_void,
        name: *const SAP_UC,
        value: *mut c_int,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    // 执行远程调用
    pub fn RfcInvoke(
        connection: RFC_CONNECTION_HANDLE,
        funcHandle: RFC_FUNCTION_HANDLE,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    // 表操作
    pub fn RfcGetTable(
        funcHandle: RFC_FUNCTION_HANDLE,
        name: *const SAP_UC,
        tableHandle: *mut RFC_TABLE_HANDLE,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    pub fn RfcGetRowCount(
        tableHandle: RFC_TABLE_HANDLE,
        rowCount: *mut u32,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    pub fn RfcMoveTo(
        tableHandle: RFC_TABLE_HANDLE,
        index: u32,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_RC;

    pub fn RfcGetCurrentRow(
        tableHandle: RFC_TABLE_HANDLE,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_STRUCTURE_HANDLE;

    pub fn RfcAppendNewRow(
        tableHandle: RFC_TABLE_HANDLE,
        errorInfo: *mut RFC_ERROR_INFO,
    ) -> RFC_STRUCTURE_HANDLE;
}