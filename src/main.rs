use log::{debug, error, info, warn};
use metrics::Metrics;
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, str::FromStr, time::Instant};
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

#[tokio::main]
async fn main() -> Result<()> {
    // Default log level to INFO unless environment override
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "INFO"),
    );
    let metrics = metrics::Metrics::init();

    let cli = Opt::from_args();
    let instant = Instant::now();
    let settings = settings::Settings::new(&cli.settings)?;
    let host = SocketAddr::from_str(settings.host.as_str())?;
    // Start Prom Metrics Endpoint
    let addr = ([127, 0, 0, 1], 9898).into();
    let mut metrics_sender: HashMap<String, metrics::Sender> = HashMap::new();
    for oui in settings.ouis {
        let sender = metrics.run(&oui);
        metrics_sender.insert(oui, sender);
    }
    println!("Listening on http://{}", addr);

    let serve_future = Server::bind(&addr).serve(make_service_fn(|_| async {
        Ok::<_, hyper::Error>(service_fn(Metrics::serve_req))
    }));

    tokio::spawn(async move {
        if let Err(e) = serve_future.await {
            error!("prometheus serv threw error: {:?}", e)
        }
    });

    for (label, device) in settings.devices {
        let oui = if let Some(oui) = &device.oui {
            oui
        } else {
            &settings.default_oui
        };
        let metrics_sender = if let Some(sender) = metrics_sender.get(oui) {
            sender.clone()
        } else {
            panic!("Device is set for invalid OUI: {}", oui);
        };

        let lorawan_app = virtual_device::VirtualDevice::new(
            instant,
            host,
            device.mac_cloned_into_buf()?,
            device.credentials,
            metrics_sender,
        )
        .await?;

        tokio::spawn(async move {
            if let Err(e) = lorawan_app.run().await {
                error!("{} device threw error: {:?}", label, e)
            }
        });
    }

    tokio::signal::ctrl_c().await?;
    info!("User exit via ctrl C");
    Ok(())
}
