mod certs;
mod config;
// mod context;
// mod handler;
mod server;
// mod server;

use clap::Parser;
use tokio::signal;

/// Operator server for RustBucket.
#[derive(Parser, Debug)]
#[command(name = "RustBucket Server", version, about)]
struct Args {
    /// Host to bind the operator server to
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Port to bind the operator server to
    #[arg(short, long, default_value_t = 6666)]
    port: u16,

    /// Enable mutual TLS on the operator channel
    #[arg(long, default_value_t = false)]
    mtls: bool,

    /// Path to the CA certificate (written at startup)
    #[arg(long, default_value = "certs/ca-cert.pem")]
    ca_path: String,

    /// Path to the client certificate (written at startup)
    #[arg(long, default_value = "certs/client-cert.pem")]
    cert_path: String,

    /// Path to the client key (written at startup)
    #[arg(long, default_value = "certs/client-key.pem")]
    key_path: String,

    /// Path to the CRL (written at startup)
    #[arg(long, default_value = "certs/crl.der")]
    crl_path: String,
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

    // Create mTLS configuration
    let mtls_config = config::MtlsConfig::new(
        args.mtls,
        args.ca_path,
        args.cert_path,
        args.key_path,
        args.crl_path,
        5, // CRL update interval in seconds
    );

    // Create server with mTLS configuration
    let conf = config::RbServerConfig::with_mtls(args.host, args.port, false, mtls_config);
    let c2 = server::RbServer::new(conf);

    // Start C2 server
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
