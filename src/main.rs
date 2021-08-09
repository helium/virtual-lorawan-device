use log::{debug, error, info, warn};
use std::{net::SocketAddr, path::PathBuf, str::FromStr, time::Instant};
use structopt::StructOpt;
use lazy_static::lazy_static;
use metrics::Metrics;

mod error;
mod settings;
mod virtual_device;
mod metrics;

pub use error::{Error, Result};
pub use settings::Credentials;

use hyper::{
    header::CONTENT_TYPE,
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use prometheus::{Encoder, TextEncoder};

#[derive(Debug, StructOpt)]
#[structopt(name = "virtual-lorawan-device", about = "LoRaWAN test device utility")]
pub struct Opt {
    #[structopt(short, long, default_value = "./settings")]
    pub settings: PathBuf,
}

lazy_static! {
    static ref METRICS: Metrics = Metrics::new("1");
}

async fn serve_req(_req: Request<Body>) -> Result<Response<Body>> {
    let encoder = TextEncoder::new();

    let metric_families = prometheus::gather();
    let mut buffer = vec![];
    let mut buffer_print = vec![];
    encoder.encode(&metric_families, &mut buffer).unwrap();
    encoder.encode(&metric_families, &mut buffer_print).unwrap();

    // Output to the standard output.
    info!("{}", String::from_utf8(buffer_print).unwrap());

    let response = Response::builder()
        .status(200)
        .header(CONTENT_TYPE, encoder.format_type())
        .body(Body::from(buffer))
        .unwrap();

    Ok(response)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Default log level to INFO unless environment override
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "DEBUG"),
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

    let addr = ([127, 0, 0, 1], 9898).into();
    println!("Listening on http://{}", addr);

    let serve_future = Server::bind(&addr).serve(make_service_fn(|_| async {
        Ok::<_, hyper::Error>(service_fn(serve_req))
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
