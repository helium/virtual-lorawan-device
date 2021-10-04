use log::{debug, error, info, warn};
use metrics::Metrics;
use semtech_udp::client_runtime::UdpRuntime;
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, time::Instant};
use structopt::StructOpt;

mod error;
mod metrics;
mod settings;
mod virtual_device;

pub use error::{Error, Result};
pub use settings::{mac_string_into_buf, Credentials};

#[derive(Debug, StructOpt)]
#[structopt(name = "virtual-lorawan-device", about = "LoRaWAN test device utility")]
pub struct Opt {
    #[structopt(short, long, default_value = "./settings")]
    pub settings: PathBuf,
}

const DEFAULT_PF: &str = "default";

#[tokio::main]
async fn main() -> Result<()> {
    // Default log level to INFO unless environment override
    let mut log_builder = env_logger::Builder::from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "INFO"),
    );

    // Allow timestamps to be disabled
    let timestamps = std::env::var("VDEVICE_LOG_TIMESTAMP").unwrap_or_else(|_| "true".to_string());
    if timestamps != "true" {
        log_builder.format_timestamp(None).init();
    } else {
        log_builder.init();
    }

    let cli = Opt::from_args();
    let instant = Instant::now();
    let settings = settings::Settings::new(&cli.settings)?;
    let metrics = Metrics::run(([127, 0, 0, 1], 9898).into(), settings.get_servers());

    let pf_map = setup_packet_forwarders(settings.packet_forwarder).await?;

    for (label, device) in settings.device {
        let packet_forwarder = if let Some(pf) = &device.packet_forwarder {
            pf
        } else {
            DEFAULT_PF
        };

        let metrics_sender = metrics.get_server_sender(if let Some(server) = &device.server {
            server
        } else {
            &settings.default_server
        });

        let lorawan_app = virtual_device::VirtualDevice::new(
            label.clone(),
            instant,
            if let Some(pf) = pf_map.get(packet_forwarder) {
                pf
            } else {
                panic!("{} is invalid packet forwarder", packet_forwarder)
            },
            device.credentials,
            metrics_sender,
            device.rejoin_frames,
            device.region,
        )
        .await?;

        tokio::spawn(async move {
            if let Err(e) = lorawan_app.run().await {
                error!("{} device threw error: {:?}", label, e)
            }
        });
    }

    for (_, runtime) in pf_map {
        tokio::spawn(runtime.run());
    }

    tokio::signal::ctrl_c().await?;
    info!("User exit via ctrl C");
    Ok(())
}

async fn setup_packet_forwarders(
    mut packet_forwarder: HashMap<String, settings::PacketForwarder>,
) -> Result<HashMap<String, UdpRuntime>> {
    // prune the deafult packet forwarder if we have more than one
    if packet_forwarder.len() != 1 && packet_forwarder.contains_key("default") {
        packet_forwarder.remove("default");
    }

    let mut pf_map = HashMap::new();
    for (label, packet_forwarder) in packet_forwarder {
        let outbound = SocketAddr::from(([0, 0, 0, 0], 0));
        info!(
            "Creating packet forwarder {} connecting to {} from {}",
            label,
            packet_forwarder.host,
            outbound.to_string()
        );
        let udp_runtime = UdpRuntime::new(
            packet_forwarder.mac_cloned_into_buf().unwrap(),
            outbound,
            packet_forwarder.host,
        )
        .await?;
        pf_map.insert(label, udp_runtime);
    }

    Ok(pf_map)
}
