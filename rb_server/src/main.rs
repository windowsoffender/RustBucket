mod certs;
mod config;
// mod context;
// mod handler;
mod server;
// mod server;

use clap::Parser;
use std::path::PathBuf;
use tokio::signal;

/// Operator server for RustBucket.
#[derive(Parser, Debug)]
#[command(name = "RustBucket Server", version, about)]
struct Args {
    /// Path to the config file (defaults to ./rb_server.toml if present)
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Host to bind the operator server to
    #[arg(long)]
    host: Option<String>,

    /// Port to bind the operator server to
    #[arg(short, long)]
    port: Option<u16>,

    /// Enable mutual TLS on the operator channel
    #[arg(long)]
    mtls: bool,

    /// Path to the CA certificate (written at startup)
    #[arg(long)]
    ca_path: Option<String>,

    /// Path to the client certificate (written at startup)
    #[arg(long)]
    cert_path: Option<String>,

    /// Path to the client key (written at startup)
    #[arg(long)]
    key_path: Option<String>,

    /// Path to the CRL (written at startup)
    #[arg(long)]
    crl_path: Option<String>,
}

#[tokio::main]
async fn main() {
    // Parse CLI arguments
    let args = Args::parse();

    // Initialize the logger
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .init()
        .unwrap();

    // Load the config file (if any), then apply CLI overrides.
    let mut conf = match config::RbServerConfig::load(args.config.as_deref()) {
        Ok(conf) => conf,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    };
    if let Some(host) = args.host {
        conf.host = host;
    }
    if let Some(port) = args.port {
        conf.port = port;
    }
    if args.mtls {
        conf.mtls.enabled = true;
    }
    if let Some(path) = args.ca_path {
        conf.mtls.ca_path = path;
    }
    if let Some(path) = args.cert_path {
        conf.mtls.cert_path = path;
    }
    if let Some(path) = args.key_path {
        conf.mtls.key_path = path;
    }
    if let Some(path) = args.crl_path {
        conf.mtls.crl_path = path;
    }

    log::info!(
        "Starting server on {}:{} (mTLS: {})",
        conf.host,
        conf.port,
        if conf.mtls.enabled { "on" } else { "off" }
    );

    // Create the C2 server and start it
    let c2 = server::RbServer::new(conf);

    match c2.start().await {
        Ok(_) => {
            log::info!("C2 server started successfully");
        }
        Err(err) => {
            log::error!("Error starting C2 server: {}", err);
            return;
        }
    }

    // Wait for Ctrl+C
    match signal::ctrl_c().await {
        Ok(()) => {
            log::info!("\nReceived Ctrl+C, shutting down gracefully...");
        }
        Err(err) => {
            log::error!("Error setting up Ctrl+C handler: {}", err);
        }
    }

    // Stop C2 server
    match c2.stop().await {
        Ok(_) => {
            log::info!("C2 server stopped successfully");
        }
        Err(err) => {
            log::error!("Error stopping C2 server: {}", err);
        }
    }

    log::info!("All services stopped successfully");
}
