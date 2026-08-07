mod api;
mod config;
mod connection;
mod error;
mod executor;
mod ffi;
mod function;
mod metadata;
mod pool;
mod server;
mod string_utils;

use crate::pool::RfcConnectionPool;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 初始化结构化日志（受 RUST_LOG 环境变量控制，默认 info）
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("=== Rust SAP RFC -> REST 服务启动 ===");

    // 2. 加载 .env（找不到文件不报错，可由真实环境变量替代）
    let dotenv_result = dotenvy::dotenv();
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            // 配置缺失时给完整引导，而非冷冰冰的单变量报错
            print_config_guide(&e, dotenv_result.is_err());
            return Err(e.into());
        }
    };
    tracing::info!(listen = cfg.listen_addr, "配置加载完成");

    // 3. 创建连接池：首次建连，后续失败自动重连
    let pool = RfcConnectionPool::new(cfg.conn_params)?;
    tracing::info!("SAP 系统连接成功");
    let shared: server::SharedPool = Arc::new(pool);

    // 4. 构造优雅停机信号：Ctrl+C 或 SIGTERM 触发
    let shutdown = async move {
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("安装 ctrl-c 信号处理器失败");
        };

        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("安装 SIGTERM 信号处理器失败")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => tracing::info!("收到 Ctrl+C 信号，开始优雅停机"),
            _ = terminate => tracing::info!("收到 SIGTERM 信号，开始优雅停机"),
        }
    };

    // 5. 启动 axum（with_graceful_shutdown 让在飞请求完成后再退出）
    server::run(shared, &cfg.listen_addr, shutdown).await?;
    tracing::info!("服务已停止");
    Ok(())
}

/// 配置缺失时的友好引导。检测「无 .env 文件」这一典型场景，给出针对性步骤。
/// `dotenv_not_found` 为 true 表示项目根目录没有 .env 文件。
fn print_config_guide(err: &str, dotenv_not_found: bool) {
    eprintln!();
    eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    eprintln!("  ❌ 配置加载失败: {}", err);
    eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    eprintln!();
    if dotenv_not_found {
        eprintln!("  未检测到 .env 文件。请按以下步骤操作：");
        eprintln!();
        eprintln!("  1. 复制配置模板：");
        eprintln!("       cp .env.example .env        (Linux/macOS)");
        eprintln!("       copy .env.example .env      (Windows CMD)");
        eprintln!("       Copy-Item .env.example .env (PowerShell)");
        eprintln!();
        eprintln!("  2. 编辑 .env，填入 SAP 连接参数：");
        eprintln!("       SAP_ASHOST=<SAP 应用服务器地址>");
        eprintln!("       SAP_SYSNR=00");
        eprintln!("       SAP_CLIENT=001");
        eprintln!("       SAP_USER=<你的账号>");
        eprintln!("       SAP_PASSWD=<你的密码>");
        eprintln!();
        eprintln!("  3. 重新运行：cargo run --release  （或 ./start.sh / start.ps1）");
    } else {
        eprintln!("  .env 文件已存在，但缺少必填项或值无效。");
        eprintln!("  请检查 .env 中的以下变量是否都已填写：");
        eprintln!("    SAP_ASHOST / SAP_SYSNR / SAP_CLIENT / SAP_USER / SAP_PASSWD");
        eprintln!("  完整字段说明见 README.md §3 配置。");
    }
    eprintln!();
}
