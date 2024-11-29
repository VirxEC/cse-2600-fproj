mod consts;
mod indexer;

use clap::Parser;
use std::fs::exists;
use tokio::fs::read_to_string;

#[derive(Parser)]
#[command(name = "cargo")]
#[command(bin_name = "cargo")]
struct Args {
    setup_index: bool,
    auto_download: bool,
}

/// example using: cargo r -r --setup-index upon first run
/// and then cargo r -r --auto-download after
#[tokio::main]
async fn main() {
    let mut args = Args::parse();

    if !args.setup_index && !args.auto_download {
        if exists("replays").unwrap() {
            args.auto_download = true;
        } else {
            args.setup_index = true;
        }
    }

    let token = read_to_string("token")
        .await
        .expect("Failed to read token file");
    let client = reqwest::Client::new();

    if args.setup_index {
        indexer::get_initial_index(client, &token).await;
    } else if args.auto_download {
        indexer::replay_auto_timer(client, &token).await;
    }
}
