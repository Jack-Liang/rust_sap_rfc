use crate::ffi::SAP_UC;

/// Rust UTF-8 字符串 → SAP UTF-16 宽字符数组（自动补末尾 0 终止符）
/// 对应 C SDK 中的 cU("xxx") 宏
pub fn str_to_sap_uc(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// SAP UTF-16 指针 → Rust UTF-8 String
/// # Safety
/// 调用方需保证指针有效，且 max_len 不越界
pub unsafe fn sap_uc_to_string(ptr: *const SAP_UC, max_len: usize) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // 统计字符串实际长度（遇到 0 终止符结束）
    let mut len = 0;
    while len < max_len && *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(ptr, len);
    String::from_utf16_lossy(slice)
}