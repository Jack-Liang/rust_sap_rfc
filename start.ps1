# 启动检查脚本（Windows PowerShell）：验证 Rust 工具链、SAP SDK、配置文件，然后 cargo run。
# 用法：powershell -File start.ps1   或在资源管理器双击（需先 Set-ExecutionPolicy）

$ErrorActionPreference = "Stop"

function Write-Ok($msg)   { Write-Host "✓ $msg" -ForegroundColor Green }
function Write-Warn2($msg){ Write-Host "! $msg" -ForegroundColor Yellow }
function Write-Err2($msg) { Write-Host "✗ $msg" -ForegroundColor Red }

Write-Host "=== rust-sap-rfc 启动检查 ===" -ForegroundColor Cyan
Write-Host ""

# 1. Rust 工具链
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargo) {
    Write-Ok "Rust 已安装: $(cargo --version)"
} else {
    Write-Err2 "未检测到 Rust 工具链"
    Write-Host ""
    Write-Host "请安装 Rust："
    Write-Host "  winget install Rustlang.Rustup"
    Write-Host "  或访问 https://rustup.rs"
    exit 1
}
Write-Host ""

# 2. 检测当前平台对应的 SDK 子目录
# Windows 上默认就是 windows-x86_64（ARM 设备暂不支持 .dll，遇特殊情况手动调整）
$os = "windows"
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "aarch64" } else { "x86_64" }
$sdkSubdir = "nwrfcsdk\lib\$os-$arch"

Write-Host "检测到平台: $os-$arch"
if (Test-Path $sdkSubdir) {
    $libFiles = Get-ChildItem -Path $sdkSubdir -File -ErrorAction SilentlyContinue | Where-Object {
        $_.Extension -in @(".dll", ".so", ".dylib")
    }
    if ($libFiles.Count -gt 0) {
        Write-Ok "SAP SDK 已就位: $sdkSubdir ($($libFiles.Count) 个库文件)"
    } else {
        Write-Err2 "SDK 目录存在但无库文件: $sdkSubdir"
        Write-Host ""
        Write-Host "请把 Windows 版 SAP NWRFC SDK 库（sapnwrfc.dll 等）放入该目录。"
        Write-Host "详见 nwrfcsdk\README.md。"
        exit 1
    }
} else {
    Write-Err2 "未找到当前平台的 SDK 目录: $sdkSubdir"
    Write-Host ""
    Write-Host "请创建该目录并放入 SAP NWRFC SDK 库文件。"
    Write-Host "获取方式见 nwrfcsdk\README.md。"
    exit 1
}
Write-Host ""

# 3. .env 配置文件
if (-not (Test-Path ".env")) {
    if (Test-Path ".env.example") {
        Write-Warn2 "未找到 .env，已自动从 .env.example 复制（请编辑后再运行）"
        Copy-Item .env.example .env
        Write-Host ""
        Write-Host "请编辑 .env 填入 SAP 连接参数（SAP_ASHOST/SAP_USER/SAP_PASSWD 等），然后重新运行 .\start.ps1" -ForegroundColor Yellow
        exit 0
    } else {
        Write-Err2 "未找到 .env 且无 .env.example 模板"
        exit 1
    }
} else {
    Write-Ok ".env 配置文件存在"
    if (Select-String -Path .env -Pattern "your-password-here" -Quiet) {
        Write-Warn2 ".env 中仍含占位文本 'your-password-here'，请确认已填入真实 SAP 凭据"
    }
}
Write-Host ""

# 4. 启动
Write-Host "=== 检查通过，启动服务 ===" -ForegroundColor Cyan
Write-Host ""
cargo run --release
