#[macro_use]
extern crate lazy_static;
use std::net::SocketAddr;
mod udp_runtime;
use udp_runtime::UdpRuntime;
mod udp_radio;
use lorawan_device::{
    radio, Device as LoRaWanDevice, Event as LoRaWanEvent, Response as LoRaWanResponse,
    State as LoRaWanState,
};
use rand::Rng;
use std::process;
use std::sync::Mutex;
use std::time::Duration;
use std::{thread, time};
use structopt::StructOpt;
use tokio::time::delay_for;
use udp_radio::{UdpRadio, IntermediateEvent};
use semtech_udp::StringOrNum;
mod state_channels;
use state_channels::*;

const DEVICES_PATH: &str = "lorawan-devices.json";
mod config;
#[derive(Debug, StructOpt)]
#[structopt(name = "virtual-lorawan-device", about = "LoRaWAN test device utility")]
struct Opt {
    /// IP address and port of miner mirror port
    /// (eg: 192.168.1.30:1681)
    #[structopt(short, long, default_value = "127.0.0.1:1680")]
    host: String,

    /// Path to JSON devices file
    #[structopt(short, long, default_value = DEVICES_PATH)]
    console: String,

    /// Run State Channel test
    #[structopt(long)]
    sc_test: bool,
}

static mut RANDOM: Option<Mutex<Vec<u32>>> = None;

// this is a workaround so that we can have a global function for random u32
fn get_random_u32() -> u32 {
    //0xFFFF
    unsafe {
        if let Some(mutex) = &RANDOM {
            let mut random = mutex.lock().unwrap();
            if let Some(number) = random.pop() {
                number
            } else {
                panic!("Random queue empty!")
            }
        } else {
            panic!("Random queue not uninitialized!")
        }
    }
}

#[macro_export]
macro_rules! debugln {
    ($fmt:expr) => { print!("{} | {: >8}] ", chrono::Utc::now().format("[%F %H:%M:%S%.3f "), INSTANT.elapsed().as_millis());
        println!($fmt);
    };
    ($fmt:expr, $($arg:tt)*) => {
        print!("{} | {: >8}] ", chrono::Utc::now().format("[%F %H:%M:%S%.3f "), INSTANT.elapsed().as_millis());
        println!($fmt, $($arg)*);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Opt::from_args();
    if let Err(e) = run(cli).await {
        println!("error: {}", e);
        process::exit(1);
    }
    Ok(())
}
use std::time::Instant;



lazy_static! {
    static ref INSTANT: Instant = Instant::now();
}

use std::str::FromStr;
async fn run<'a>(opt: Opt) -> Result<(), Box<dyn std::error::Error>> {
    let config = config::load_config(DEVICES_PATH)?;

    let devices = config.devices;//[0].clone();

    if opt.sc_test && config.gateways.is_none() {
        panic!("Running State Channel test without gateways in config file isn't useful");
        // TODO: validate gateway fields
    }

    println!("Insantianting {} virtual devices", devices.len());
    for device in &devices {
        println!("{:?}", device);
    }

    unsafe {
        RANDOM = Some(Mutex::new(Vec::new()));
    }

    // this is a workaround so that we can have a global function for random u32
    // it basically maintains 32 random u32's in a vector
    thread::spawn(move || {
        let mut rng = rand::thread_rng();
        loop {
            unsafe {
                if let Some(mutex) = &RANDOM {
                    let mut random = mutex.lock().unwrap();
                    while random.len() < 32 {
                        random.push(rng.gen())
                    }
                }
                thread::sleep(time::Duration::from_millis(100));
            }
        }
    });
    // the delay gives the random number generator get started
    delay_for(Duration::from_millis(50)).await;

    let my_address = SocketAddr::from(([0, 0, 0, 0], 1687));
    let host = SocketAddr::from_str(opt.host.as_str())?;

    let (receiver, sender, udp_runtime) = UdpRuntime::new(my_address, host).await?;



    tokio::spawn(async move {
        udp_runtime.run().await.unwrap();
    });

    for device in devices {
        // UdpRadio implements the LoRaWAN device Radio trait
        // use it by sending requested via the lorawan_sender
        let (mut lorawan_receiver, mut radio_runtime, mut lorawan_sender, mut radio) =
            UdpRadio::new(sender.clone(), receiver, INSTANT.clone());

        tokio::spawn(async move {
            radio_runtime.run().await.unwrap();
        });

        let transmit_delay = device.transmit_delay();
        let lorawan = LoRaWanDevice::new(
            radio,
            device.credentials().deveui_cloned_into_buf()?,
            device.credentials().appeui_cloned_into_buf()?,
            device.credentials().appkey_cloned_into_buf()?,
            get_random_u32,
        );

        tokio::spawn(async move {
            udp_radio::run_loop(lorawan_receiver, lorawan_sender, lorawan, transmit_delay).await.unwrap();
        });
    }

    loop {}
}
