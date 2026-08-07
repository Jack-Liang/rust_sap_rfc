# rust-sap-rfc

把 SAP NWRFC SDK 包装成常驻 HTTP 服务，提供 RESTful 接口调用任意 SAP RFC / BAPI。
其他服务无需安装 SAP SDK，发一个 JSON POST 即可调用。

- **技术栈**：Rust（标准库 FFI 直连 `sapnwrfc.dll`）+ axum + tokio + serde
- **零 SDK 依赖客户端**：调用方只要会发 HTTP POST
- **通用接口**：一个端点 `/api/rfc` 描述任意 BAPI，无需为每个 BAPI 写代码

---

## 目录

- [Quick Start（5 分钟跑起来）](#quick-start5-分钟跑起来)
- [1. 适用场景与限制](#1-适用场景与限制)
- [2. 准备工作](#2-准备工作)
- [3. 配置](#3-配置)
- [4. 启动](#4-启动)
- [5. API 参考](#5-api-参考)
- [6. 调用示例](#6-调用示例)
- [7. 常见 BAPI 速查](#7-常见-bapi-速查)
- [8. 错误处理](#8-错误处理)
- [9. 部署提示](#9-部署提示)
- [10. 架构与限制](#10-架构与限制)

---

## Quick Start（5 分钟跑起来）

> 唯一的硬前提：**SAP NWRFC SDK**（受版权限制不能随仓库分发，需从 [SAP Support Portal](https://launchpad.support.sap.com) 下载）。详见 [`nwrfcsdk/README.md`](./nwrfcsdk/README.md)。

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

---

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
| 标量输出 | ✅ `int_outputs`（整数）/ `string_outputs`（字符串）/ `auto_outputs`（按元数据真实类型自动读，含 float/bcd/二进制 Base64）|
| 表参数（TABLES） | ✅ 输入多行 + 输出遍历 |
| 顶层结构体参数 | ✅ `struct_inputs` / `struct_outputs`（如 BAPI_USER_CREATE.ADDRESS） |
| BCD 金额/数量 | ✅ 以字符串保留小数位语义 |
| 二进制（XSTRING/BYTE）| ✅ Base64 编码传输 |
| 元数据自动发现 | ✅ 字段长度+类型缓存，无需手填 max_len |
| Server 端（被 SAP 回调）| ❌ 暂不支持（需 SAP 侧 SM59 配合，单独立项） |
| tRFC/qRFC/bgRFC | ❌ 不支持 |
| SSO/SNC 安全登录 | ❌ 仅用户名密码 |

**其他限制**

| 项 | 说明 |
|---|---|
| 并发 | 全局单连接 + 互斥锁，请求**串行**执行。低并发够用；高并发需自行扩展为连接池 |
| 字符集 | 通过 UTF-16 桥接 SAP UC，UTF-8 输入输出 |
| 平台 | Windows/Linux/macOS × x86_64/aarch64（`build.rs` 自动选 SDK 子目录） |

---

## 2. 准备工作

### 2.1 SAP NWRFC SDK

本项目**不附带** SDK 二进制。请从 [SAP Support](https://launchpad.support.sap.com) 下载
`sapnwrfcsdk`（Windows x64），放到项目根目录或自定义路径：

```
rust_sap_rfc/
└── nwrfcsdk/
    └── lib/
        └── sapnwrfc.dll       ← 必须存在
```

`build.rs` 默认链接 `./nwrfcsdk/lib`，路径不同请改 `build.rs` 中的 `sdk_dir`。

### 2.2 Rust 工具链

```bash
# 安装 rustup（https://rustup.rs）后，stable 即可
rustc --version   # 建议MSVC toolchain，用于本地链接 SAP DLL
```

---

## 3. 配置

所有配置走环境变量，可写在项目根目录的 `.env` 文件里（已被 gitignore，不会提交）：

```bash
cp .env.example .env
```

`.env` 字段：

| 变量 | 必填 | 默认 | 说明 |
|---|:---:|---|---|
| `SAP_ASHOST` | ✅ | — | SAP 应用服务器主机名/IP |
| `SAP_SYSNR` | ✅ | — | 系统号，如 `00` |
| `SAP_CLIENT` | ✅ | — | 集团号，如 `001` |
| `SAP_USER` | ✅ | — | 登录账号 |
| `SAP_PASSWD` | ✅ | — | 登录密码 |
| `SAP_LANG` | ❌ | `EN` | 登录语言 |
| `SAP_LISTEN_ADDR` | ❌ | `127.0.0.1:3000` | HTTP 服务监听地址 |

> **生产部署提示**：不要把 `.env` 放进镜像层。用容器编排系统的密钥注入（K8s Secret / Docker Swarm secret）替代。

---

## 4. 启动

```bash
cargo run --release
```

成功启动会输出：

```
=== Rust SAP RFC -> REST 服务 ===
✅ SAP 系统连接成功
✅ HTTP 服务监听: http://127.0.0.1:3000
   - POST /api/rfc   通用 RFC 调用
   - GET  /health    健康检查
```

启动失败常见原因见 [§9 部署提示](#9-部署提示)。

---

## 5. API 参考

### 5.1 `GET /health`

健康检查，**不触碰 SAP**，用于探活。

```json
{ "status": "ok" }
```

---

### 5.2 `POST /api/rfc`

通用 RFC 调用。请求体描述要调哪个函数、传什么参数、要读哪些输出。

#### 请求体字段

| 字段 | 类型 | 必填 | 说明 |
|---|---|:---:|---|
| `func_name` | string | ✅ | SAP 函数模块名，如 `BAPI_USER_GETLIST` |
| `inputs` | object | ❌ | 标量输入参数：参数名 → 值（隐式：字符串→CHARS、整数→INT、浮点→FLOAT） |
| `table_inputs` | object | ❌ | 表输入参数：表名 → 行数组；每行是 字段名 → 值 |
| `struct_inputs` | object | ❌ | 顶层结构体输入：结构体名 → {字段名 → 值}（如 `ADDRESS`） |
| `int_outputs` | string[] | ❌ | 要读取的整型输出参数名列表 |
| `string_outputs` | object | ❌ | 要读取的字符串输出参数：参数名 → 最大长度 |
| `auto_outputs` | string[] | ❌ | 按**元数据真实类型**自动读的标量输出名（保留 float/bcd/二进制语义）|
| `table_outputs` | object | ❌ | 要读取的输出表：表名 → `[字段名, 最大长度]` 数组 |
| `struct_outputs` | object | ❌ | 要读取的顶层结构体输出：结构体名 → 字段列表 |
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
  "scalars": {                                 // 标量输出（按读取方式决定类型）
    "ROWS": 50,                                //   int_outputs → 数字
    "AMOUNT": 123.45,                          //   auto_outputs + FLOAT → 浮点
    "PRICE": "999.99",                         //   auto_outputs + BCD → 字符串保留小数
    "BINARY": "aGVsbG8="                       //   auto_outputs + XSTRING → Base64 字符串
  },
  "tables": {                                  // 表输出：表名 → 行数组（字段统一字符串）
    "USERLIST": [
      { "USERNAME": "DEVELOPER", "FIRSTNAME": "Dev", "LASTNAME": "User" }
    ]
  },
  "structs": {                                 // 顶层结构体输出（struct_outputs 声明时出现）
    "RETURN": { "TYPE": "S", "MESSAGE": "..." }
  },
  "return": [                                  // 仅当 read_return=true 且存在 RETURN 表时
    { "TYPE": "S", "ID": "01", "NUMBER": "123", "MESSAGE": "..." }
  ]
}
```

未读取的字段（如没传 `table_outputs`）在响应中对应键**不出现**（`tables` 为空对象）。

---

## 6. 调用示例

### 6.1 最小连通测试 — `STFC_CONNECTION`

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

### 6.2 读取用户列表 — `BAPI_USER_GETLIST`

```bash
curl -X POST http://127.0.0.1:3000/api/rfc \
  -H "Content-Type: application/json" \
  -d '{
    "func_name": "BAPI_USER_GETLIST",
    "inputs": { "MAX_ROWS": 50, "WITH_USERNAME": "X" },
    "int_outputs": ["ROWS"],
    "table_outputs": {
      "USERLIST": [
        ["USERNAME", 12],
        ["FIRSTNAME", 40],
        ["LASTNAME", 40]
      ]
    },
    "read_return": true
  }'
```

### 6.3 带选择条件 — `BAPI_USER_GETLIST` + `SELECTION_RANGE`

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
      "USERLIST": [["USERNAME", 12], ["FIRSTNAME", 40], ["LASTNAME", 40]]
    }
  }'
```

### 6.4 用其他语言调用

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

## 7. 常见 BAPI 速查

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

## 8. 错误处理

### 8.1 HTTP 状态码

| 状态码 | 触发场景 |
|---|---|
| `200 OK` | 调用成功 |
| `400 Bad Request` | 请求体 JSON 不合法或字段类型不符（由 axum 自动返回） |
| `500 Internal Server Error` | SAP 调用失败（连接断、参数名错、ABAP 抛异常等） |

### 8.2 错误响应体

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

### 8.3 500 错误排查思路

1. **看 `key`**：`RFC_COMMUNICATION_FAILURE` 多半是网络/连接；`RFC_ABAP_EXCEPTION` 是 ABAP 抛错
2. **看 `code` 对照 SDK 头文件 `sapnwrfc.h` 中的 `RFC_RC` 枚举**
3. **本地复现**：把同一个请求体直接对 SAP 系统用 SE37 跑一遍，验证参数名/类型

---

## 9. 部署提示

> **快速部署**：项目提供 `docker-compose.yml`，配好 `.env`（含 `SAP_SDK_HOST_PATH`）后
> `docker compose up -d --build` 即可。详见 [Quick Start](#quick-start5-分钟跑起来)。

### 9.1 `sapnwrfc.dll` 找不到

运行期 Windows 必须能加载 `sapnwrfc.dll`。两种方式：

- **PATH**：把 `nwrfcsdk\lib` 加入系统 `PATH`
- **同目录**：把 `sapnwrfc.dll` 复制到 exe 同级目录

启动时若报「找不到 DLL入口」之类，多半是 PATH 问题。

### 9.2 启动失败：连接相关

```
配置加载失败: 缺少必填环境变量: SAP_ASHOST
```
→ `.env` 没配全，看 [§3](#3-配置)。

```
RFC调用错误(代码: 2)：...
```
连不上 SAP：检查 `ASHOST`/`SYSNR` 网络可达、账号密码、`CLIENT` 集团号。

### 9.3 服务化部署

- 用 systemd / NSSM / Windows Service 把二进制注册为开机自启服务
- 监听 `0.0.0.0:3000` 仅在内网；对外请加反向代理（Nginx）+ 鉴权 + HTTPS
- 建议加 `SAP_LISTEN_ADDR` 限制到内网网卡

### 9.4 信任边界

本服务**不做鉴权**。任何能访问监听端口的调用方都能用配置的 SAP 账号执行任意 RFC。
务必放在受控网络或加一层网关鉴权。

---

## 10. 架构与限制

### 10.1 模块结构

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

### 10.2 并发模型

```
[HTTP 请求] ─▶ axum handler（async）
                │
                ├─▶ spawn_blocking ─▶ [Mutex 锁] ─▶ RfcInvoke (FFI)
                │                                    │
                └─◀── await JoinHandle ◀────────────┘
```

- **为什么用 `spawn_blocking`**：SAP 调用是阻塞 FFI，直接在 tokio worker 上跑会卡住整个运行时
- **为什么需要 `unsafe impl Send`**：`RfcConnection` 持裸指针非 Send；在 `Mutex` 串行化保护下，NWRFC SDK 允许跨线程串行使用同一连接，故 sound
- **代价**：所有请求共享一把锁 → 并发请求排队

### 10.3 升级路径（当前不支持，提示扩展点）

| 需求 | 改造方向 |
|---|---|
| 高并发 | `Arc<Mutex<Vec<RfcConnection>>>` 连接池，handler 抢空闲连接 |
| 数值类型输出表字段 | 在 `table_outputs` 字段定义里加类型标记，executor 按类型读 |
| 结构体输入 | `api.rs` 加 `struct_inputs`，executor 用 `RfcGetStructure` |
| 鉴权 | axum 加 `tower-http` 中间件 + API Key 校验 |
| 可观测性 | 引入 `tracing` + `tracing-subscriber`，替换 `println!` |

---

## License

私有项目，未指定开源协议。
