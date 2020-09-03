#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate prometheus;

mod device_loop;
mod udp_runtime;
use udp_runtime::UdpRuntime;
mod udp_radio;
use udp_radio::UdpRadio;
mod cli;
use cli::*;
mod config;
mod http;

mod prometheus_service;

use {
    lorawan_device::Device as LorawanDevice,
    lorawan_encoding::default_crypto::DefaultFactory as LorawanCrypto,
    prometheus_service::PrometheusBuilder,
    rand::Rng,
    std::{
        net::SocketAddr,
        process,
        str::FromStr,
        sync::Mutex,
        thread,
        time::{self, Duration, Instant},
    },
    structopt::StructOpt,
    tokio::time::delay_for,
};

// this is a workaround so that we can have a global function for random u32
fn get_random_u32() -> u32 {
    let random = &mut *RANDOM.lock().unwrap();
    if let Some(number) = random.pop() {
        number
    } else {
        panic!("Random queue empty!")
    }
}

#[macro_export]
macro_rules! debugln {
    ($fmt:expr) => {
        println!("{} | {: >8}] {}", chrono::Utc::now().format("[%F %H:%M:%S%.3f "), INSTANT.elapsed().as_millis(), $fmt);
    };
    ($fmt:expr, $($arg:tt)*) => {
        println!("{} | {: >8}] {}", chrono::Utc::now().format("[%F %H:%M:%S%.3f "), INSTANT.elapsed().as_millis(), format!($fmt, $($arg)*));
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Virtual LoRaWAN Device v{}", VERSION);

    let cli = Opt::from_args();
    let prometheus = if cli.prometheus {
        Some(PrometheusBuilder::new())
    } else {
        None
    };

    if let Err(e) = run(cli, prometheus).await {
        println!("error: {}", e);
        process::exit(1);
    }
    Ok(())
}

lazy_static! {
    static ref INSTANT: Instant = Instant::now();
    static ref RANDOM: Mutex<Vec<u32>> = Mutex::new(Vec::new());
}

const CONSOLE_CREDENTIALS_PATH: &str = "console-credentials.json";

async fn run<'a>(
    opt: Opt,
    mut prometheus: Option<PrometheusBuilder>,
) -> Result<(), Box<dyn std::error::Error>> {
    let http_uplink_sender = if let Some(port) = opt.http_port {
        let prom_sender = if let Some(prometheus) = &mut prometheus {
            Some(prometheus.get_sender())
        } else {
            None
        };

        let http_endpoint = http::Server::new(prom_sender).await;
        let ret = Some(http_endpoint.get_sender());
        tokio::spawn(async move {
            http_endpoint.run(port).await.unwrap();
        });
        ret
    } else {
        None
    };

    let devices = if let Some(cmd) = opt.command {
        let Command::Console { cmd } = cmd;
        let clients = config::load_console_client(CONSOLE_CREDENTIALS_PATH)?;
        let (oui, client) = match cmd {
            Console::Production {} => {
                if let Some(client) = clients.production {
                    (1, client)
                } else {
                    panic!("No credentials for production Console")
                }
            }
            Console::Staging {} => {
                if let Some(client) = clients.staging {
                    (2, client)
                } else {
                    panic!("No credentials for staging Console")
                }
            }
        };

        let console_devices = client.get_devices().await?;
        let mut devices = Vec::new();
        for console_device in console_devices {
            devices.push(config::Device::from_console_device(oui, console_device));
            if devices.len() == opt.max_devices {
                break;
            }
        }
        devices
    } else {
        let config = config::load_config(&opt.device_file)?;
        let mut devices = config.devices;
        devices.truncate(opt.max_devices);
        devices
    };

    println!("Instantiating {} virtual devices", devices.len());
    for device in &devices {
        println!("{},", serde_json::to_string(&device)?);
    }

    // this is a workaround so that we can have a global function for random u32
    // it basically maintains 32 random u32's in a vector
    thread::spawn(move || {
        let mut rng = rand::thread_rng();
        loop {
            {
                let random = &mut *RANDOM.lock().unwrap();
                while random.len() < 2056 {
                    random.push(rng.gen())
                }
            }
            // scope ensures that mutex is dropped during sleep
            thread::sleep(time::Duration::from_millis(1000));
        }
    });
    // the delay gives the random number generator get started
    delay_for(Duration::from_millis(50)).await;

    let my_address = SocketAddr::from(([0, 0, 0, 0], get_random_u32() as u16));
    let host = SocketAddr::from_str(opt.host.as_str())?;

    let udp_runtime = UdpRuntime::new(my_address, host).await?;

    let num_devices = devices.len();
    for device in devices {
        // UdpRadio implements the LoRaWAN device Radio trait
        // use it by sending requested via the lorawan_sender
        let (lorawan_receiver, mut radio_runtime, lorawan_sender, mut radio) = UdpRadio::new(
            udp_runtime.publish_to(),
            udp_runtime.subscribe(),
            *INSTANT,
            device.clone(),
        );

        // disable jitter by default if there is only one device
        if opt.disable_jitter || num_devices == 1 {
            radio.disable_jitter();
        }

        tokio::spawn(async move {
            radio_runtime.run().await.unwrap();
        });

        let lorawan: LorawanDevice<UdpRadio, LorawanCrypto> = LorawanDevice::new(
            radio,
            device.credentials().deveui_cloned_into_buf()?,
            device.credentials().appeui_cloned_into_buf()?,
            device.credentials().appkey_cloned_into_buf()?,
            get_random_u32,
        );

        let prom_sender = if let Some(prometheus) = &mut prometheus {
            prometheus.register(&device);
            Some(prometheus.get_sender())
        } else {
            None
        };

        // only give a sender to devices that have the HTTP integration
        let http = if device.has_http_integration() {
            if let Some(http_sender) = &http_uplink_sender {
                Some(http_sender.clone())
            } else {
                None
            }
        } else {
            None
        };

        tokio::spawn(async move {
            device_loop::run(lorawan_receiver, lorawan_sender, lorawan, prom_sender, http)
                .await
                .unwrap();
        });
    }

    if let Some(prometheus) = prometheus {
        let server = prometheus.build();

        tokio::spawn(async move {
            server.run().await.unwrap();
        });
    }

    udp_runtime.run().await.unwrap();

    loop {
        thread::sleep(time::Duration::from_millis(6_000_000))
    }
}
