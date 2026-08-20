# 启动检查脚本（Windows PowerShell）：验证 Rust 工具链、SAP SDK、配置文件、端口，然后 cargo run。
# 用法：powershell -File start.ps1   或在资源管理器双击（需先 Set-ExecutionPolicy）

$ErrorActionPreference = "Stop"

function Write-Ok($msg)   { Write-Host "✓ $msg" -ForegroundColor Green }
function Write-Warn2($msg){ Write-Host "! $msg" -ForegroundColor Yellow }
function Write-Err2($msg) { Write-Host "✗ $msg" -ForegroundColor Red }

Write-Host "=== sap-for-agents 启动检查 ===" -ForegroundColor Cyan
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
    Write-Host ""
    Write-Host "或直接下载预编译二进制：见 README §「下载预编译二进制」"
    exit 1
}
Write-Host ""

# 2. 检测当前平台对应的 SDK 子目录
# Windows 上默认就是 windows-x86_64（ARM 设备暂不支持 .dll，遇特殊情况手动调整）
$os = "windows"
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "aarch64" } else { "x86_64" }
$sdkSubdir = "nwrfcsdk\lib\$os-$arch"

Write-Host "检测到平台: $os-$arch"

# 3. SAP SDK 检查（优先级：SAP_SDK_DIR 环境变量 > 已部署库 > 自动解压 zip）
$script:SDK_OK = $false

# 3a. 优先：环境变量 SAP_SDK_DIR
if ($env:SAP_SDK_DIR) {
    $envSdkLib = Join-Path $env:SAP_SDK_DIR "lib\$os-$arch"
    if (Test-Path $envSdkLib) {
        $hasLib = Get-ChildItem -Path $envSdkLib -File -ErrorAction SilentlyContinue | Where-Object {
            $_.Extension -in @(".dll", ".so", ".dylib")
        }
        if ($hasLib.Count -gt 0) {
            Write-Ok "SAP SDK 已就位（环境变量 SAP_SDK_DIR）: $envSdkLib"
            $script:SDK_OK = $true
        } else {
            Write-Warn2 "环境变量 SAP_SDK_DIR='$env:SAP_SDK_DIR' 已设置但其下 lib/$os-$arch/ 缺少库文件，忽略"
        }
    }
}

# 3b. 默认：nwrfcsdk/lib/<os>-<arch>/ 已有库文件
if (-not $script:SDK_OK -and (Test-Path $sdkSubdir)) {
    $libFiles = Get-ChildItem -Path $sdkSubdir -File -ErrorAction SilentlyContinue | Where-Object {
        $_.Extension -in @(".dll", ".so", ".dylib")
    }
    if ($libFiles.Count -gt 0) {
        Write-Ok "SAP SDK 已就位: $sdkSubdir ($($libFiles.Count) 个库文件)"
        $script:SDK_OK = $true
    }
}

# 3c. 自动解压 nwrfcsdk/lib/*/*.zip
#     用户的常见痛点：下载了 NWRFC SDK zip 但不知道文件应该放哪。
if (-not $script:SDK_OK) {
    $zipFile = Get-ChildItem -Path "nwrfcsdk\lib" -Recurse -Filter "nwrfcsdk-*.zip" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($zipFile) {
        Write-Host ""
        Write-Host "发现 SDK 安装包: $($zipFile.FullName)"
        Write-Host "自动解压到 $sdkSubdir ..."
        $extractTmp = Join-Path $env:TEMP ("sdk-extract-" + [Guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Path $extractTmp -Force | Out-Null
        try {
            Expand-Archive -Path $zipFile.FullName -DestinationPath $extractTmp -Force
            # 找 zip 内含 .dll/.so/.dylib 的目录
            $srcLibDir = Get-ChildItem -Path $extractTmp -Recurse -Directory -Filter "lib" -ErrorAction SilentlyContinue | Where-Object {
                $_.FullName -match "nwrfcsdk"
            } | Select-Object -First 1
            New-Item -ItemType Directory -Path $sdkSubdir -Force | Out-Null
            if ($srcLibDir) {
                Get-ChildItem -Path $srcLibDir.FullName -File -ErrorAction SilentlyContinue | Where-Object {
                    $_.Extension -in @(".dll", ".so", ".dylib")
                } | ForEach-Object {
                    Copy-Item -Path $_.FullName -Destination $sdkSubdir -Force
                }
            } else {
                # 兜底：扫描整个 zip 找库文件
                Get-ChildItem -Path $extractTmp -Recurse -File -ErrorAction SilentlyContinue | Where-Object {
                    $_.Extension -in @(".dll", ".so", ".dylib")
                } | ForEach-Object {
                    Copy-Item -Path $_.FullName -Destination $sdkSubdir -Force
                }
            }
        } finally {
            Remove-Item -Path $extractTmp -Recurse -Force -ErrorAction SilentlyContinue
        }

        # 验证
        $libFiles = Get-ChildItem -Path $sdkSubdir -File -ErrorAction SilentlyContinue | Where-Object {
            $_.Extension -in @(".dll", ".so", ".dylib")
        }
        if ($libFiles.Count -gt 0) {
            Write-Ok "SAP SDK 解压完成: $sdkSubdir ($($libFiles.Count) 个库文件)"
            $script:SDK_OK = $true
        } else {
            Write-Err2 "解压后仍未发现库文件，请检查 zip 是否完整"
            exit 1
        }
    }
}

# 3d. 全部失败：清晰指引
if (-not $script:SDK_OK) {
    Write-Host ""
    Write-Err2 "未找到 SAP NWRFC SDK 库文件"
    Write-Host ""
    Write-Host "三种方式任选其一："
    Write-Host ""
    Write-Host "  方式 A（推荐）：把 SAP 下载的 zip 放到 nwrfcsdk\lib\<任意>\ 目录下，"
    Write-Host "                 本脚本会自动解压到正确路径。"
    Write-Host ""
    Write-Host "  方式 B：手动把库文件复制到 $sdkSubdir\"
    Write-Host "                 详见 nwrfcsdk\README.md §3。"
    Write-Host ""
    Write-Host "  方式 C：通过环境变量指向已安装路径："
    Write-Host "                 `$env:SAP_SDK_DIR = 'C:\path\to\nwrfcsdk'"
    Write-Host ""
    Write-Host "获取 SDK：https://launchpad.support.sap.com （需 SAP 账号）"
    exit 1
}
Write-Host ""

# 4. .env 配置文件
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

# 5. 端口检查（默认 3000，可通过 SAP_LISTEN_ADDR 覆盖）
$listenAddr = if ($env:SAP_LISTEN_ADDR) { $env:SAP_LISTEN_ADDR } else { "127.0.0.1:3000" }
$listenPort = [int]($listenAddr -split ":")[1]

# 用 Get-NetTCPConnection 检查端口占用
$portBusy = $false
try {
    $conn = Get-NetTCPConnection -LocalPort $listenPort -State Listen -ErrorAction SilentlyContinue
    if ($conn) {
        $portBusy = $true
    }
} catch {
    # Get-NetTCPConnection 在某些系统不可用，回退到 netstat
    $netstatOut = netstat -an 2>$null | Select-String ":$listenPort\s.*LISTENING"
    if ($netstatOut) {
        $portBusy = $true
    }
}
if ($portBusy) {
    Write-Err2 "端口 $listenPort 已被占用"
    Write-Host ""
    Write-Host "处理方式："
    Write-Host "  - 设置 SAP_LISTEN_ADDR 改用其它端口，如："
    Write-Host "      `$env:SAP_LISTEN_ADDR = '127.0.0.1:3001'"
    Write-Host "  - 或停止占用进程："
    Write-Host "      Get-Process -Id (Get-NetTCPConnection -LocalPort $listenPort -State Listen).OwningProcess"
    exit 1
} else {
    Write-Ok "端口 $listenPort 可用"
}
Write-Host ""

# 6. 启动
Write-Host "=== 检查通过，启动服务 ===" -ForegroundColor Cyan
Write-Host ""

# SAP SDK 的 ICU DLL 通过运行时动态加载，需把 SDK 目录加入 PATH。
# Windows DLL 搜索默认查 exe 同目录，但 SDK 在 nwrfcsdk\lib\windows-x86_64\ 子目录。
$sdkLibPath = (Resolve-Path "nwrfcsdk\lib\windows-x86_64").Path
$env:PATH = "$sdkLibPath;$env:PATH"

cargo run --release
