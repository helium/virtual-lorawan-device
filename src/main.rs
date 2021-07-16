use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Instant;

use structopt::StructOpt;

mod virtual_device;

pub struct Credentials {
    deveui: [u8; 8],
    appeui: [u8; 8],
    appkey: [u8; 16],
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mac_address = [0, 0, 0, 0, 4, 3, 2, 1];
    let cli = Opt::from_args();
    let host = SocketAddr::from_str(cli.host.as_str())?;
    let instant = Instant::now();

    let lorawan_app = virtual_device::VirtualDevice::new(
        instant,
        host,
        mac_address,
        Credentials {
            deveui: [0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8],
            appeui: [0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8],
            appkey: [
                0xFB, 0x4B, 0x19, 0xE9, 0xF8, 0xD4, 0xB1, 0x50, 0x35, 0x76, 0xCE, 0x9B, 0xD8, 0x79,
                0x9C, 0xD3,
            ],
        },
    )
    .await;

    lorawan_app.run().await
}

#[derive(Debug, StructOpt)]
#[structopt(name = "virtual-lorawan-device", about = "LoRaWAN test device utility")]
pub struct Opt {
    #[structopt(short, long, default_value = "127.0.0.1:1680")]
    pub host: String,
}
