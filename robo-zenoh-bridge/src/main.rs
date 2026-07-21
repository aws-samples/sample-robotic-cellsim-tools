use std::process;

use clap::Parser;

use robo_zenoh_bridge::{run, Args};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(e) = run(Args::parse()) {
        eprintln!("ERROR: {}", e);
        process::exit(1);
    }
}
