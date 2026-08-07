# rust-sap-rfc

把 SAP NWRFC SDK 包装成常驻 HTTP 服务，提供 RESTful 接口调用任意 SAP RFC / BAPI。
其他服务无需安装 SAP SDK，发一个 JSON POST 即可调用。

- **技术栈**：Rust（标准库 FFI 直连 `sapnwrfc.dll`）+ axum + tokio + serde
- **零 SDK 依赖客户端**：调用方只要会发 HTTP POST
- **通用接口**：一个端点 `/api/rfc` 描述任意 BAPI，无需为每个 BAPI 写代码
- **面向 AI**：5 个元数据端点（搜函数/查接口/查文档/查数据字典），Agent 能自服务探索。给 AI 的操作指南见 [`AGENTS.md`](./AGENTS.md)

## 目录

- [Quick Start](#quick-start5-分钟跑起来)
- [§1 适用场景与限制](#1-适用场景与限制)
- [§2 配置参考](#2-配置参考)
- [§3 API 参考](#3-api参考) — 含 [3.3 面向 AI 的元数据 API](#33-面向-ai-的元数据-api)
- [§4 调用示例](#4-调用示例)
- [§5 常见 BAPI 速查](#5-常见-bapi-速查)
- [§6 错误处理](#6-错误处理)
- [§7 部署提示](#7-部署提示)
- [§8 架构与限制](#8-架构与限制)
- [§9 Server 端模式](#9-server-端模式被-sap-调用) — 详见 [docs/SERVER_MODE.md](./docs/SERVER_MODE.md)
- [§10 发布新版本](#10-发布新版本维护者)
- [License](#license)

---

## Quick Start（5 分钟跑起来）

> **两个东西都需要**：
> 1. **预编译二进制 / Rust 源码**：提供 HTTP 服务和 SAP 协议绑定代码
> 2. **SAP NWRFC SDK**：SAP 私有 C 库，提供实际的 SAP 通信实现（受版权限制不能随项目分发）
>
> 预编译二进制**省的是装 Rust 工具链 + 23 秒编译**，但**省不掉 SDK**——运行时仍要链接 SAP 库。
>
> 📦 **省事的做法**：把 SAP 下载的**对应平台** zip（如 macOS 上放 `nwrfcsdk-...-darwin-arm64.zip`，Linux 上放 `...-linux-x86_64.zip`）丢进 `nwrfcsdk/lib/` 下任意子目录，启动脚本会自动解压到 `<os>-<arch>/` 正确路径。详见 §「自动安装 SDK」。
>
> ⚠️ zip 必须与当前系统平台匹配（`.dylib`↔macOS、`.so`↔Linux、`.dll`↔Windows）。脚本不校验平台，放错平台的 zip 会导致解压后库文件无法加载。

### 自动安装 SDK（推荐）

`start.sh` / `start.ps1` 会按以下顺序找 SDK：

1. **环境变量 `SAP_SDK_DIR`** —— 指向已安装的 SDK 根目录（最灵活，Docker/CI 常用）
2. **`nwrfcsdk/lib/<os>-<arch>/`** —— 已放好库文件的默认路径
3. **`nwrfcsdk/lib/<任意>/nwrfcsdk-*.zip`** —— 自动识别并解压到正确路径 ✨
4. 都没有 → 报错并清晰指引

最省事：把 SAP 下载的**对应平台** zip 整个丢到 `nwrfcsdk/lib/` 下任意子目录，启动脚本会自动处理。例如（macOS Apple Silicon）：

```
nwrfcsdk/
└── lib/
    └── incoming/                  ← 随便建一个目录
        └── nwrfcsdk-...-darwin-arm64.zip   ← 必须是当前平台的 SDK
```

然后跑 `./start.sh`，脚本会自动：

- 解压 zip
- 识别 zip 内的库文件位置（SAP SDK zip 通常是 `nwrfcsdk/lib/<file>` 无平台子目录）
- 复制到 `nwrfcsdk/lib/darwin-aarch64/`（或对应平台子目录）
- 清理临时文件

解压后的真实路径仍按 SAP 官方约定保留，便于后续更新 SDK。

### 下载预编译二进制（推荐给最终用户）

不用装 Rust，直接用现成二进制：

1. 打开 [GitHub Releases](../../releases) → 选最新 tag
2. 下载对应平台的压缩包：
   - Linux x86_64: `rust_sap_rfc-x86_64-unknown-linux-gnu.tar.gz`
   - Linux ARM64: `rust_sap_rfc-aarch64-unknown-linux-gnu.tar.gz`
   - macOS Intel: `rust_sap_rfc-x86_64-apple-darwin.tar.gz`
   - macOS Apple Silicon: `rust_sap_rfc-aarch64-apple-darwin.tar.gz`
   - Windows x86_64: `rust_sap_rfc-x86_64-pc-windows-msvc.zip`
3. 解压，里面有 `rust_sap_rfc`（或 `.exe`）+ `README.md` + `.env.example` + `nwrfcsdk/` 目录骨架
4. **下载 SAP NWRFC SDK**：到 [SAP Support Portal](https://launchpad.support.sap.com) 注册账号（需 SAP 客户/合作伙伴身份），搜索 `SAP NW RFC SDK`，按平台下载 zip
5. **把 zip 放到 `nwrfcsdk/lib/` 下任意子目录**（如 `nwrfcsdk/lib/incoming/`），启动脚本会自动解压到 `<os>-<arch>/` 正确路径。**注意 zip 必须匹配当前平台**（macOS→`.dylib`、Linux→`.so`、Windows→`.dll`）
6. `cp .env.example .env` 并填 SAP 连接参数
7. 运行：
   - Linux/macOS: `./rust_sap_rfc`
   - Windows: 双击 `rust_sap_rfc.exe` 或 PowerShell 启动

> **Windows 用户**：CI 现已自动构建 Windows x86_64 二进制（通过 `.def` 文件生成 stub 导入库绕过 MSVC 链接限制）。解压 zip 后仍需自行放置 `sapnwrfc.dll`，详见第 4–5 步。

### 方式一：本地运行（开发/调试）

```bash
# 1. 安装 Rust（已装可跳过）
#    Windows: winget install Rustlang.Rustup
#    Linux/macOS: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. 放置 SAP SDK（平台对应子目录，详见 nwrfcsdk/README.md）
#    Windows: nwrfcsdk/lib/windows-x86_64/   ← 放 sapnwrfc.dll 等
#    Linux:   nwrfcsdk/lib/linux-x86_64/     ← 放 libsapnwrfc.so 等

# 3. 配置连接信息
cp .env.example .env        # 然后编辑 .env 填入 SAP 连接参数

# 4. 一键检查环境并启动（Windows 用 start.ps1，Linux/macOS 用 start.sh）
./start.sh                  # 或：powershell -File start.ps1

# 5. 验证（新开一个终端）
curl http://127.0.0.1:3000/health
# → {"status":"ok"}
```

### 方式二：Docker（部署）

```bash
# 1. 放置 Linux 版 SDK 到 nwrfcsdk/lib/linux-x86_64/（构建期需要）

# 2. 复制并填写配置
cp .env.example .env        # 编辑：填 SAP 连接参数 + SAP_SDK_HOST_PATH（见下）

# 3. 一键起（docker compose 自动 build + run + 挂载 SDK + 注入配置）
docker compose up -d --build

# 4. 验证
curl http://127.0.0.1:3000/health
```

`.env` 里需额外填一项 `SAP_SDK_HOST_PATH`：宿主机上 Linux SDK 目录的**绝对路径**（如 `C:\Users\you\sap-sdk` 或 `/opt/sap/nwrfcsdk`），compose 会把它挂载进容器的 `/app/nwrfcsdk`。该目录结构需与 `nwrfcsdk/` 一致（含 `lib/linux-x86_64/libsapnwrfc.so`）。

### 第一次调用

服务起来后，任何 HTTP 客户端都能调用：

```bash
curl -X POST http://127.0.0.1:3000/api/rfc \
  -H "Content-Type: application/json" \
  -d '{"func_name":"STFC_CONNECTION","inputs":{"REQUTEXT":"hello"},"string_outputs":{"ECHOTEXT":{"max_len":null}}}'
```

> `max_len` 留 `null` —— 服务会自动从 SAP 元数据发现字段长度，不必手填。



## 1. 适用场景与限制

**适用**

- 微服务架构中，让 Python / Node / Java 服务通过 HTTP 调 SAP
- 自动化脚本、ETL、报表后端
- 本地开发调试 BAPI

**能力边界**

| 能力 | 支持情况 |
|---|---|
| 连接 | ✅ 直连（ASHOST），自动重连，优雅停机 |
| 标量输入 | ✅ string/int/float 自动按 JSON 类型派发；BCD/INT8/二进制用显式 `{"type":"...","value":...}` |
| 标量输出 | ✅ `int_outputs`（按 INT 读）/ `string_outputs`（按字符串读，长度可自动发现） |
| 表参数（TABLES） | ✅ 输入多行 + 输出遍历（**输出字段统一按字符串读**，无类型保留） |
| 顶层结构体参数 | ✅ `struct_inputs` / `struct_outputs`（如 BAPI_USER_CREATE.ADDRESS），输出同样统一按字符串读 |
| BCD/INT8/二进制 输入 | ✅ `{"type":"BCD",...}` / `{"type":"INT8",...}` / `{"type":"BYTES",...}`（BYTES 用 Base64） |
| 元数据自动发现 | ✅ 字段长度缓存，无需手填 max_len（标量/表/结构体输出均生效） |
| Server 端（被 SAP 回调）| ✅ 配置驱动 webhook 转发（`SAP_ROLE=server`），详见 [§9](#9-server-端模式被-sap-调用) |
| tRFC/qRFC/bgRFC | ❌ 不支持 |
| SSO/SNC 安全登录 | ❌ 仅用户名密码 |

**其他限制**

| 项 | 说明 |
|---|---|
| 并发 | 多连接池（默认 8，`SAP_POOL_SIZE` 可配），不同请求可并行执行 SAP 调用 |
| 字符集 | 通过 UTF-16 桥接 SAP UC，UTF-8 输入输出 |
| 平台 | Windows/Linux/macOS × x86_64/aarch64（`build.rs` 自动选 SDK 子目录） |
| RFC 调用超时 | 当前无超时，慢请求可能长时间占用连接 |

---

## 2. 配置参考

所有配置走环境变量，写在项目根目录 `.env`（已被 gitignore，不会提交）。Quick Start 已涵盖基本用法，本节是完整字段参考。

| 变量 | 必填 | 默认 | 说明 |
|---|:---:|---|---|
| `SAP_ASHOST` | ✅ | — | SAP 应用服务器主机名/IP |
| `SAP_SYSNR` | ✅ | — | 系统号，如 `00` |
| `SAP_CLIENT` | ✅ | — | 集团号，如 `001` |
| `SAP_USER` | ✅ | — | 登录账号 |
| `SAP_PASSWD` | ✅ | — | 登录密码 |
| `SAP_LANG` | ❌ | `EN` | 登录语言（也影响文档端点的默认语言） |
| `SAP_LISTEN_ADDR` | ❌ | `127.0.0.1:3000` | HTTP 服务监听地址 |
| `SAP_POOL_SIZE` | ❌ | `8` | SAP 连接池上限（并发调用数），≥1 |
| `SAP_ROLE` | ❌ | `client` | 运行模式：`client`/`server`/`both`（server 模式见 [§9](#9-server-端模式被-sap-调用)） |
| `SAP_SDK_DIR` | ❌ | `./nwrfcsdk` | SDK 根目录（Docker/CI/自定义路径用） |

> **生产部署提示**：不要把 `.env` 放进镜像层。用容器编排系统的密钥注入（K8s Secret / Docker Swarm secret）替代。

---


## 3. API 参考

### 3.1 `GET /health`

健康检查，**不触碰 SAP**，用于探活。

```json
{ "status": "ok" }
```

---

### 3.2 `POST /api/rfc`

通用 RFC 调用。请求体描述要调哪个函数、传什么参数、要读哪些输出。

#### 请求体字段

| 字段 | 类型 | 必填 | 说明 |
|---|---|:---:|---|
| `func_name` | string | ✅ | SAP 函数模块名，如 `BAPI_USER_GETLIST` |
| `inputs` | object | ❌ | 标量输入参数：参数名 → 值（隐式：字符串→CHARS、整数→INT、浮点→FLOAT） |
| `table_inputs` | object | ❌ | 表输入参数：表名 → 行数组；每行是 字段名 → 值 |
| `struct_inputs` | object | ❌ | 顶层结构体输入：结构体名 → {字段名 → 值}（如 `ADDRESS`） |
| `int_outputs` | string[] | ❌ | 要读取的整型输出参数名列表（按 SAP `INT` 读） |
| `string_outputs` | object | ❌ | 要读取的字符串输出参数：参数名 → 最大长度。`max_len` 留 `null` 时由服务端从元数据自动发现 |
| `table_outputs` | object | ❌ | 要读取的输出表：表名 → 字段对象数组 `{"name":"...","max_len":...}` |
| `struct_outputs` | object | ❌ | 要读取的顶层结构体输出：结构体名 → 字段对象数组 |
| `read_return` | bool | ❌ | 是否自动读取 BAPI 通用 `RETURN` 消息表，默认 `false` |

**值类型规则**（`inputs` / `table_inputs` / `struct_inputs` 的字段值）：

- JSON 字符串 → SAP `CHARS`（如 `"X"`, `"D*"`）
- JSON 整数   → SAP `INT`（如 `50`）
- JSON 浮点   → SAP `FLOAT`（如 `123.45`）
- 显式类型（用于 BCD/INT8/二进制）：`{"type":"BCD","value":"999.99"}`、`{"type":"INT8","value":9876543210}`、`{"type":"BYTES","value":"<Base64>"}`

类型由调用方通过 JSON 字面量或显式 `type` 决定，服务端不猜。

#### 响应体

```jsonc
{
  "func": "BAPI_USER_GETLIST",                // 回显函数名
  "scalars": {                                 // 标量输出
    "ROWS": 50,                                //   int_outputs → JSON 整数
    "ECHOTEXT": "Hello"                        //   string_outputs → JSON 字符串
  },
  "tables": {                                  // 表输出：表名 → 行数组（字段统一字符串）
    "USERLIST": [
      { "USERNAME": "DEVELOPER", "FIRSTNAME": "Dev", "LASTNAME": "User" }
    ]
  },
  "structs": {                                 // 顶层结构体输出（struct_outputs 声明时出现）
    "ADDRESS": { "FIRSTNAME": "Dev", "LASTNAME": "User" }
  },
  "return_table": [                            // 仅当 read_return=true 且存在 RETURN 表时
    { "TYPE": "S", "ID": "01", "NUMBER": "123", "MESSAGE": "..." }
  ]
}
```

> ⚠️ **当前版本的限制**：`table_outputs` 与 `struct_outputs` 的字段值统一按字符串读取，即使 SAP 侧类型是 INT/FLOAT/BCD/二进制。需要在调用方保留数值/二进制精度时，可改用 `BAPI_TRANSACTION_COMMIT` 等带结构化输入/输出的 BAPI，或自行用 SAP GUI/SE37 二次处理。

未读取的字段（如没传 `table_outputs`）在响应中对应键**不出现**（`tables` 为空对象）。

---

### 3.3 面向 AI 的元数据 API

5 个端点让 AI/Agent 自服务地发现函数、理解参数、查数据字典、读文档。典型工作流：**搜索 → 查接口 → 查文档 → 调用**。给 AI 的完整操作指南见 [`AGENTS.md`](./AGENTS.md)。

| 端点 | 用途 | 示例 |
|------|------|------|
| `POST /api/functions/search` | 按通配符搜索函数 | `{"pattern":"BAPI_USER_*","max_results":10}` |
| `GET /api/functions/:name` | 查函数完整接口（参数/类型/方向/嵌套字段） | `/api/functions/BAPI_USER_GET_DETAIL` |
| `GET /api/functions/:name/doc` | 查文档（短文本+SE37长文档+参数说明） | `/api/functions/BAPI_USER_GET_DETAIL/doc?lang=EN` |
| `GET /api/ddic/type/:name` | 查 DDIC 结构/表字段定义 | `/api/ddic/type/BAPIRET2` |
| `GET /api/ddic/field/:table/:field` | 查字段语义（数据元素/域/固定值） | `/api/ddic/field/BAPIRET2/TYPE` |

端到端示例（列出用户）：
```bash
# 1. 搜函数 → 2. 查接口(发现 EXPORT 表 USERLIST) → 3. 调用
curl http://127.0.0.1:3000/api/functions/BAPI_USER_GETLIST
curl -X POST http://127.0.0.1:3000/api/rfc -H "Content-Type: application/json" \
  -d '{"func_name":"BAPI_USER_GETLIST","table_outputs":{"USERLIST":[["USERNAME",12]]},"read_return":true}'
```

> **约束**
> - DDIC 类型查询(端点 4/5)对**结构**普遍可用；**透明表**(如 MARA)视目标系统 DDIC 配置可能 `NOT_FOUND`。
> - 长文档(端点 3)依赖 `DOCU_GET`，个别系统未启用时 `long_text` 为空，但参数描述仍可用。
> - `fixed_values` 对理解状态码/枚举字段的合法取值特别有用。

---


## 4. 调用示例

### 4.1 最小连通测试 — `STFC_CONNECTION`

SAP 标准 ping 函数，回显你发的文本。

```bash
curl -X POST http://127.0.0.1:3000/api/rfc \
  -H "Content-Type: application/json" \
  -d '{
    "func_name": "STFC_CONNECTION",
    "inputs": { "REQUTEXT": "Hello from Rust!" },
    "string_outputs": { "ECHOTEXT": 255, "RESPTEXT": 255 }
  }'
```

响应：

```json
{
  "func": "STFC_CONNECTION",
  "scalars": {
    "ECHOTEXT": "Hello from Rust!",
    "RESPTEXT": "SAP R/3 Rel. ..."
  },
  "tables": {}
}
```

### 4.2 读取用户列表 — `BAPI_USER_GETLIST`

```bash
curl -X POST http://127.0.0.1:3000/api/rfc \
  -H "Content-Type: application/json" \
  -d '{
    "func_name": "BAPI_USER_GETLIST",
    "inputs": { "MAX_ROWS": 50, "WITH_USERNAME": "X" },
    "int_outputs": ["ROWS"],
    "table_outputs": {
      "USERLIST": [
        {"name": "USERNAME", "max_len": 12},
        {"name": "FIRSTNAME", "max_len": 40},
        {"name": "LASTNAME",  "max_len": 40}
      ]
    },
    "read_return": true
  }'
```

> `max_len` 也可省略 → 服务端按 SAP 元数据自动发现。例如 `{"name": "USERNAME"}`。

### 4.3 带选择条件 — `BAPI_USER_GETLIST` + `SELECTION_RANGE`

`table_inputs` 演示：过滤用户名以 `D` 开头。

```bash
curl -X POST http://127.0.0.1:3000/api/rfc \
  -H "Content-Type: application/json" \
  -d '{
    "func_name": "BAPI_USER_GETLIST",
    "inputs": { "MAX_ROWS": 10, "WITH_USERNAME": "X" },
    "table_inputs": {
      "SELECTION_RANGE": [
        {
          "PARAMETER": "USERNAME",
          "SIGN": "I",
          "OPTION": "CP",
          "LOW": "D*"
        }
      ]
    },
    "int_outputs": ["ROWS"],
    "table_outputs": {
      "USERLIST": [
        {"name": "USERNAME",  "max_len": 12},
        {"name": "FIRSTNAME", "max_len": 40},
        {"name": "LASTNAME",  "max_len": 40}
      ]
    }
  }'
```

### 4.4 用其他语言调用

**Python (requests)**

```python
import requests
resp = requests.post("http://127.0.0.1:3000/api/rfc", json={
    "func_name": "STFC_CONNECTION",
    "inputs": {"REQUTEXT": "from python"},
    "string_outputs": {"ECHOTEXT": 255},
})
print(resp.json())
```

**Node.js (fetch)**

```js
const r = await fetch("http://127.0.0.1:3000/api/rfc", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    func_name: "STFC_CONNECTION",
    inputs: { REQUTEXT: "from node" },
    string_outputs: { ECHOTEXT: 255 },
  }),
});
console.log(await r.json());
```

---

## 5. 常见 BAPI 速查

> 下表帮你快速知道哪些字段名该填哪里。具体可用字段需查 SE37 / SAP 官方文档。

| BAPI | inputs | table_inputs | 输出 |
|---|---|---|---|
| `STFC_CONNECTION` | `REQUTEXT` | — | `ECHOTEXT` / `RESPTEXT` (string) |
| `BAPI_USER_GETLIST` | `MAX_ROWS`(int) / `WITH_USERNAME` | `SELECTION_RANGE` | `ROWS`(int) / `USERLIST`(table) |
| `BAPI_USER_GET_DETAIL` | `USERNAME` | — | `ADDRESS`(struct→string) / `RETURN`(table) |
| `BAPI_MATERIAL_GETLIST` | `MAXROWS`(int) | `MATNRSELECTION` | `MATNRLIST`(table) |

调用模式：

- **结构体输入**（如 `ADDRESS`）当前不支持嵌套结构，仅支持表行里的扁平字段。
- **`RETURN` 表**：大多数 BAPI 用 `BAPIRET2` 结构，开启 `read_return: true` 自动解析
  `TYPE` / `ID` / `NUMBER` / `MESSAGE` 四个字段。

---

## 6. 错误处理

### 6.1 HTTP 状态码

| 状态码 | 触发场景 |
|---|---|
| `200 OK` | 调用成功 |
| `400 Bad Request` | 请求体 JSON 不合法或字段类型不符（由 axum 自动返回） |
| `500 Internal Server Error` | SAP 调用失败（连接断、参数名错、ABAP 抛异常等） |

### 6.2 错误响应体

```json
{
  "error": {
    "code": 7,
    "key": "RFC_ABAP_MESSAGE",
    "message": "User DEVELOPER has no authorization..."
  }
}
```

| 字段 | 含义 |
|---|---|
| `code` | SAP NWRFC 返回码（数字）。常见：`1`=通信错误，`2`=系统失败，`5`=授权，`7`=ABAP 消息，`10`=内部错误 |
| `key` | SDK 错误 key 字符串，便于精确分类 |
| `message` | 人类可读错误描述 |

### 6.3 500 错误排查思路

1. **看 `key`**：`RFC_COMMUNICATION_FAILURE` 多半是网络/连接；`RFC_ABAP_EXCEPTION` 是 ABAP 抛错
2. **看 `code` 对照 SDK 头文件 `sapnwrfc.h` 中的 `RFC_RC` 枚举**
3. **本地复现**：把同一个请求体直接对 SAP 系统用 SE37 跑一遍，验证参数名/类型

---

## 7. 部署提示

> **快速部署**：项目提供 `docker-compose.yml`，配好 `.env`（含 `SAP_SDK_HOST_PATH`）后
> `docker compose up -d --build` 即可。详见 [Quick Start](#quick-start5-分钟跑起来)。

### 7.1 `sapnwrfc.dll` 找不到

运行期 Windows 必须能加载 `sapnwrfc.dll`。两种方式：

- **PATH**：把 `nwrfcsdk\lib` 加入系统 `PATH`
- **同目录**：把 `sapnwrfc.dll` 复制到 exe 同级目录

启动时若报「找不到 DLL入口」之类，多半是 PATH 问题。

### 7.2 启动失败：连接相关

```
配置加载失败: 缺少必填环境变量: SAP_ASHOST
```
→ `.env` 没配全，看 [§3](#3-配置)。

```
RFC调用错误(代码: 2)：...
```
连不上 SAP：检查 `ASHOST`/`SYSNR` 网络可达、账号密码、`CLIENT` 集团号。

### 7.3 服务化部署

- 用 systemd / NSSM / Windows Service 把二进制注册为开机自启服务
- 监听 `0.0.0.0:3000` 仅在内网；对外请加反向代理（Nginx）+ 鉴权 + HTTPS
- 建议加 `SAP_LISTEN_ADDR` 限制到内网网卡

### 7.4 信任边界

本服务**不做鉴权**。任何能访问监听端口的调用方都能用配置的 SAP 账号执行任意 RFC。
务必放在受控网络或加一层网关鉴权。

---

## 8. 架构与限制

### 8.1 模块结构

```
src/
├── main.rs         启动入口：.env → 建连 → tokio::main → 启动 axum
├── config.rs       从环境变量组装连接参数 + 监听地址
├── server.rs       axum Router + handler（spawn_blocking 执行 FFI）
├── api.rs          请求/响应 DTO（serde）
├── executor.rs     execute_collect：取函数→填参→invoke→收集结果
├── connection.rs   RfcConnection：建连/关闭/取函数（含 unsafe impl Send）
├── function.rs     RfcFunction / RfcTable / RfcRow：参数读写、表操作
├── error.rs        RfcError + IntoResponse（500 + JSON 错误体）
├── ffi.rs          底层 C FFI 绑定（sapnwrfc.dll 函数签名）
└── string_utils.rs UTF-8 ↔ UTF-16(SAP UC) 转换
```

### 8.2 并发模型

```
[HTTP 请求 N] ─▶ axum handler（async）
                  │
                  ├─▶ spawn_blocking ─▶ [连接池抢一个空闲连接] ─▶ RfcInvoke (FFI)
                  │                                                     │
                  └─◀── await JoinHandle ◀───────────────────────────────┘
```

- **为什么用 `spawn_blocking`**：SAP 调用是阻塞 FFI，直接在 tokio worker 上跑会卡住整个运行时
- **为什么需要 `unsafe impl Send`**：`RfcConnection` 持裸指针非 Send；在 `Mutex` 串行化保护下，NWRFC SDK 允许跨线程串行使用同一连接，故 sound
- **池大小**：默认 `SAP_POOL_SIZE=8`，可在 `.env` 调整。请求从池里抢空闲连接，未抢到则等待；连接失败时按需自动重连

### 8.3 升级路径（规划中，提示扩展点）

| 需求 | 状态 / 改造方向 |
|---|---|
| 鉴权 | axum 加 `tower-http` 中间件 + API Key 校验 |
| RFC 调用超时 | `tokio::time::timeout` 包 `spawn_blocking`，慢请求不挂死服务 |
| 数值/二进制类型输出表字段 | 当前表/结构体输出统一按 string 读；后续在 `table_outputs` 字段定义加类型标记，executor 按真实类型读（`get_int` / `get_float` / `get_xstring` 等已实现） |
| tRFC/qRFC | 暂不支持 |

---

## 9. Server 端模式（被 SAP 调用）

除 client 模式（HTTP→SAP）外，本服务还支持 **server 模式**：让 SAP 通过 RFC 回调本服务，转发到配置的 HTTP webhook，实现「SAP → HTTP」反向代理。适合让 ABAP 调用外部微服务、或把业务事件从 SAP 推送出去。

启用方式：`SAP_ROLE=server`，配合 `servers.toml` 配置 gateway/program_id/函数/webhook。

完整说明（工作原理、SM59 配置、webhook 协议、示例）见 **[`docs/SERVER_MODE.md`](./docs/SERVER_MODE.md)**。

---


## 10. 发布新版本（维护者）

1. 提交所有改动，本地构建确认通过：
   ```bash
   cargo test
   cargo build --release
   ```
2. 更新 `Cargo.toml` 里的 `version` 字段（如 `0.1.0` → `0.2.0`）
3. 打 tag 并推送，CI 自动构建 Linux/macOS/Windows 二进制并上传到 GitHub Release：
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```
4. Windows 二进制由 CI 自动产出（`.def` + `lib.exe` 生成 stub 导入库绕过 MSVC 链接限制），
   无需维护者手工构建。如需本地复现 CI 的 stub 链接，可在「x64 Native Tools Command Prompt」中：
   ```powershell
   lib /def:sapnwrfc.def /machine:x64 /out:nwrfcsdk\lib\windows-x86_64\sapnwrfc.lib
   cargo build --release
   ```

> CI 工作流：[`.github/workflows/release.yml`](./.github/workflows/release.yml)。改动 [build.rs](./build.rs) 让 `SAP_SDK_DIR` 环境变量可指向任意 SDK 安装目录，方便 Docker / CI / 自定义路径使用。

## License

本项目基于 [MIT License](LICENSE) 开源。

> **关于 SAP NWRFC SDK**：本项目链接了 SAP 私有 SDK（[`build.rs`](./build.rs)），其使用受你与 SAP 之间的协议约束。MIT 协议仅适用于本仓库源码，不延伸至 SDK 本身。
