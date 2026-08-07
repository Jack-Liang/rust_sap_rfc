#!/usr/bin/env bash
# 启动检查脚本：验证 Rust 工具链、SAP SDK、配置文件、端口，然后启动。
# 用法：./start.sh
# 适用于 Linux / macOS / Git Bash（Windows）。

set -e

# 颜色输出（非交互终端自动关色）
if [ -t 1 ]; then
    GREEN='\033[0;32m'; YELLOW='\033[0;33m'; RED='\033[0;31m'; NC='\033[0m'
else
    GREEN=''; YELLOW=''; RED=''; NC=''
fi

ok()   { printf "${GREEN}✓${NC} %s\n" "$1"; }
warn() { printf "${YELLOW}!${NC} %s\n" "$1"; }
err()  { printf "${RED}✗${NC} %s\n" "$1"; }

echo "=== rust-sap-rfc 启动检查 ==="
echo ""

# 1. Rust 工具链
if command -v cargo >/dev/null 2>&1; then
    ok "Rust 已安装: $(cargo --version)"
else
    err "未检测到 Rust 工具链"
    echo ""
    echo "请安装 Rust："
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "  Windows 也可用：winget install Rustlang.Rustup"
    echo ""
    echo "或直接下载预编译二进制：见 README §「下载预编译二进制」"
    exit 1
fi
echo ""

# 2. 检测当前平台对应的 SDK 子目录
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
case "$ARCH" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
esac
case "$OS" in
    mingw*|msys*|cygwin*) OS="windows" ;;
    darwin) OS="darwin" ;;
    linux) OS="linux" ;;
esac
SDK_SUBDIR="nwrfcsdk/lib/${OS}-${ARCH}"

echo "检测到平台: ${OS}-${ARCH}"

# 3. SAP SDK 检查（三选一即可通过）
SDK_OK=0

# 3a. 优先：环境变量 SAP_SDK_DIR（build.rs 也识别这个变量）
if [ -n "$SAP_SDK_DIR" ]; then
    if [ -d "$SAP_SDK_DIR/lib/${OS}-${ARCH}" ] && \
       [ -n "$(ls "$SAP_SDK_DIR/lib/${OS}-${ARCH}"/*.so "$SAP_SDK_DIR/lib/${OS}-${ARCH}"/*.dylib "$SAP_SDK_DIR/lib/${OS}-${ARCH}"/*.dll 2>/dev/null | head -1)" ]; then
        ok "SAP SDK 已就位（环境变量 SAP_SDK_DIR）: $SAP_SDK_DIR/lib/${OS}-${ARCH}"
        SDK_OK=1
    else
        warn "环境变量 SAP_SDK_DIR='$SAP_SDK_DIR' 已设置但其下 lib/${OS}-${ARCH}/ 缺少库文件，忽略"
    fi
fi

# 3b. 默认：nwrfcsdk/lib/<os>-<arch>/ 已有库文件
if [ "$SDK_OK" = "0" ] && [ -d "$SDK_SUBDIR" ]; then
    LIB_COUNT=$(find "$SDK_SUBDIR" -maxdepth 1 \( -name "*.dll" -o -name "*.so" -o -name "*.dylib" \) 2>/dev/null | wc -l)
    if [ "$LIB_COUNT" -gt 0 ]; then
        ok "SAP SDK 已就位: $SDK_SUBDIR ($LIB_COUNT 个库文件)"
        SDK_OK=1
    fi
fi

# 3c. 自动解压 nwrfcsdk/lib/*/*.zip
#     用户的常见痛点：下载了 NWRFC SDK zip（如 nwrfcsdk-7.50.18-linux-x86_64.zip）
#     但不知道文件应该放哪。脚本自动识别 + 解压到正确目录。
if [ "$SDK_OK" = "0" ]; then
    ZIP_FILE=$(find nwrfcsdk/lib -maxdepth 2 -name "nwrfcsdk-*.zip" 2>/dev/null | head -1 || true)
    if [ -n "$ZIP_FILE" ]; then
        echo ""
        echo "发现 SDK 安装包: $ZIP_FILE"
        echo "自动解压到 nwrfcsdk/lib/${OS}-${ARCH}/ ..."
        EXTRACT_TMP=$(mktemp -d)
        if command -v unzip >/dev/null 2>&1; then
            unzip -q "$ZIP_FILE" -d "$EXTRACT_TMP"
        else
            err "需要 unzip 命令来解压 SDK（macOS 默认无，可 brew install unzip）"
            rm -rf "$EXTRACT_TMP"
            exit 1
        fi

        # SAP SDK zip 通常结构：nwrfcsdk/lib/<file>（无平台子目录）
        # 或者：nwrfcsdk/<os>-<arch>/lib/<file>
        # 把任何包含 .so/.dylib 的目录复制到目标平台子目录
        mkdir -p "$SDK_SUBDIR"
        SRC_LIB_DIR=$(find "$EXTRACT_TMP" -type d -name "lib" -path "*nwrfcsdk*" 2>/dev/null | head -1 || true)
        if [ -n "$SRC_LIB_DIR" ]; then
            cp -f "$SRC_LIB_DIR"/*.so "$SRC_LIB_DIR"/*.dylib "$SRC_LIB_DIR"/*.dll "$SDK_SUBDIR/" 2>/dev/null || true
        else
            # 兜底：扫描整个 zip 找库文件
            find "$EXTRACT_TMP" -type f \( -name "*.so" -o -name "*.dylib" -o -name "*.dll" \) \
                -exec cp -f {} "$SDK_SUBDIR/" \; 2>/dev/null || true
        fi
        rm -rf "$EXTRACT_TMP"

        # 验证
        LIB_COUNT=$(find "$SDK_SUBDIR" -maxdepth 1 \( -name "*.dll" -o -name "*.so" -o -name "*.dylib" \) 2>/dev/null | wc -l)
        if [ "$LIB_COUNT" -gt 0 ]; then
            ok "SAP SDK 解压完成: $SDK_SUBDIR ($LIB_COUNT 个库文件)"
            SDK_OK=1
        else
            err "解压后仍未发现库文件，请检查 zip 是否完整"
            exit 1
        fi
    fi
fi

# 3d. 全部失败：清晰指引
if [ "$SDK_OK" = "0" ]; then
    echo ""
    err "未找到 SAP NWRFC SDK 库文件"
    echo ""
    echo "三种方式任选其一："
    echo ""
    echo "  方式 A（推荐）：把 SAP 下载的 zip 放到 nwrfcsdk/lib/<任意>/ 目录下，"
    echo "                 本脚本会自动解压到正确路径。"
    echo ""
    echo "  方式 B：手动把库文件复制到 $SDK_SUBDIR/"
    echo "                 详见 nwrfcsdk/README.md §3。"
    echo ""
    echo "  方式 C：通过环境变量指向已安装路径："
    echo "                 export SAP_SDK_DIR=/path/to/nwrfcsdk"
    echo ""
    echo "获取 SDK：https://launchpad.support.sap.com （需 SAP 账号）"
    exit 1
fi
echo ""

# 4. .env 配置文件
if [ ! -f ".env" ]; then
    if [ -f ".env.example" ]; then
        warn "未找到 .env，已自动从 .env.example 复制（请编辑后再运行）"
        cp .env.example .env
        echo ""
        echo "请编辑 .env 填入 SAP 连接参数（SAP_ASHOST/SAP_USER/SAP_PASSWD 等），然后重新运行 ./start.sh"
        exit 0
    else
        err "未找到 .env 且无 .env.example 模板"
        exit 1
    fi
else
    ok ".env 配置文件存在"
    if grep -q "your-password-here" .env 2>/dev/null; then
        warn ".env 中仍含占位文本 'your-password-here'，请确认已填入真实 SAP 凭据"
    fi
fi
echo ""

# 5. 端口检查（默认 3000，可通过 SAP_LISTEN_ADDR 覆盖）
LISTEN_ADDR="${SAP_LISTEN_ADDR:-127.0.0.1:3000}"
LISTEN_PORT="${LISTEN_ADDR##*:}"
if command -v ss >/dev/null 2>&1 && ss -tln 2>/dev/null | grep -q ":$LISTEN_PORT\b"; then
    err "端口 $LISTEN_PORT 已被占用"
    echo ""
    echo "处理方式："
    echo "  - 设置 SAP_LISTEN_ADDR 改用其它端口，如："
    echo "      export SAP_LISTEN_ADDR=127.0.0.1:3001"
    echo "  - 或停止占用进程："
    if command -v lsof >/dev/null 2>&1; then
        lsof -i :"$LISTEN_PORT" 2>/dev/null | tail -n +2 || true
    elif command -v fuser >/dev/null 2>&1; then
        fuser "$LISTEN_PORT/tcp" 2>&1 || true
    fi
    exit 1
elif command -v netstat >/dev/null 2>&1 && netstat -tln 2>/dev/null | grep -q ":$LISTEN_PORT\b"; then
    err "端口 $LISTEN_PORT 已被占用（netstat 检测）"
    exit 1
else
    ok "端口 $LISTEN_PORT 可用"
fi
echo ""

# 6. 启动
echo "=== 检查通过，启动服务 ==="
echo ""
exec cargo run --release
