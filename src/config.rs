//! 配置加载：从环境变量（.env 文件或真实环境变量）读取 SAP 连接参数与监听地址。
//!
//! 必填项缺失时给出明确的中文报错，便于部署排查。

use std::env;

/// 服务运行配置
#[derive(Debug)]
pub struct AppConfig {
    /// SAP 连接参数（已组装成 RfcConnection::new 所需的键值对）
    pub conn_params: Vec<(&'static str, String)>,
    /// HTTP 监听地址，如 "127.0.0.1:3000"
    pub listen_addr: String,
    /// SAP 连接池上限（并发 SAP 调用数）
    pub pool_size: usize,
}

fn required(key: &str) -> Result<String, String> {
    env::var(key).map_err(|_| format!("缺少必填环境变量: {}", key))
}

/// 从环境变量读取配置。调用方应先执行 dotenvy::dotenv()。
pub fn load() -> Result<AppConfig, String> {
    let ashost = required("SAP_ASHOST")?;
    let sysnr = required("SAP_SYSNR")?;
    let client = required("SAP_CLIENT")?;
    let user = required("SAP_USER")?;
    let passwd = required("SAP_PASSWD")?;
    let lang = env::var("SAP_LANG").unwrap_or_else(|_| "EN".to_string());
    let listen_addr = env::var("SAP_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let pool_size = env::var("SAP_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(8);

    // 键为 'static 字面量，值使用环境变量的 String（运行期存活）
    Ok(AppConfig {
        conn_params: vec![
            ("ASHOST", ashost),
            ("SYSNR", sysnr),
            ("CLIENT", client),
            ("USER", user),
            ("PASSWD", passwd),
            ("LANG", lang),
        ],
        listen_addr,
        pool_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用的唯一键前缀，避免与真实环境变量或并行测试冲突。
    /// load() 读的是固定键名，所以这里用 set/unset 真实键，但所有测试串行运行。
    /// Cargo 默认多线程并行，故用互斥的 set_var + remove_var 配合串行 attribute。
    use std::sync::Mutex;

    // 保证本模块内测试串行执行（环境变量是全局状态）
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        for k in [
            "SAP_ASHOST",
            "SAP_SYSNR",
            "SAP_CLIENT",
            "SAP_USER",
            "SAP_PASSWD",
            "SAP_LANG",
            "SAP_LISTEN_ADDR",
        ] {
            unsafe {
                std::env::remove_var(k);
            }
        }
    }

    fn set_all_required() {
        unsafe {
            std::env::set_var("SAP_ASHOST", "sap.example.com");
            std::env::set_var("SAP_SYSNR", "00");
            std::env::set_var("SAP_CLIENT", "100");
            std::env::set_var("SAP_USER", "TESTUSER");
            std::env::set_var("SAP_PASSWD", "secret");
        }
    }

    #[test]
    fn load_success_with_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        set_all_required();
        let cfg = load().unwrap();
        assert_eq!(
            cfg.conn_params[0],
            ("ASHOST", "sap.example.com".to_string())
        );
        assert_eq!(cfg.conn_params[1], ("SYSNR", "00".to_string()));
        assert_eq!(cfg.listen_addr, "127.0.0.1:3000"); // 默认值
                                                       // LANG 默认 EN
        assert_eq!(cfg.conn_params[5], ("LANG", "EN".to_string()));
    }

    #[test]
    fn load_respects_lang_and_listen_addr() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        set_all_required();
        unsafe {
            std::env::set_var("SAP_LANG", "ZH");
            std::env::set_var("SAP_LISTEN_ADDR", "0.0.0.0:8080");
        }
        let cfg = load().unwrap();
        assert_eq!(cfg.conn_params[5], ("LANG", "ZH".to_string()));
        assert_eq!(cfg.listen_addr, "0.0.0.0:8080");
    }

    #[test]
    fn load_missing_ashost_errors() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        // 只设其他必填项，故意漏 ASHOST
        unsafe {
            std::env::set_var("SAP_SYSNR", "00");
            std::env::set_var("SAP_CLIENT", "100");
            std::env::set_var("SAP_USER", "U");
            std::env::set_var("SAP_PASSWD", "P");
        }
        let err = load().unwrap_err();
        assert!(
            err.contains("SAP_ASHOST"),
            "错误信息应指出缺失的变量: {}",
            err
        );
    }

    #[test]
    fn load_missing_passwd_errors() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        unsafe {
            std::env::set_var("SAP_ASHOST", "h");
            std::env::set_var("SAP_SYSNR", "00");
            std::env::set_var("SAP_CLIENT", "100");
            std::env::set_var("SAP_USER", "U");
            // 故意漏 PASSWD
        }
        let err = load().unwrap_err();
        assert!(err.contains("SAP_PASSWD"));
    }
}
