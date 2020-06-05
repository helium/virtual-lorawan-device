use tokio::prelude::*;
use std::net::SocketAddr;
mod udp_runtime;
use udp_runtime::UdpRuntime;
mod udp_radio;
use udp_radio::UdpRadio;
use semtech_udp;
use lorawan_device::{Device as LoRaWanDevice, Event as LoRaWanEvent, Response as LoRaWanResponse};
use rand::Rng;
use tokio::sync::mpsc;
use std::sync::Mutex;
use std::{time, thread};

static RANDOM: Option<Mutex<Vec<u32>>> = None;

// this is a workaround so that we can have a global function for random u32
fn get_random_u32() -> u32 {
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_addr = SocketAddr::from(([0, 0, 0, 0], 1324));
    let (mut receiver, mut udp_runtime_rx, mut sender, mut udp_runtime_tx) = UdpRuntime::new(socket_addr).await?;
    tokio::spawn(async move {
        udp_runtime_rx.run().await.unwrap();
    });

    tokio::spawn(async move {
        udp_runtime_tx.run().await.unwrap();
    });

    // this is a workaround so that we can have a global function for random u32
    thread::spawn(move || {
        let mut rng = rand::thread_rng();
        if let Some(mutex) = &RANDOM {
            let mut random = mutex.lock().unwrap();
            while random.len() < 32 {
                random.push(rng.gen())
            }
        }
        thread::sleep(time::Duration::from_millis(100));

    });

    let mut radio = UdpRadio::new(sender);
    let mut lorawan: LoRaWanDevice<UdpRadio, udp_runtime::RxMessage> = LoRaWanDevice::new(
        [0x83, 0x19, 0x20, 0xB5, 0x5C, 0x1E, 0x16, 0x7C],
        [0x11, 0x6B, 0x8A, 0x61, 0x3E, 0x37, 0xA1, 0x0C],
        [
            0xAC, 0xC3, 0x87, 0x2A, 0x2F, 0x82, 0xED, 0x20, 0x47, 0xED, 0x18, 0x92, 0xD6, 0xFC,
            0x8C, 0x0E,
        ],
        get_random_u32,
    );

    // this pushes received UDP packets into the radio
    tokio::spawn(async move {
        loop {
            if let Some(event) = receiver.recv().await {
                lorawan.handle_radio_event(&mut radio, event);
            }
        }
    });

    Ok(())
}
