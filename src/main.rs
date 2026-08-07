mod ffi;
mod string_utils;
mod error;
mod connection;
mod function;
mod call_spec;

use call_spec::{execute, RfcCallSpec, RfcParamValue};
use connection::RfcConnection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Rust 手写 SAP RFC - 配置化调用 ===");

    // 1. 建立连接（连接信息在此处配置）
    let conn = RfcConnection::new(&[
        ("ASHOST", "localhost"),
        ("SYSNR", "00"),
        ("CLIENT", "001"),
        ("USER", "DEVELOPER"),
        ("PASSWD", "ABAPtr2023#00"),
        ("LANG", "EN"),
    ])?;
    println!("✅ SAP 系统连接成功");

    // ============ 以下为调用定义区：增删/替换这里即可切换 BAPI ============

    // 示例 1：STFC_CONNECTION（基础连通测试）
    let spec1 = RfcCallSpec {
        func_name: "STFC_CONNECTION".into(),
        inputs: vec![("REQUTEXT".into(), RfcParamValue::Chars("Hello from Rust!".into()))],
        table_inputs: vec![],
        int_outputs: vec![],
        string_outputs: vec![("ECHOTEXT".into(), 255), ("RESPTEXT".into(), 255)],
        table_outputs: vec![],
    };

    // 示例 2：BAPI_USER_GETLIST（取全部用户）
    let spec2 = RfcCallSpec {
        func_name: "BAPI_USER_GETLIST".into(),
        inputs: vec![
            ("MAX_ROWS".into(), RfcParamValue::Int(50)),
            ("WITH_USERNAME".into(), RfcParamValue::Chars("X".into())),
        ],
        table_inputs: vec![],
        int_outputs: vec!["ROWS".into()],
        string_outputs: vec![],
        table_outputs: vec![(
            "USERLIST".into(),
            vec![
                ("USERNAME".into(), 12),
                ("FIRSTNAME".into(), 40),
                ("LASTNAME".into(), 40),
            ],
        )],
    };

    // 示例 3：BAPI_USER_GETLIST（带过滤条件 SELECTION_RANGE）
    let spec3 = RfcCallSpec {
        func_name: "BAPI_USER_GETLIST".into(),
        inputs: vec![
            ("MAX_ROWS".into(), RfcParamValue::Int(10)),
            ("WITH_USERNAME".into(), RfcParamValue::Chars("X".into())),
        ],
        table_inputs: vec![(
            "SELECTION_RANGE".into(),
            vec![vec![
                ("PARAMETER", RfcParamValue::Chars("USERNAME".into())),
                ("FIELD", RfcParamValue::Chars("".into())),
                ("SIGN", RfcParamValue::Chars("I".into())),
                ("OPTION", RfcParamValue::Chars("CP".into())),
                ("LOW", RfcParamValue::Chars("D*".into())),
                ("HIGH", RfcParamValue::Chars("".into())),
            ]],
        )],
        int_outputs: vec!["ROWS".into()],
        string_outputs: vec![],
        table_outputs: vec![(
            "USERLIST".into(),
            vec![
                ("USERNAME".into(), 12),
                ("FIRSTNAME".into(), 40),
                ("LASTNAME".into(), 40),
            ],
        )],
    };

    // ============ 执行区：选择要跑的 spec ============
    execute(&conn, spec2)?; // BAPI_USER_GETLIST 全量
    execute(&conn, spec1)?; // STFC_CONNECTION
    // execute(&conn, spec3)?; // BAPI_USER_GETLIST 过滤版（按需开启）

    println!("\n✅ 全部调用完成，资源已自动释放");
    Ok(())
}