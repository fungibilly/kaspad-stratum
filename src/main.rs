mod kaspad;
mod stratum;

use crate::kaspad::KaspadHandle;
pub use crate::kaspad::U256;
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
    #[clap(short, long, default_value = "kaspad-stratum")]
    extra_data: String,
    #[clap(short, long)]
    mining_addr: String,
    #[clap(short, long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let level = if args.debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .filter_module("kaspad_stratum", level)
        .init();

    let (handle, recv_cmd) = KaspadHandle::new();
    let stratum = stratum::Stratum::new(&args.stratum_addr, handle.clone()).await?;

    let (client, mut msgs) = Client::new(
        &args.rpc_url,
        &args.mining_addr,
        &args.extra_data,
        handle,
        recv_cmd,
    );
    while let Some(msg) = msgs.recv().await {
        match msg {
            Message::Info { version, .. } => {
                info!("Connected to Kaspad {version}");
            }
            Message::NewTemplate => {
                debug!("Requesting new template");
                if !client.request_template() {
                    debug!("Channel closed");
                    break;
                }
            }
            Message::Template(template) => {
                debug!("Received block template");
                stratum.broadcast(template).await;
            }
        }
    }

    Ok(())
}
