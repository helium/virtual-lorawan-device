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
}
