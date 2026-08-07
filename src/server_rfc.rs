//! SAP RFC Server 端：让 SAP 系统通过 RFC 回调本服务。
//!
//! 工作模式：注册到 SAP Gateway → 监听入站调用 → 把调用转发到配置的 HTTP webhook →
//! 把 webhook 响应回填给 SAP。实现「SAP → HTTP」的反向代理，与 client 模式对称。
//!
//! 线程模型：dispatch 循环在独立 OS 线程跑（FFI 阻塞调用，不能在 tokio worker）。
//! 回调在 dispatch 线程内同步执行，webhook 用 reqwest::blocking。

use crate::error::RfcError;
use crate::ffi::*;
use crate::function;
use crate::server_config::{ParamDef, ServerConfig};
use crate::string_utils::{sap_uc_to_string, str_to_sap_uc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::os::raw::c_void;
use std::sync::{Mutex, OnceLock};

// ============ webhook 请求/响应 DTO ============

/// 发给 webhook 的请求体：函数名 + 入参
#[derive(Debug, Serialize)]
struct WebhookRequest<'a> {
    func: &'a str,
    /// 入参：参数名 → 字符串值（所有类型统一字符串表达）
    inputs: HashMap<String, String>,
}

/// webhook 返回的响应体：出参
#[derive(Debug, Deserialize)]
struct WebhookResponse {
    /// 出参：参数名 → 字符串值
    #[serde(default)]
    outputs: HashMap<String, String>,
}

// ============ 全局 handler 注册表 ============

/// 单个函数的处理配置：webhook URL + 参数定义
struct HandlerEntry {
    webhook_url: String,
    params: Vec<ParamDef>,
}

static HANDLERS: OnceLock<Mutex<HashMap<String, HandlerEntry>>> = OnceLock::new();

fn handlers() -> &'static Mutex<HashMap<String, HandlerEntry>> {
    HANDLERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 把配置里的函数定义注册到全局表，并构造 SDK 函数描述符 + 安装回调。
/// 返回所有已构造的 funcDescHandle（需在 dispatch 期间保持存活）。
fn install_handlers(cfg: &ServerConfig) -> Result<Vec<RFC_FUNCTION_DESC_HANDLE>, RfcError> {
    let mut desc_handles = Vec::new();
    let mut map = handlers().lock().map_err(|e| RfcError {
        code: -1,
        message: format!("handler 锁毒化: {}", e),
        key: String::new(),
    })?;

    for f in &cfg.functions {
        // 1. 构造函数描述符
        let desc = unsafe { create_function_desc(&f.name)? };
        // 2. 添加参数
        for p in &f.params {
            unsafe { add_parameter(desc, p)? };
        }
        // 3. 注册到全局表
        map.insert(
            f.name.to_uppercase(),
            HandlerEntry {
                webhook_url: f.webhook_url.clone(),
                params: f.params.clone(),
            },
        );
        // 4. 安装回调（sysId = null，对所有 SAP 系统生效）
        let rc = unsafe {
            let mut err = std::mem::zeroed::<RFC_ERROR_INFO>();
            RfcInstallServerFunction(
                std::ptr::null(),
                desc,
                generic_handler as *const c_void,
                &mut err,
            )
        };
        if rc != RFC_OK {
            return Err(RfcError {
                code: rc,
                message: format!("RfcInstallServerFunction 失败: {}", f.name),
                key: String::new(),
            });
        }
        tracing::info!(func = %f.name, webhook = %f.webhook_url, "已注册 server 函数");
        desc_handles.push(desc);
    }
    Ok(desc_handles)
}

// ============ FFI 包装（unsafe，构造描述符/参数）============

unsafe fn create_function_desc(name: &str) -> Result<RFC_FUNCTION_DESC_HANDLE, RfcError> {
    let mut err = std::mem::zeroed::<RFC_ERROR_INFO>();
    let name_uc = str_to_sap_uc(name);
    let h = RfcCreateFunctionDesc(name_uc.as_ptr(), &mut err);
    if h.is_null() {
        return Err(RfcError {
            code: err.code,
            message: sap_uc_to_string(err.message.as_ptr(), 256),
            key: sap_uc_to_string(err.key.as_ptr(), 64),
        });
    }
    Ok(h)
}

unsafe fn add_parameter(desc: RFC_FUNCTION_DESC_HANDLE, p: &ParamDef) -> Result<(), RfcError> {
    let mut pdesc: RFC_PARAMETER_DESC = std::mem::zeroed();
    // 填 name（RFC_ABAP_NAME = [SAP_UC; 31]，0 终止）
    let name_uc = str_to_sap_uc(&p.name);
    let name_chars: &[SAP_UC] = name_uc.as_slice();
    let copy_len = name_chars.len().min(pdesc.name.len() - 1);
    pdesc.name[..copy_len].copy_from_slice(&name_chars[..copy_len]);

    pdesc.type_ = p.rfc_type().map_err(|e| RfcError {
        code: -1,
        message: e,
        key: String::new(),
    })?;
    pdesc.direction = p.direction_mask().map_err(|e| RfcError {
        code: -1,
        message: e,
        key: String::new(),
    })?;
    // charLength = ucLength/2；ucLength 用字节（2 bytes/SAP_CHAR）
    if let Some(len) = p.length {
        pdesc.ucLength = (len * 2) as u32;
        pdesc.nucLength = len as u32;
    }
    pdesc.decimals = 0;
    pdesc.optional = 1; // 默认可选，避免 SAP 端必须传所有参数

    let mut err = std::mem::zeroed::<RFC_ERROR_INFO>();
    let rc = RfcAddParameter(desc, &pdesc, &mut err);
    if rc != RFC_OK {
        return Err(RfcError {
            code: rc,
            message: format!(
                "RfcAddParameter 失败 ({}): {}",
                p.name,
                sap_uc_to_string(err.message.as_ptr(), 256)
            ),
            key: String::new(),
        });
    }
    Ok(())
}

// ============ 回调函数（C→Rust）============

/// 通用回调：SAP 每次调用注册的函数时，SDK 在 dispatch 线程内调此函数。
///
/// 流程：取函数名 → 读入参 → POST webhook → 回填出参 → 返回 RC。
/// 返回非 RFC_OK 时，SDK 向 SAP 回传 SYSTEM_FAILURE。
extern "system" fn generic_handler(
    _rfc_handle: RFC_CONNECTION_HANDLE,
    func_handle: RFC_FUNCTION_HANDLE,
    err_info: *mut RFC_ERROR_INFO,
) -> RFC_RC {
    match handle_call(func_handle) {
        Ok(()) => RFC_OK,
        Err(e) => {
            tracing::error!(code = e.code, msg = %e.message, "server 回调处理失败");
            // 把错误写进 errorInfo，让 SAP 看到
            if !err_info.is_null() {
                unsafe {
                    let info = &mut *err_info;
                    info.code = RFC_EXTERNAL_FAILURE;
                    write_msg_to_uc_buf(&mut info.message, &e.message);
                }
            }
            RFC_EXTERNAL_FAILURE
        }
    }
}

/// 回调主体：返回 Ok 表示成功，Err 表示失败（回传 SAP）。
fn handle_call(func_handle: RFC_FUNCTION_HANDLE) -> Result<(), RfcError> {
    // 1. 取函数名（funcHandle → DescribeFunction → GetFunctionName）
    let func_name = unsafe { get_func_name(func_handle)? };

    // 2. 查 handler 配置
    let entry = {
        let map = handlers().lock().map_err(|e| RfcError {
            code: -1,
            message: format!("handler 锁毒化: {}", e),
            key: String::new(),
        })?;
        map.get(&func_name.to_uppercase())
            .ok_or_else(|| RfcError {
                code: RFC_NOT_FOUND_ENUM,
                message: format!("未注册的函数: {}", func_name),
                key: String::new(),
            })?
            .clone_entry()
    };

    // 3. 读入参
    let inputs = read_inputs(func_handle, &entry.params)?;

    // 4. POST 到 webhook
    let resp = forward_to_webhook(&entry.webhook_url, &func_name, inputs)?;

    // 5. 回填出参
    write_outputs(func_handle, &entry.params, &resp.outputs)?;

    tracing::info!(func = %func_name, "server 调用处理完成");
    Ok(())
}

/// RFC_NOT_FOUND 的枚举值（= 18，按 RFC_RC 顺序）
const RFC_NOT_FOUND_ENUM: i32 = 18;

// HandlerEntry 的克隆辅助（Vec<ParamDef> 整体克隆）
impl HandlerEntry {
    fn clone_entry(&self) -> HandlerEntryClone {
        HandlerEntryClone {
            webhook_url: self.webhook_url.clone(),
            params: self.params.clone(),
        }
    }
}
struct HandlerEntryClone {
    webhook_url: String,
    params: Vec<ParamDef>,
}

// ============ 回调内部辅助 ============

/// 从 funcHandle 取函数名（大写）
unsafe fn get_func_name(func_handle: RFC_FUNCTION_HANDLE) -> Result<String, RfcError> {
    let mut err = std::mem::zeroed::<RFC_ERROR_INFO>();
    let desc = RfcDescribeFunction(func_handle, &mut err);
    if desc.is_null() {
        return Err(RfcError {
            code: err.code,
            message: "RfcDescribeFunction 失败".into(),
            key: String::new(),
        });
    }
    let mut buf = [0u16; 31]; // RFC_ABAP_NAME 长度
    let rc = RfcGetFunctionName(desc, buf.as_mut_ptr(), &mut err);
    if rc != RFC_OK {
        return Err(RfcError {
            code: rc,
            message: "RfcGetFunctionName 失败".into(),
            key: String::new(),
        });
    }
    Ok(sap_uc_to_string(buf.as_ptr(), 30))
}

/// 读所有 import/changing 参数 → HashMap<name, string_value>
fn read_inputs(
    func_handle: RFC_FUNCTION_HANDLE,
    params: &[ParamDef],
) -> Result<HashMap<String, String>, RfcError> {
    let mut inputs = HashMap::new();
    for p in params {
        let is_input = matches!(
            p.direction.to_lowercase().as_str(),
            "import" | "changing" | "tables"
        );
        if !is_input {
            continue;
        }
        // 简化：所有类型统一按字符串读（server 端首版不区分类型）
        let val = unsafe {
            let name_uc = str_to_sap_uc(&p.name);
            let max_len = p.length.unwrap_or(255);
            function::read_string_adaptive(func_handle as *mut c_void, name_uc.as_ptr(), max_len)
        }?;
        inputs.insert(p.name.clone(), val);
    }
    Ok(inputs)
}

/// 把 webhook 响应的 outputs 回填到 funcHandle 的 export/changing 参数
fn write_outputs(
    func_handle: RFC_FUNCTION_HANDLE,
    params: &[ParamDef],
    outputs: &HashMap<String, String>,
) -> Result<(), RfcError> {
    for p in params {
        let is_output = matches!(p.direction.to_lowercase().as_str(), "export" | "changing");
        if !is_output {
            continue;
        }
        // webhook 必须返回该参数；缺失则跳过（参数已标记 optional）
        let Some(val) = outputs.get(&p.name) else {
            continue;
        };
        unsafe {
            let name_uc = str_to_sap_uc(&p.name);
            let val_uc = str_to_sap_uc(val);
            let mut err = std::mem::zeroed::<RFC_ERROR_INFO>();
            let rc = RfcSetString(
                func_handle as *mut c_void,
                name_uc.as_ptr(),
                val_uc.as_ptr(),
                val.chars().count() as i32,
                &mut err,
            );
            if rc != RFC_OK {
                return Err(RfcError {
                    code: rc,
                    message: format!(
                        "回填参数失败 ({}): {}",
                        p.name,
                        sap_uc_to_string(err.message.as_ptr(), 256)
                    ),
                    key: String::new(),
                });
            }
        }
    }
    Ok(())
}

/// 把 UTF-8 错误消息写入 SAP UC 缓冲区（0 终止）
unsafe fn write_msg_to_uc_buf(buf: &mut [SAP_UC], msg: &str) {
    let uc: Vec<u16> = msg.encode_utf16().collect();
    let copy_len = uc.len().min(buf.len() - 1);
    buf[..copy_len].copy_from_slice(&uc[..copy_len]);
    if copy_len < buf.len() {
        buf[copy_len] = 0;
    }
}

/// 同步 POST 到 webhook
fn forward_to_webhook(
    url: &str,
    func: &str,
    inputs: HashMap<String, String>,
) -> Result<WebhookResponse, RfcError> {
    let req = WebhookRequest { func, inputs };
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| RfcError {
            code: -1,
            message: format!("HTTP 客户端构造失败: {}", e),
            key: String::new(),
        })?;

    let resp = client.post(url).json(&req).send().map_err(|e| RfcError {
        code: -1,
        message: format!("webhook 请求失败 ({}): {}", url, e),
        key: String::new(),
    })?;

    let resp_body = resp
        .error_for_status()
        .map_err(|e| RfcError {
            code: -1,
            message: format!("webhook 返回错误: {}", e),
            key: String::new(),
        })?
        .json::<WebhookResponse>()
        .map_err(|e| RfcError {
            code: -1,
            message: format!("webhook 响应解析失败: {}", e),
            key: String::new(),
        })?;

    Ok(resp_body)
}

// ============ 启动入口 ============

/// 启动 SAP Server：注册到 Gateway + dispatch 循环（阻塞当前线程）。
///
/// 在独立 OS 线程调用。返回即表示 server 停止（gateway 断开或致命错误）。
pub fn run(cfg: &ServerConfig) -> Result<(), RfcError> {
    // 1. 注册 handler（构造描述符 + 安装回调）
    let _desc_handles = install_handlers(cfg)?;
    tracing::info!("所有 handler 注册完成");

    // 2. 注册到 Gateway
    let server_handle = register_gateway(&cfg.gateway)?;
    tracing::info!(
        program_id = %cfg.gateway.program_id,
        gwhost = %cfg.gateway.gwhost,
        "已注册到 SAP Gateway，开始监听入站调用"
    );

    // 3. dispatch 循环
    dispatch_loop(server_handle);

    Ok(())
}

/// 构造连接参数并 RfcRegisterServer
fn register_gateway(
    gw: &crate::server_config::GatewayConfig,
) -> Result<RFC_CONNECTION_HANDLE, RfcError> {
    let params: Vec<(&str, &str)> = vec![
        ("GWHOST", &gw.gwhost),
        ("GWSERV", &gw.gwserv),
        ("PROGRAM_ID", &gw.program_id),
    ];
    let owned: Vec<(Vec<u16>, Vec<u16>)> = params
        .iter()
        .map(|(k, v)| (str_to_sap_uc(k), str_to_sap_uc(v)))
        .collect();
    let rfc_params: Vec<RFC_CONNECTION_PARAMETER> = owned
        .iter()
        .map(|(k, v)| RFC_CONNECTION_PARAMETER {
            name: k.as_ptr(),
            value: v.as_ptr(),
        })
        .collect();

    unsafe {
        let mut err = std::mem::zeroed::<RFC_ERROR_INFO>();
        let h = RfcRegisterServer(rfc_params.as_ptr(), rfc_params.len() as u32, &mut err);
        if h.is_null() {
            return Err(RfcError {
                code: err.code,
                message: format!(
                    "注册 Gateway 失败: {}",
                    sap_uc_to_string(err.message.as_ptr(), 256)
                ),
                key: sap_uc_to_string(err.key.as_ptr(), 64),
            });
        }
        Ok(h)
    }
}

/// dispatch 循环：阻塞监听，timeout=5s 轮询让循环可响应停止。
fn dispatch_loop(handle: RFC_CONNECTION_HANDLE) {
    loop {
        let mut err = unsafe { std::mem::zeroed::<RFC_ERROR_INFO>() };
        let rc = unsafe { RfcListenAndDispatch(handle, 5, &mut err) };
        match rc {
            RFC_OK => {
                tracing::debug!("一次入站调用处理完成");
            }
            RFC_RETRY => {
                // 5s 无调用，继续
            }
            RFC_CLOSED => {
                tracing::info!("Gateway 连接关闭，server 退出");
                break;
            }
            _ => {
                let msg = unsafe { sap_uc_to_string(err.message.as_ptr(), 256) };
                tracing::error!(rc, msg = %msg, "dispatch 错误，server 退出");
                break;
            }
        }
    }
}
