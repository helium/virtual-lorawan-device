use lazy_static::lazy_static;
use log::{debug, error, info, warn};
use metrics::Metrics;
use std::{net::SocketAddr, path::PathBuf, str::FromStr, time::Instant};
use structopt::StructOpt;

mod error;
mod metrics;
mod settings;
mod virtual_device;

pub use error::{Error, Result};
pub use settings::Credentials;

use hyper::{
    service::{make_service_fn, service_fn},
    Server,
};

#[derive(Debug, StructOpt)]
#[structopt(name = "virtual-lorawan-device", about = "LoRaWAN test device utility")]
pub struct Opt {
    #[structopt(short, long, default_value = "./settings")]
    pub settings: PathBuf,
}

lazy_static! {
    static ref METRICS: Metrics = Metrics::new("1");
}

#[tokio::main]
async fn main() -> Result<()> {
    // Default log level to INFO unless environment override
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "INFO"),
    );
    let cli = Opt::from_args();
    let instant = Instant::now();
    let settings = settings::Settings::new(&cli.settings)?;
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
                error!("{} device threw error: {:?}", label, e)
            }
        });
    }

    // Start Prom Metrics Endpoint
    let addr = ([127, 0, 0, 1], 9898).into();
    println!("Listening on http://{}", addr);

    let serve_future = Server::bind(&addr).serve(make_service_fn(|_| async {
        Ok::<_, hyper::Error>(service_fn(Metrics::serve_req))
    }));

    tokio::spawn(async move {
        if let Err(e) = serve_future.await {
            error!("prometheus serv threw error: {:?}", e)
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("User exit via ctrl C");
    Ok(())
}
