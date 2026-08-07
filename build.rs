use std::path::PathBuf;

fn main() {
    // SDK 根目录查找顺序：
    //   1) 环境变量 SAP_SDK_DIR（推荐用于 Docker、CI、自定义安装路径）
    //   2) 环境变量 SAP_SDK_HOST_PATH（兼容 docker-compose.yml 里的旧名）
    //   3) ./nwrfcsdk（默认，仓库内子目录）
    let sdk_dir = std::env::var("SAP_SDK_DIR")
        .or_else(|_| std::env::var("SAP_SDK_HOST_PATH"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./nwrfcsdk"));

    // 根据目标平台三元组选择对应的 lib 子目录。
    // 目录约定：<sdk_dir>/lib/<os>-<arch>/
    //   - windows-x86_64   sapnwrfc.dll + sapnwrfc.lib + ICU dlls
    //   - linux-x86_64     libsapnwrfc.so (+ libsapucum.so)
    //   - linux-aarch64    同上 ARM64
    //   - darwin-x86_64    libsapnwrfc.dylib
    //   - darwin-aarch64    同上 Apple Silicon
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_dir = format!("{}-{}", os, arch);

    let lib_dir = sdk_dir.join("lib").join(&target_dir);

    if !lib_dir.exists() {
        panic!(
            "未找到目标平台 [{target_dir}] 的 SAP NWRFC SDK 库目录：{}\n\
             请创建该目录并放入对应平台的 SDK 文件（见 nwrfcsdk/README.md）。\n\
             也可通过环境变量 SAP_SDK_DIR 指向已安装的 SDK 根目录。",
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
             sapnwrfc 的动态库放入该目录（详见 nwrfcsdk/README.md）。\n\
             也可通过环境变量 SAP_SDK_DIR 指向已安装的 SDK 根目录。",
            lib_dir.display()
        );
    }

    // 配置库文件搜索路径
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    // 链接 SAP RFC 动态库（Windows: sapnwrfc.dll, Linux: libsapnwrfc.so）
    println!("cargo:rustc-link-lib=dylib=sapnwrfc");

    // CI stub 构建兼容：当 libsapnwrfc.so/.dylib 是空占位库（只含 stub 符号，
    // 没有 RfcOpenConnection 等真实符号）时，链接器会因 undefined symbol 报错。
    // 允许未定义符号在运行时解析（用户挂载真实 SDK 后即可正常调用）。
    // 真实 SDK 存在时符号会被正常解析，此选项无副作用。
    if os == "linux" {
        println!("cargo:rustc-link-arg=-Wl,--unresolved-symbols=ignore-all");
    } else if os == "macos" {
        println!("cargo:rustc-link-arg=-Wl,-undefined,dynamic_lookup");
    }

    println!("cargo:warning=使用 SAP NWRFC SDK: {}", target_dir);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SAP_SDK_DIR");
    println!("cargo:rerun-if-env-changed=SAP_SDK_HOST_PATH");
}
