//! TCP server that accepts connections and parses the binary message stream.

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use dashmap::DashMap;
use parser::{ParseError, ParseResult, Parser as _, StateMachineParser};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{self, Duration};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "server")]
struct Args {
    /// Bind address (host:port)
    #[arg(short, long, default_value = "0.0.0.0:8080")]
    bind: String,

    /// Debug mode (print debug logs instead of the stats table)
    #[arg(short, long, default_value = "false")]
    debug: bool,

    /// Stats table refresh interval in seconds
    #[arg(short, long, default_value = "1")]
    stats_interval: u64,
}

#[derive(Debug)]
struct ClientStats {
    connected: bool,
    messages: usize,
    errors: usize,
    bytes: usize,
}

impl Default for ClientStats {
    fn default() -> Self {
        Self {
            connected: true,
            messages: 0,
            errors: 0,
            bytes: 0,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let listener = TcpListener::bind(&args.bind).await?;
    info!(bind = %args.bind, "listening");

    let stats = Arc::new(DashMap::<SocketAddr, ClientStats>::new());
    let stats_interval = args.stats_interval;

    // If not in debug mode, spawn a task to periodically print the stats table.
    if !args.debug {
        let stats = Arc::clone(&stats);
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(stats_interval));
            loop {
                interval.tick().await;

                // clear screen and home cursor
                print!("\x1b[2J\x1b[H");

                // print header
                println!(
                    "{:<22} | {:>10} | {:>8} | {:>12} | {:>10}",
                    "Client Address", "Messages", "Errors", "Bytes", "Connected?"
                );
                println!("{}", "-".repeat(74));

                // print stats for each client
                for r in stats.iter() {
                    let (addr, s) = (r.key(), r.value());
                    let conn = if s.connected { "X" } else { "" };
                    println!(
                        "{:<22} | {:>10} | {:>8} | {:>12} | {:>10}",
                        addr, s.messages, s.errors, s.bytes, conn
                    );
                }
            }
        });
    }

    // Accept connections and spawn a task to handle each client.
    loop {
        let (stream, addr) = listener.accept().await?;
        info!(%addr, "accepted connection");
        {
            stats.entry(addr).or_default();
        }

        tokio::spawn(handle_client(args.debug, stream, addr, stats.clone()));
    }
}

async fn handle_client(
    debug: bool,
    stream: TcpStream,
    addr: SocketAddr,
    stats: Arc<DashMap<SocketAddr, ClientStats>>,
) {
    let (mut reader, _writer) = stream.into_split();
    let mut buf = [0u8; 4096];
    let mut parser = StateMachineParser::new();

    loop {
        match reader.read(&mut buf).await {
            // connection closed
            Ok(0) => break,

            // read some bytes
            Ok(n) => {
                let chunk = &buf[..n];
                let results = parser.feed(chunk);

                if let Some(mut s) = stats.get_mut(&addr) {
                    s.bytes += n;

                    for r in results {
                        match r {
                            ParseResult::Complete(msg) => {
                                s.messages += 1;
                                if debug {
                                    debug!(%addr, ?msg, "message");
                                }
                            }
                            ParseResult::Error(e) => {
                                s.errors += 1;
                                if debug {
                                    match e {
                                        ParseError::ChecksumMismatch {
                                            expected,
                                            calculated,
                                        } => {
                                            warn!(%addr, expected, calculated, "checksum mismatch");
                                        }
                                        ParseError::InvalidEscapeSequence { offset } => {
                                            warn!(%addr, offset, "invalid escape sequence");
                                        }
                                        ParseError::Gap(n) => {
                                            warn!(%addr, n, "gap bytes");
                                        }
                                        ParseError::UnexpectedStartSequence { offset } => {
                                            warn!(%addr, offset, "unexpected start sequence");
                                        }
                                    }
                                }
                            }
                            ParseResult::Partial => {}
                        }
                    }
                }
            }

            // read error
            Err(e) => {
                warn!(%addr, error = %e, "read error");
                break;
            }
        }
    }

    {
        if let Some(mut s) = stats.get_mut(&addr) {
            s.connected = false;
        }
    }

    info!(%addr, "disconnected");
}
