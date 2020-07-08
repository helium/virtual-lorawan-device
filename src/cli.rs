use structopt::StructOpt;
const DEVICES_PATH: &str = "lorawan-devices.json";

#[derive(Debug, StructOpt)]
#[structopt(name = "virtual-lorawan-device", about = "LoRaWAN test device utility")]
pub struct Opt {
    /// IP address and port of miner mirror port
    /// (eg: 192.168.1.30:1681)
    #[structopt(short, long, default_value = "127.0.0.1:1680")]
    pub host: String,

    /// Path to JSON devices file
    #[structopt(short, long, default_value = DEVICES_PATH)]
    pub device_file: String,

    /// Disable jitter on transmits (is done by default for one device)
    #[structopt(long)]
    pub disable_jitter: bool,

    /// Maximum amount of devices to use
    #[structopt(short, long, default_value = "32")]
    pub max_devices: usize,

    /// Whether to put up a Prometheus service to be scraped
    #[structopt(short, long)]
    pub prometheus: bool,

    /// Choose command to run
    #[structopt(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, StructOpt)]
pub enum Command {
    /// Use device credentials from Console
    Console {
        /// Staging or console
        #[structopt(subcommand)]
        cmd: Console,
    },
}

#[derive(Debug, StructOpt)]
pub enum Console {
    /// Use device credentials from Console
    Staging {},
    Production {},
}
