use std::path::PathBuf;

fn main() {
    // SDK 根目录，可根据实际路径调整
    let sdk_dir = PathBuf::from("./nwrfcsdk");
    let lib_dir = sdk_dir.join("lib");

    // 配置库文件搜索路径
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // 链接 SAP RFC 动态库
    println!("cargo:rustc-link-lib=dylib=sapnwrfc");
}