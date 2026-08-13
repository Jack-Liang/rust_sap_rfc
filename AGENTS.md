# AGENTS.md

本文件指导 AI/Agent 如何使用 rust_sap_rfc 服务。

## 这是什么

rust_sap_rfc 是一个 **SAP NWRFC → REST 网关**：把 SAP 的 RFC/BAPI 函数暴露为 HTTP 接口。你（AI）通过它能在不安装 SAP 客户端的情况下，搜索、查询、调用 SAP 系统里的函数模块。

- **项目地址**：https://github.com/Jack-Liang/rust_sap_rfc
- **问题反馈**：https://github.com/Jack-Liang/rust_sap_rfc/issues

服务默认监听 `http://127.0.0.1:3000`（地址可由 `SAP_LISTEN_ADDR` 覆盖）。

## 认证（可选）

默认**免鉴权**（本机访问）。若部署方设置了环境变量 `SAP_API_KEY`，则所有 `/api/*` 端点要求请求头 `Authorization: Bearer <token>`：

```bash
curl -H "Authorization: Bearer <SAP_API_KEY>" http://127.0.0.1:3000/api/functions/BAPI_USER_GETLIST
```

- 未带 / 错 token → `401 {"code":401,"message":"..."}`。
- 探针 `/health`、`/ready` 与公开页 `/`、`/agents.md` **始终免鉴权**（不需要 token）。
- 是否启用由部署方决定。本机默认环境通常免鉴权——你可先不带 token 试，收到 401 再向部署方索取。

## 你能做什么

| 目标 | 用哪个端点 |
|------|-----------|
| 不知道有哪些函数 → 按名字模糊搜索 | `POST /api/functions/search` |
| 知道函数名，想知道参数怎么填 | `GET /api/functions/{name}` |
| 想读函数的完整文档（用途、示例） | `GET /api/functions/{name}/doc` |
| 想查某张表/结构有哪些字段 | `GET /api/ddic/type/{name}` |
| 想理解某个字段的含义、合法取值 | `GET /api/ddic/field/{table}/{field}` |
| 想看函数的 ABAP 源码（怎么实现的） | `GET /api/functions/{name}/source` |
| 想看程序/报表/include 的源码 | `GET /api/programs/{name}/source` |
| **实际调用一个 SAP 函数** | `POST /api/rfc` |

## 标准操作流程

绝大多数任务遵循 **搜索 → 查接口 → 查文档 → 调用** 四步：

```
1. 搜函数    POST /api/functions/search     找到目标函数名
2. 查接口    GET  /api/functions/{name}     看清楚参数名、类型、方向
3. 查文档    GET  /api/functions/{name}/doc 理解用途、约束、示例
4. 调用      POST /api/rfc                  按 interface 填参执行
```

> 不要跳过第 2 步直接调用——SAP 参数名区分大小写且必须大写，类型（CHAR/INT/BCD...）决定如何传值。先查接口能避免 90% 的传参错误。

## 端点速查（含可复制的示例）

### 1. 搜索函数

```bash
curl -X POST http://127.0.0.1:3000/api/functions/search \
  -H "Content-Type: application/json" \
  -d '{"pattern":"BAPI_USER_*","max_results":10}'
```

- `pattern`：函数名通配符，`*` 匹配任意。如 `BAPI_*`、`RFC_*`。
- 返回 `functions` 数组，每项含 `name` / `group` / `description`。
- 无匹配返回 `200 {"count":0,"functions":[]}`（**不是错误**）。

### 2. 查函数接口

```bash
curl http://127.0.0.1:3000/api/functions/BAPI_USER_GETLIST
```

返回该函数的**全部参数**，每个参数含：
- `name`：参数名（**传入时必须用这个原样大写名**）
- `type`：`CHAR` / `INT` / `STRUCTURE` / `TABLE` / `BCD` / `DATE` ...
- `direction`：`IMPORT`（你要填）/ `EXPORT`（返回值）/ `TABLES`（可进可出）
- `length`：字符长度（CHAR/NUM/DATE 等）
- `optional`：是否可省略
- `description`：参数说明
- `fields`：若为 STRUCTURE/TABLE，列出嵌套字段

### 3. 查函数文档

```bash
curl 'http://127.0.0.1:3000/api/functions/BAPI_USER_GETLIST/doc?lang=EN'
```

返回 `short_text`（短说明）、`long_text`（SE37 完整文档，可能很长）、`parameter_docs`（各参数描述）。`lang` 不传则用 `SAP_LANG` 环境变量（默认 EN）。

> 不是所有函数都有长文档。`long_text` 为空属正常，看 `parameter_docs` 即可。

### 4. 查 DDIC 表/结构字段

```bash
curl http://127.0.0.1:3000/api/ddic/type/BAPIRET2
```

返回该 DDIC 对象的全部字段定义。⚠️ 对**结构**（如 `BAPIRET2`）普遍可用；对**透明表**（如 `MARA`）取决于目标系统 DDIC 配置，部分系统会返回 `NOT_FOUND`。

### 5. 查字段语义（数据元素/域/合法取值）

```bash
curl 'http://127.0.0.1:3000/api/ddic/field/BAPIRET2/TYPE?lang=EN'
```

返回 `data_element`（数据元素）、`domain`（域）、`description`、`fixed_values`（域的固定值，对状态码/类型字段特别有用——告诉你这个字段能填哪些合法值）。

### 6. 调用 SAP 函数

```bash
curl -X POST http://127.0.0.1:3000/api/rfc \
  -H "Content-Type: application/json" \
  -d '{
    "func_name": "STFC_CONNECTION",
    "inputs": {"REQUTEXT": "hello"},
    "string_outputs": {"ECHOTEXT": 255, "RESPTEXT": 255}
  }'
```

请求体字段：
- `func_name`：**必填**，函数名（大写）
- `inputs`：IMPORT 标量参数 → 值。字符串直接传，整数直接传数字
- `table_inputs`：TABLES 输入参数 → 行数组（每行是 `{字段: 值}`）
- `struct_inputs`：顶层 IMPORT 结构体参数 → `{字段: 值}`
- `string_outputs`：要读的 EXPORT 字符串参数 → 最大长度（`null` 表示自动发现）
- `int_outputs`：要读的 EXPORT 整型参数名数组
- `auto_outputs`：要按元数据真实类型读的 EXPORT 标量参数名（INT→整数、FLOAT→浮点、INT8→i64、BCD→字符串、BYTE/XSTRING→Base64）
- `table_outputs`：要遍历的 EXPORT 表 → 字段列表。字段项 `{"name":"FIELD"}` 或 `{"name":"FIELD","max_len":12}`；加 `"auto":true` 让该字段按真实类型读（INT→整数、FLOAT→浮点、INT8→i64、BYTE/XSTRING→Base64、其余→字符串）
- `struct_outputs`：顶层结构体输出 → 字段列表（规则同 `table_outputs`）
- `read_return`：是否自动读 BAPI 的 RETURN 消息表
- `timeout_secs`：本次调用超时秒数（可选，≥1）。不传用全局默认 60s；慢接口（批量 BAPI、大表查询）可自主放宽。超时返回 504

响应体：
- `scalars`：标量输出（参数名 → 值，值类型由读取方式决定）
- `tables`：表输出（表名 → 行数组，每行 `{字段: 值}`）。默认字段值为字符串；`auto:true` 的字段按真实类型返回（整数/浮点/Base64 字符串）
- `structs`：顶层结构体输出（同 `tables` 的值类型规则）
- `return_table`：RETURN 消息（若有，字段统一为字符串）

> ⚠️ **表/结构输出默认按字符串读**。需要保留数值语义时，给字段加 `"auto":true`，服务端按 DDIC 真实类型（INT/FLOAT/INT8/BYTE）选择对应 getter。

## 关键约束（避坑）

1. **参数名必须大写**：SAP 参数名区分大小写，JSON 里永远用大写（如 `USERNAME` 不是 `username`）。
2. **先查接口再调用**：参数名/类型不要猜，先用端点 2 查准。
3. **CHAR 类型传字符串，INT 传数字**：`{"REQUTEXT":"hi"}`、`{"MAX_ROWS":100}`。
4. **BCD/INT8/二进制**用显式类型标记：`{"type":"BCD","value":"123.45"}`、`{"type":"BYTES","value":"<base64>"}`。
5. **BAPI 要显式提交事务**：写操作的 BAPI（CREATE/UPDATE/DELETE）成功后需调 `BAPI_TRANSACTION_COMMIT`，否则改动不生效。
6. **错误看 RETURN**：BAPI 通常不报 HTTP 错，而是返回 `RETURN` 表里带 `TYPE=E`（错误）的行。`read_return: true` 能自动带出。
7. **HTTP 状态码有语义**：4xx（400/401/403/404/405/429）多为调用方问题，5xx（500/502/504）多为 SAP 系统或网络问题。响应体 `error.code`=HTTP 状态码、`error.key`=机器码（如 `FU_NOT_FOUND`/`AUTH_INVALID`/`RATE_LIMITED`），按二者精确分支。
8. **透明表查询受限**：端点 4/5 对 DDIC 结构普遍可用，透明表（如 MARA）视系统配置可能 `NOT_FOUND`。
9. **调用有超时**：单次 SAP 调用默认 60s 超时（`SAP_REQUEST_TIMEOUT_SECS` 可配），超时返回 `504`。`/api/rfc` 可在请求体传 `timeout_secs` per-request 覆盖（慢接口如批量 BAPI、大表查询可放宽）。
10. **限流**：设了 `SAP_RATE_LIMIT_RPS` 时，`/api` 按调用方 IP 限速；超限返回 `429`（`key=RATE_LIMITED`）。默认不限流。

## 典型任务示例

**任务：列出 SAP 系统里的用户**

```bash
# 1. 搜相关函数
curl -X POST http://127.0.0.1:3000/api/functions/search \
  -H "Content-Type: application/json" \
  -d '{"pattern":"BAPI_USER_GETLIST"}'
# → 确认 BAPI_USER_GETLIST 存在

# 2. 查接口，看返回表叫什么、有哪些字段
curl http://127.0.0.1:3000/api/functions/BAPI_USER_GETLIST
# → 发现 EXPORT 表 USERLIST，含 USERNAME 等字段

# 3. 调用，读 USERLIST 表
curl -X POST http://127.0.0.1:3000/api/rfc \
  -H "Content-Type: application/json" \
  -d '{
    "func_name": "BAPI_USER_GETLIST",
    "table_outputs": {"USERLIST": [{"name": "USERNAME", "max_len": 12}, {"name": "FULLNAME", "max_len": 50}]},
    "read_return": true
  }'
```

## 健康检查

两个探针语义不同：

- `GET /health` —— liveness，**不触碰 SAP**，秒回 `{"status":"ok"}`，判断进程是否存活。
- `GET /ready` —— readiness，借连接池调 `RFC_PING`（5s 超时）验证 SAP 可达；成功 `{"status":"ready","sap":"ok"}`，失败/超时返回 `503`。
- `GET /metrics` —— Prometheus 指标（免鉴权）：连接池 idle/total/max、RFC 调用计数/耗时。供采集系统抓取。

```bash
curl http://127.0.0.1:3000/health
# → {"status":"ok"}   （不触碰 SAP，仅探活）

curl http://127.0.0.1:3000/ready
# → {"status":"ready","sap":"ok"}   （连 SAP 跑 RFC_PING；失败返回 503）
```
