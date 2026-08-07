use std::path::PathBuf;

fn main() {
    // SDK 根目录，可根据实际路径调整
    let sdk_dir = PathBuf::from("./nwrfcsdk");

    // 根据目标平台三元组选择对应的 lib 子目录。
    // 目录约定：nwrfcsdk/lib/<os>-<arch>/
    //   - windows-x86_64   sapnwrfc.dll + sapnwrfc.lib + ICU dlls
    //   - linux-x86_64     libsapnwrfc.so (+ libsapucum.so)
    //   - linux-aarch64    同上 ARM64
    //   - darwin-x86_64    libsapnwrfc.dylib
    //   - darwin-aarch64   同上 Apple Silicon
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_dir = format!("{}-{}", os, arch);

    let lib_dir = sdk_dir.join("lib").join(&target_dir);

    if !lib_dir.exists() {
        panic!(
            "未找到目标平台 [{target_dir}] 的 SAP NWRFC SDK 库目录：{}\n\
             请创建该目录并放入对应平台的 SDK 文件（见 nwrfcsdk/README.md）。",
            lib_dir.display()
        );
    }

    // 检查目录内确实存在库文件（区分于「只有占位说明文件的空目录」）
    let has_lib = std::fs::read_dir(&lib_dir)
        .map(|entries| {
            entries.filter_map(Result::ok).any(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                name.ends_with(".dll") || name.ends_with(".so") || name.ends_with(".dylib")
            })
        })
        .unwrap_or(false);

    if !has_lib {
        panic!(
            "目标平台 [{target_dir}] 的库目录中未找到任何 SDK 库文件 (.dll/.so/.dylib)：{}\n\
             目录可能只含占位说明。请从 SAP 下载对应平台的 NWRFC SDK 并把\n\
             sapnwrfc 的动态库放入该目录（详见 nwrfcsdk/README.md）。",
            lib_dir.display()
        );
    }

    // 配置库文件搜索路径
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    // 链接 SAP RFC 动态库（Windows: sapnwrfc.dll, Linux: libsapnwrfc.so）
    println!("cargo:rustc-link-lib=dylib=sapnwrfc");

    println!("cargo:warning=使用 SAP NWRFC SDK: {}", target_dir);
    println!("cargo:rerun-if-changed=build.rs");
}
