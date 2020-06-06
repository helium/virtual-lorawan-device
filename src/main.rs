use std::net::SocketAddr;
mod udp_runtime;
use udp_runtime::UdpRuntime;
mod udp_radio;
use lorawan_device::{Device as LoRaWanDevice, Event as LoRaWanEvent};
use rand::Rng;
use std::sync::Mutex;
use std::{thread, time};
use udp_radio::UdpRadio;

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

    let (mut receiver, mut udp_runtime_rx, sender, mut udp_runtime_tx) =
        UdpRuntime::new(socket_addr).await?;

    // udp_runtime_rx reads from the UDP port
    // and sends packets to the receiver channel
    tokio::spawn(async move {
        udp_runtime_rx.run().await.unwrap();
    });

    // udp_runtime_tx writes to the UDP port
    // by receiving packets from the sender channel
    tokio::spawn(async move {
        udp_runtime_tx.run().await.unwrap();
    });

    // this is a workaround so that we can have a global function for random u32
    // it basically maintains 32 random u32's in a vector
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

    // UdpRadio implements the LoRaWAN device Radio trait
    // it sends packets via the sender channel to the UDP runtime
    let (mut lorawan_receiver, mut radio_runtime, mut lorawan_sender, mut radio) =
        UdpRadio::new(sender, receiver);

    tokio::spawn(async move {
        radio_runtime.run().await.unwrap();
    });

    let mut lorawan: LoRaWanDevice<UdpRadio, udp_radio::RadioEvent> = LoRaWanDevice::new(
        [0x55, 0x6C, 0xB6, 0x1E, 0x37, 0xC5, 0x3C, 0x00],
        [0xB9, 0x94, 0x02, 0xD0, 0x7E, 0xD5, 0xB3, 0x70],
        [
            0xBF, 0x40, 0xD3, 0x0E, 0x4E, 0x23, 0x42, 0x8E, 0xF6, 0x82, 0xCA, 0x77, 0x64, 0xCD,
            0xB4, 0x23,
        ],
        get_random_u32,
    );

    tokio::spawn(async move {
        loop {
            if let Some(event) = lorawan_receiver.recv().await {
                let response = match event {
                    udp_radio::Event::Radio(radio_event) => {
                        lorawan.handle_radio_event(&mut radio, radio_event)
                    }
                    udp_radio::Event::LoRaWAN(lorawan_event) => {
                        lorawan.handle_event(&mut radio, lorawan_event)
                    }
                };
            }
        }
    });

    Ok(())
}
