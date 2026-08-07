#!/usr/bin/env bash
# 启动检查脚本：验证 Rust 工具链、SAP SDK、配置文件，然后 cargo run。
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
    exit 1
fi
echo ""

# 2. 检测当前平台对应的 SDK 子目录
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
# 归一化架构名
case "$ARCH" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
esac
# 归一化 OS 名（uname 在不同系统返回值不一）
case "$OS" in
    mingw*|msys*|cygwin*) OS="windows" ;;
    darwin) OS="darwin" ;;
    linux) OS="linux" ;;
esac
SDK_SUBDIR="nwrfcsdk/lib/${OS}-${ARCH}"

echo "检测到平台: ${OS}-${ARCH}"
if [ -d "$SDK_SUBDIR" ]; then
    # 统计目录内的库文件
    LIB_COUNT=$(find "$SDK_SUBDIR" -maxdepth 1 \( -name "*.dll" -o -name "*.so" -o -name "*.dylib" \) 2>/dev/null | wc -l)
    if [ "$LIB_COUNT" -gt 0 ]; then
        ok "SAP SDK 已就位: $SDK_SUBDIR ($LIB_COUNT 个库文件)"
    else
        err "SDK 目录存在但无库文件: $SDK_SUBDIR"
        echo ""
        echo "请把对应平台的 SAP NWRFC SDK 库（.dll/.so/.dylib）放入该目录。"
        echo "详见 nwrfcsdk/README.md。"
        exit 1
    fi
else
    err "未找到当前平台的 SDK 目录: $SDK_SUBDIR"
    echo ""
    echo "请创建该目录并放入 SAP NWRFC SDK 库文件。"
    echo "获取方式见 nwrfcsdk/README.md。"
    exit 1
fi
echo ""

# 3. .env 配置文件
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
    # 检查必填项是否仍是占位符
    if grep -q "your-password-here" .env 2>/dev/null; then
        warn ".env 中仍含占位文本 'your-password-here'，请确认已填入真实 SAP 凭据"
    fi
fi
echo ""

# 4. 启动
echo "=== 检查通过，启动服务 ==="
echo ""
exec cargo run --release
