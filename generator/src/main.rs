//! TCP client that connects to the message server and streams bytes from the generator.

use anyhow::Result;
use clap::Parser;
use generator::{Generator, GeneratorConfig};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "generator")]
struct Args {
    /// Server address (host:port)
    #[arg(short, long, default_value = "127.0.0.1:8080")]
    address: String,

    /// Message rate per second
    #[arg(short, long, default_value = "1000")]
    rate: usize,

    /// Probability of injecting an error (0.0..=1.0)
    #[arg(short, long, default_value = "0.001")]
    error_prob: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let config = GeneratorConfig {
        message_rate_per_sec: args.rate,
        error_probability: args.error_prob,
    };

    let (_gen, mut rx) = Generator::new(config);
    info!(address = %args.address, "connecting to server");
    let mut stream = TcpStream::connect(&args.address).await?;
    info!("connected, streaming data");

    while let Some(bytes) = rx.recv().await {
        stream.write_all(&bytes).await?;
    }

    info!("generator closed");
    Ok(())
}
