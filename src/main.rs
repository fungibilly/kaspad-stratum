mod kaspad;
mod stratum;

use anyhow::Result;
use clap::Parser;
use kaspad::{Client, Message};
use log::{debug, info, LevelFilter};

#[derive(Parser)]
struct Args {
    #[clap(short, long)]
    rpc_url: String,
    #[clap(short, long, default_value = "127.0.0.1:6969")]
    stratum_addr: String,
    #[clap(short, long)]
    mining_addr: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .filter_module("kaspad_stratum", LevelFilter::Debug)
        .init();

    let args = Args::parse();

    let stratum = stratum::Stratum::new(&args.stratum_addr).await?;

    let (client, mut msgs) = Client::new(&args.rpc_url, &args.mining_addr);
    while let Some(msg) = msgs.recv().await {
        match msg {
            Message::Info { version, .. } => {
                info!("Connected to Kaspad {version}");
            }
            Message::NewBlock => {
                debug!("New block, requesting template");
                if !client.request_template() {
                    debug!("Channel closed");
                    break;
                }
            }
            Message::Template(header) => {
                info!("Received block template");
                stratum.broadcast(header);
            }
        }
    }

    Ok(())
}
