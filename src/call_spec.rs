use crate::connection::RfcConnection;
use crate::error::RfcError;
use crate::function::RfcFunction;

/// 单个参数值的通用表达，支持字符串、整数、表三种类型
#[derive(Debug, Clone)]
pub enum RfcParamValue {
    Chars(String),
    Int(i32),
    /// 输入表：每个元素是一行，行内是 (字段名 -> 字段值) 的映射
    Table(Vec<Vec<(&'static str, RfcParamValue)>>),
}

/// 一次 RFC 调用的完整描述 —— 改这里就能切换不同 BAPI
#[derive(Debug)]
pub struct RfcCallSpec {
    pub func_name: String,
    /// 标量输入参数（字符串 / 整数）
    pub inputs: Vec<(String, RfcParamValue)>,
    /// 输入表参数（与 inputs 平行：参数名 -> 行数据）
    pub table_inputs: Vec<(String, Vec<Vec<(&'static str, RfcParamValue)>>)>,
    /// 需要打印的整型输出字段
    pub int_outputs: Vec<String>,
    /// 需要打印的字符串输出字段（参数名 -> 最大长度）
    pub string_outputs: Vec<(String, usize)>,
    /// 需要遍历打印的输出表（表名 -> 行字段定义）
    pub table_outputs: Vec<(String, Vec<(String, usize)>)>,
}

/// 应用单个标量输入
fn apply_scalar_input(
    func: &mut RfcFunction,
    name: &str,
    value: &RfcParamValue,
) -> Result<(), RfcError> {
    match value {
        RfcParamValue::Chars(s) => func.set_chars(name, s),
        RfcParamValue::Int(i) => func.set_int(name, *i),
        RfcParamValue::Table(_) => Err(RfcError {
            code: -1,
            message: format!("参数 {} 应使用 table_inputs，不应放在 inputs 里", name),
            key: String::new(),
        }),
    }
}

/// 应用表输入
fn apply_table_input(
    func: &mut RfcFunction,
    name: &str,
    rows: &[Vec<(&'static str, RfcParamValue)>],
) -> Result<(), RfcError> {
    let mut table = func.get_table(name)?;
    for row_spec in rows {
        let row = table.append_row()?;
        for (field_name, field_value) in row_spec {
            apply_scalar_input_struct(&row, field_name, field_value)?;
        }
    }
    Ok(())
}

/// 行对象（结构体）的标量赋值
fn apply_scalar_input_struct(
    row: &crate::function::RfcRow,
    name: &str,
    value: &RfcParamValue,
) -> Result<(), RfcError> {
    match value {
        RfcParamValue::Chars(s) => row.set_chars(name, s),
        RfcParamValue::Int(i) => row.set_int(name, *i),
        RfcParamValue::Table(_) => Err(RfcError {
            code: -1,
            message: format!("表行字段 {} 不允许嵌套表", name),
            key: String::new(),
        }),
    }
}

/// 通用执行入口：传入 spec，自动完成"取函数 → 填参数 → invoke → 读结果 → 打印"全过程
pub fn execute(conn: &RfcConnection, spec: RfcCallSpec) -> Result<(), RfcError> {
    println!("\n=== 调用 RFC: {} ===", spec.func_name);

    let mut func = conn.get_function(&spec.func_name)?;

    // 1. 填标量输入
    for (name, value) in &spec.inputs {
        apply_scalar_input(&mut func, name, value)?;
    }

    // 2. 填表输入
    for (name, rows) in &spec.table_inputs {
        apply_table_input(&mut func, name, rows)?;
    }

    // 3. 执行
    func.invoke()?;

    // 4. 打印整型输出
    for name in &spec.int_outputs {
        let v = func.get_int(name)?;
        println!("📊 {} = {}", name, v);
    }

    // 5. 打印字符串输出
    for (name, max_len) in &spec.string_outputs {
        let v = func.get_chars(name, *max_len)?;
        println!("📝 {} = {}", name, v);
    }

    // 6. 遍历打印表输出
    for (table_name, fields) in &spec.table_outputs {
        let table = func.get_table(table_name)?;
        let count = table.row_count()?;
        println!("📋 {} ({} 行):", table_name, count);
        for i in 0..count {
            let row = table.get_row(i)?;
            let mut parts = Vec::new();
            for (field, max_len) in fields {
                let v = row.get_chars(field, *max_len)?;
                parts.push(format!("{}={}", field, v));
            }
            println!("  [{}] {}", i + 1, parts.join(" | "));
        }
    }

    // 7. 始终尝试打印 RETURN 表（如果存在）
    if let Ok(ret) = func.get_table("RETURN") {
        let count = ret.row_count()?;
        if count > 0 {
            println!("📨 RETURN ({} 行):", count);
            for i in 0..count {
                let row = ret.get_row(i)?;
                let msg_type = row.get_chars("TYPE", 1).unwrap_or_default();
                let msg_id = row.get_chars("ID", 20).unwrap_or_default();
                let msg_no = row.get_chars("NUMBER", 3).unwrap_or_default();
                let message = row.get_chars("MESSAGE", 220).unwrap_or_default();
                println!("  [{}] {} | {} | {} | {}", i + 1, msg_type, msg_id, msg_no, message);
            }
        }
    }

    Ok(())
}