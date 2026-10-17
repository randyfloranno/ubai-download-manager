//! `udm-testsrv` binary: runs the fault-injection HTTP(S) server on 127.0.0.1.

use clap::Parser;
use std::io::Write;
use std::process::ExitCode;
use udm_testsrv::TestServer;

#[derive(Parser, Debug)]
#[command(
    name = "udm-testsrv",
    about = "UDM local fault-injection test server (dev only)"
)]
struct Args {
    /// Port on 127.0.0.1 (0 = pick a free port and print it).
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// Serve HTTPS (self-signed certificate, ALPN h2 + http/1.1).
    #[arg(long)]
    tls: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let srv = match TestServer::bind(args.port, args.tls).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("udm-testsrv: cannot listen on 127.0.0.1:{}: {e}", args.port);
            return ExitCode::FAILURE;
        }
    };
    let mut out = std::io::stdout();
    let _ = writeln!(
        out,
        "udm-testsrv listening on {} (port {})",
        srv.base_url(),
        srv.port()
    );
    let _ = out.flush();
    let _ = tokio::signal::ctrl_c().await;
    srv.shutdown();
    ExitCode::SUCCESS
}
