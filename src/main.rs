#[macro_use]
extern crate lazy_static;

mod udp_runtime;
use udp_runtime::UdpRuntime;
mod udp_radio;
use udp_radio::UdpRadio;
mod cli;
use cli::*;
mod config;

use {
    lorawan_device::Device as LoRaWanDevice,
    rand::Rng,
    structopt::StructOpt,
    std::{
        net::SocketAddr,
        process,
        sync::Mutex,
        time::{self, Duration, Instant},
        thread
    },
    tokio::time::delay_for,
};


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

lazy_static! {
    static ref INSTANT: Instant = Instant::now();
}

use std::str::FromStr;
async fn run<'a>(opt: Opt) -> Result<(), Box<dyn std::error::Error>> {
    let config = config::load_config(&opt.device_file)?;

    let devices = config.devices;

    println!("Insantiating {} virtual devices", devices.len());
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

    let udp_runtime = UdpRuntime::new(my_address, host).await?;

    for device in devices {
        // UdpRadio implements the LoRaWAN device Radio trait
        // use it by sending requested via the lorawan_sender
        let (lorawan_receiver, mut radio_runtime, lorawan_sender, radio) =
            UdpRadio::new(udp_runtime.publish_to(), udp_runtime.subscribe(), INSTANT.clone());

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

    udp_runtime.run().await.unwrap();
    Ok(())
}