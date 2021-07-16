use std::{net::SocketAddr, path::PathBuf, str::FromStr, time::Instant};
use structopt::StructOpt;

mod error;
mod settings;
mod virtual_device;

pub use error::Result;
pub use settings::Credentials;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Opt::from_args();
    let instant = Instant::now();
    let settings = settings::Settings::new(&cli.config)?;
    let host = SocketAddr::from_str(settings.host.as_str())?;

    for (label, device) in settings.devices {
        let lorawan_app = virtual_device::VirtualDevice::new(
            instant,
            host,
            device.mac_cloned_into_buf()?,
            device.credentials,
        )
        .await?;

        tokio::spawn(async move {
            if let Err(e) = lorawan_app.run().await {
                println!("{} device threw error: {:?}", label, e)
            }
        });
    }

    // Loop the main thread
    loop {
        use tokio::time::{sleep, Duration};
        sleep(Duration::from_secs(10)).await;
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "virtual-lorawan-device", about = "LoRaWAN test device utility")]
pub struct Opt {
    #[structopt(short, long, default_value = "./settings")]
    pub config: PathBuf,
}
