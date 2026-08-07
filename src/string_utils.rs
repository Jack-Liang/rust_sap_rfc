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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ascii() {
        let s = "Hello SAP";
        let uc = str_to_sap_uc(s);
        // 应以 0 结尾
        assert_eq!(*uc.last().unwrap(), 0);
        // 往返回来
        let back = unsafe { sap_uc_to_string(uc.as_ptr(), uc.len()) };
        assert_eq!(back, s);
    }

    #[test]
    fn roundtrip_unicode() {
        // 中文、emoji：验证 UTF-8↔UTF-16 桥接不损坏
        let s = "你好 RFC 🚀";
        let uc = str_to_sap_uc(s);
        let back = unsafe { sap_uc_to_string(uc.as_ptr(), uc.len()) };
        assert_eq!(back, s);
    }

    #[test]
    fn null_pointer_returns_empty() {
        let s = unsafe { sap_uc_to_string(std::ptr::null(), 10) };
        assert_eq!(s, "");
    }

    #[test]
    fn respects_max_len_and_terminator() {
        // uc = ['A','B','C', 0]
        let uc = str_to_sap_uc("ABC");
        // max_len=2 应只读前两个字符
        let s = unsafe { sap_uc_to_string(uc.as_ptr(), 2) };
        assert_eq!(s, "AB");
        // max_len 大于实际，遇到 0 终止符停止
        let s2 = unsafe { sap_uc_to_string(uc.as_ptr(), 100) };
        assert_eq!(s2, "ABC");
    }

    #[test]
    fn str_to_sap_uc_appends_nul() {
        let uc = str_to_sap_uc("X");
        assert_eq!(uc, vec![b'X' as u16, 0]);
    }
}
