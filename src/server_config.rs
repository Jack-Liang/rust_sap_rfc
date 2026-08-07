//! Server 模式配置：解析 `servers.toml`，定义 SAP→HTTP 反向代理的函数映射。
//!
//! 配置描述：连到哪个 SAP Gateway、注册什么 Program ID、暴露哪些 RFC 函数、
//! 每个函数的参数定义和 webhook URL。

use serde::{Deserialize, Serialize};

/// `servers.toml` 顶层结构
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    /// Gateway 连接参数
    pub gateway: GatewayConfig,
    /// 对外暴露的函数列表
    pub functions: Vec<FunctionDef>,
}

/// SAP Gateway 连接（注册模式）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    /// Gateway 主机名/IP
    pub gwhost: String,
    /// Gateway 服务，如 "sapgw00"（sysnr 00）
    pub gwserv: String,
    /// Program ID，必须与 SM59 里 RFC Destination（Type T）的 Program ID 一致
    pub program_id: String,
}

/// 单个对外暴露的 RFC 函数
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionDef {
    /// 函数名（SAP 通过 CALL FUNCTION ... DESTINATION 调用此名）
    pub name: String,
    /// 收到调用时转发到的 HTTP webhook URL
    pub webhook_url: String,
    /// 参数定义列表
    pub params: Vec<ParamDef>,
}

/// 单个参数定义
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParamDef {
    /// 参数名（大写，ABAP 命名）
    pub name: String,
    /// 方向：import / export / changing / tables
    pub direction: String,
    /// 类型：char / int / float / bcd / date / time / num / byte / xstring
    #[serde(rename = "type")]
    pub type_: String,
    /// 字符长度（char/num/date/time 等有意义；int/float 可省略）
    #[serde(default)]
    pub length: Option<usize>,
}

impl ParamDef {
    /// 方向转 SDK 的 RFC_DIRECTION 位掩码值
    pub fn direction_mask(&self) -> Result<i32, String> {
        match self.direction.to_lowercase().as_str() {
            "import" => Ok(crate::ffi::RFC_DIRECTION_IMPORT),
            "export" => Ok(crate::ffi::RFC_DIRECTION_EXPORT),
            "changing" => Ok(crate::ffi::RFC_DIRECTION_CHANGING),
            "tables" => Ok(crate::ffi::RFC_DIRECTION_TABLES),
            other => Err(format!("未知参数方向 '{}': {}", self.name, other)),
        }
    }

    /// 类型字符串转 RFCTYPE 常量值
    pub fn rfc_type(&self) -> Result<i32, String> {
        match self.type_.to_lowercase().as_str() {
            "char" => Ok(crate::ffi::rfctype::CHAR),
            "date" => Ok(crate::ffi::rfctype::DATE),
            "time" => Ok(crate::ffi::rfctype::TIME),
            "num" => Ok(crate::ffi::rfctype::NUM),
            "int" => Ok(crate::ffi::rfctype::INT),
            "int2" => Ok(crate::ffi::rfctype::INT2),
            "int1" => Ok(crate::ffi::rfctype::INT1),
            "float" => Ok(crate::ffi::rfctype::FLOAT),
            "byte" => Ok(crate::ffi::rfctype::BYTE),
            "xstring" => Ok(crate::ffi::rfctype::XSTRING),
            "bcd" => Ok(crate::ffi::rfctype::BCD),
            "string" => Ok(crate::ffi::rfctype::STRING),
            other => Err(format!("未知参数类型 '{}': {}", self.name, other)),
        }
    }
}

/// 从文件加载配置
pub fn load(path: &str) -> Result<ServerConfig, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {}", path, e))?;
    let cfg: ServerConfig =
        toml::from_str(&content).map_err(|e| format!("解析 {} 失败: {}", path, e))?;
    cfg.validate()?;
    Ok(cfg)
}

impl ServerConfig {
    /// 基本校验
    fn validate(&self) -> Result<(), String> {
        if self.functions.is_empty() {
            return Err("未定义任何函数（[functions]] 为空）".into());
        }
        for f in &self.functions {
            for p in &f.params {
                p.direction_mask()?;
                p.rfc_type()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[gateway]
gwhost = "sap.example.com"
gwserv = "sapgw00"
program_id = "ZTEST"

[[functions]]
name = "Z_ECHO"
webhook_url = "http://localhost:9000/echo"

[[functions.params]]
name = "INPUT"
direction = "import"
type = "char"
length = 255

[[functions.params]]
name = "OUTPUT"
direction = "export"
type = "char"
length = 255
"#;
        let cfg: ServerConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.gateway.gwhost, "sap.example.com");
        assert_eq!(cfg.gateway.program_id, "ZTEST");
        assert_eq!(cfg.functions.len(), 1);
        assert_eq!(cfg.functions[0].name, "Z_ECHO");
        assert_eq!(cfg.functions[0].params.len(), 2);
        assert_eq!(cfg.functions[0].params[0].length, Some(255));
    }

    #[test]
    fn direction_mask_mapping() {
        let mk = |d: &str| ParamDef {
            name: "X".into(),
            direction: d.into(),
            type_: "char".into(),
            length: None,
        };
        assert_eq!(mk("import").direction_mask().unwrap(), 0x01);
        assert_eq!(mk("EXPORT").direction_mask().unwrap(), 0x02);
        assert_eq!(mk("changing").direction_mask().unwrap(), 0x03);
        assert_eq!(mk("tables").direction_mask().unwrap(), 0x07);
        assert!(mk("invalid").direction_mask().is_err());
    }

    #[test]
    fn rfc_type_mapping() {
        let mk = |t: &str| ParamDef {
            name: "X".into(),
            direction: "import".into(),
            type_: t.into(),
            length: None,
        };
        assert_eq!(mk("char").rfc_type().unwrap(), 0);
        assert_eq!(mk("INT").rfc_type().unwrap(), 8);
        assert_eq!(mk("float").rfc_type().unwrap(), 7);
        assert!(mk("unknown").rfc_type().is_err());
    }

    #[test]
    fn validate_rejects_empty_functions() {
        let cfg = ServerConfig {
            gateway: GatewayConfig {
                gwhost: "h".into(),
                gwserv: "sapgw00".into(),
                program_id: "P".into(),
            },
            functions: vec![],
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_bad_direction() {
        let cfg = ServerConfig {
            gateway: GatewayConfig {
                gwhost: "h".into(),
                gwserv: "sapgw00".into(),
                program_id: "P".into(),
            },
            functions: vec![FunctionDef {
                name: "Z".into(),
                webhook_url: "http://x".into(),
                params: vec![ParamDef {
                    name: "P".into(),
                    direction: "sideways".into(),
                    type_: "char".into(),
                    length: None,
                }],
            }],
        };
        assert!(cfg.validate().is_err());
    }
}
