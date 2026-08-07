# SAP NWRFC SDK 占位目录

本目录存放 **SAP NetWeaver RFC SDK** 的二进制文件，用于本项目的编译链接与运行时加载。

> ⚠️ **SAP SDK 受版权限制，不能随本仓库分发。**
> 二进制文件已在 `.gitignore` 中排除，不会上传。每位开发者/构建环境需自行从 SAP 获取并放入本目录。

---

## 1. 获取 SDK

1. 登录 [SAP Support Portal](https://launchpad.support.sap.com)
2. 进入 **Software Downloads**（维护 → SAP NW RFC SDK）
3. 搜索 `SAP NW RFC SDK`，按目标平台下载压缩包
4. 解压后，把对应平台的库文件放到下方对应目录

参考：[SAP Note 2573790 - SAP NW RFC SDK](https://launchpad.support.sap.com/#/notes/2573790)

---

## 2. 目录结构与平台支持

`build.rs` 会根据 `cargo` 的目标三元组（`CARGO_CFG_TARGET_OS` + `CARGO_CFG_TARGET_ARCH`）
自动从 `lib/<os>-<arch>/` 子目录选取库文件。你需要为目标平台创建对应子目录：

```
nwrfcsdk/
├── README.md                      ← 本文件（已纳入版本控制）
├── include/                       ← 头文件（可选，仅查阅参考时用）
│   ├── sapnwrfc.h
│   ├── sapdecf.h
│   └── sapucrfc.h
└── lib/
    ├── windows-x86_64/            ← Windows x64
    │   ├── sapnwrfc.dll
    │   ├── sapnwrfc.lib
    │   ├── libsapucum.dll
    │   ├── libsapucum.lib
    │   ├── icudt57.dll
    │   ├── icuin57.dll
    │   └── icuuc57.dll
    ├── linux-x86_64/              ← Linux x86_64
    │   ├── libsapnwrfc.so
    │   └── libsapucum.so
    ├── linux-aarch64/             ← Linux ARM64（如 AWS Graviton、树莓派4）
    │   ├── libsapnwrfc.so
    │   └── libsapucum.so
    ├── darwin-x86_64/             ← macOS Intel
    │   ├── libsapnwrfc.dylib
    │   └── libsapucum.dylib
    └── darwin-aarch64/            ← macOS Apple Silicon (M1/M2/M3)
        ├── libsapnwrfc.dylib
        └── libsapucum.dylib
```

> 你只需为**实际使用的目标平台**准备对应子目录即可，不必齐全。
> 例如只在 Windows 上开发、在 Linux 容器里部署，那只准备
> `windows-x86_64/` 和 `linux-x86_64/` 两个目录。

### 哪个子目录会被选中？

由 `cargo` 的 `--target` 决定（默认是宿主机平台）：

| 构建目标 | 选中的子目录 |
|---|---|
| 默认（不传 `--target`） | 宿主机平台，如本机 Windows → `windows-x86_64` |
| `--target x86_64-unknown-linux-gnu` | `linux-x86_64` |
| `--target aarch64-unknown-linux-gnu` | `linux-aarch64` |
| `--target x86_64-pc-windows-msvc` | `windows-x86_64` |
| `--target aarch64-apple-darwin` | `darwin-aarch64` |

若 `build.rs` 找不到对应子目录，会 panic 给出明确报错。

---

## 3. 各平台文件清单

每个 `<os>-<arch>/` 子目录里需要哪些文件，取决于平台。

### Windows (`windows-x86_64`)

| 文件 | 必需 | 来源 |
|---|:---:|---|
| `sapnwrfc.dll` | ✅ | SDK 的 `bin/` 或 `lib/`（不同版本位置略不同） |
| `sapnwrfc.lib` | ✅ | 编译链接需要 |
| `libsapucum.dll` | ✅ | SAP Unicode 运行时 |
| `libsapucum.lib` | ✅ | 同上的导入库 |
| `icudt57.dll` / `icuin57.dll` / `icuuc57.dll` | ✅ | ICU 国际化组件，随 SDK 附带 |

> Windows 版 SDK 解压后通常直接含上述 dll 与 lib，复制过来即可。

### Linux (`linux-x86_64` / `linux-aarch64`)

| 文件 | 必需 | 备注 |
|---|:---:|---|
| `libsapnwrfc.so` | ✅ | 主库 |
| `libsapucum.so` | ✅ | SAP Unicode 运行时 |

> Linux 版 SDK 通常把这些 `.so` 放在解压目录的 `lib/` 下。
> 运行时还需 `libssl` 等系统依赖（见 Dockerfile）。
> ARM64 版本需从 SAP 下载 `NWRFC SDK Linux on ARM` 变体。

### macOS (`darwin-x86_64` / `darwin-aarch64`)

| 文件 | 必需 | 备注 |
|---|:---:|---|
| `libsapnwrfc.dylib` | ✅ | 主库 |
| `libsapucum.dylib` | ✅ | SAP Unicode 运行时 |

> SAP 官方对 macOS 的支持有限，建议优先用 Linux 容器部署。

---

## 4. 验证安装

放好文件后，在项目根目录运行：

```bash
cargo build
```

成功会看到构建提示，例如：

```
warning: rust_sap_rfc_demo@0.1.0: 使用 SAP NWRFC SDK: windows-x86_64
    Finished `dev` profile [unoptimized + debuginfo] target(s)
```

若报「未找到目标平台的 SDK 库目录」，请检查：
1. 子目录名是否准确（`<os>-<arch>` 小写，见上表）
2. 对应 `.dll` / `.so` / `.dylib` 是否在子目录内
3. `cargo --version` 与目标三元组是否匹配

---

## 5. 运行时加载

编译期 `build.rs` 配置了链接搜索路径，但**运行时**系统加载器还需能找到动态库：

| 平台 | 方式 |
|---|---|
| Windows | 把对应 `.dll` 所在目录加入 `PATH`，或与 exe 同目录 |
| Linux | 设置 `LD_LIBRARY_PATH=/path/to/nwrfcsdk/lib/linux-x86_64`，或拷到 `/usr/local/lib` 后 `ldconfig` |
| macOS | 设置 `DYLD_LIBRARY_PATH`，或用 `install_name_tool` 修正 dylib 路径 |

### Docker 部署（运行时挂载 SDK）

镜像**不含** SAP SDK（避免版权问题）。构建前需在宿主机准备好 Linux 版 SDK，构建时用作编译链接；运行时通过卷挂载注入：

```bash
# 1. 构建前：把 Linux SDK 放到 nwrfcsdk/lib/linux-x86_64/
#    （含 libsapnwrfc.so、libsapucum.so）

# 2. 构建镜像（构建期链接 Linux SDK）
docker build -t rust-sap-rfc .

# 3. 运行时挂载宿主机 SDK 目录
#    挂载点结构须为：/app/nwrfcsdk/lib/linux-x86_64/libsapnwrfc.so
docker run --rm -p 3000:3000 \
  -v /opt/sap/nwrfcsdk-linux:/app/nwrfcsdk \
  -e SAP_ASHOST=sap.example.com \
  -e SAP_SYSNR=00 \
  -e SAP_CLIENT=100 \
  -e SAP_USER=DEVELOPER \
  -e SAP_PASSWD=secret \
  rust-sap-rfc
```

宿主机的 `/opt/sap/nwrfcsdk-linux/` 应有 `lib/linux-x86_64/libsapnwrfc.so` 这样的结构（与仓库 `nwrfcsdk/` 一致）。`LD_LIBRARY_PATH` 已在镜像中设好，指向挂载点的子目录。

---

## 6. 版本兼容

本项目基于 **NWRFC SDK 7.50** 开发与测试。
旧版 SDK（如 7.20/7.40）可能缺少部分 API，建议始终使用最新 7.50 patch。
