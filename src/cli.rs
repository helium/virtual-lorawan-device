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

    /// Choose command to run
    #[structopt(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, StructOpt)]
pub enum Command {
    /// Use device credentials from Console
    Console {
        /// Maximum amount of devices to use
        #[structopt(short, long, default_value = "32")]
        max_devices: usize,

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
