use std::net::SocketAddr;
mod udp_runtime;
use udp_runtime::UdpRuntime;
mod udp_radio;
use lorawan_device::{
    Device as LoRaWanDevice, Event as LoRaWanEvent, Request as LoRaWanRequest,
    Response as LoRaWanResponse, State as LoRaWanState,
};
use rand::Rng;
use std::sync::Mutex;
use std::{thread, time};
use udp_radio::UdpRadio;
use std::time::Duration;
use tokio::time::delay_for;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        RANDOM = Some(Mutex::new(Vec::new()));
    }

    // this is a workaround so that we can have a global function for random u32
    // it basically maintains 32 random u32's in a vector
    thread::spawn(move || {
        let mut rng = rand::thread_rng();
        unsafe {
            if let Some(mutex) = &RANDOM {
                let mut random = mutex.lock().unwrap();
                while random.len() < 32 {
                    random.push(rng.gen())
                }
            }
            thread::sleep(time::Duration::from_millis(100));
        }
    });
    delay_for(Duration::from_millis(500)).await;

    let my_address = SocketAddr::from(([0, 0, 0, 0], 1685));
    let host = SocketAddr::from(([127, 0, 0, 1], 1680));

    let (receiver, sender, udp_runtime) = UdpRuntime::new(my_address, host).await?;

    tokio::spawn(async move {
        udp_runtime.run().await.unwrap();
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


    lorawan_sender
        .try_send(udp_radio::Event::LoRaWAN(LoRaWanEvent::StartJoin))
        .unwrap();

    //tokio::spawn(async move {
    loop {
        if let Some(event) = lorawan_receiver.recv().await {
            println!("Event: {:?}", event);
            let response = match event {
                udp_radio::Event::Radio(radio_event) => {
                    lorawan.handle_radio_event(&mut radio, radio_event)
                }
                udp_radio::Event::LoRaWAN(lorawan_event) => {
                    lorawan.handle_event(&mut radio, lorawan_event)
                }
            };

            if let Some(response) = response {
                if let Some(request) = response.request() {
                    match request {
                        LoRaWanRequest::TimerRequest(_delay) => {
                            // for now we immediately fire timer
                            lorawan_sender
                                .send(udp_radio::Event::LoRaWAN(LoRaWanEvent::TimerFired))
                                .await?;
                        }
                        LoRaWanRequest::Error => {
                            panic!("LoRawAN Device Stack threw Error!");
                        }
                    }

                }

                match response.state() {
                    LoRaWanState::JoinedIdle => {
                        println!("Join");
                        let data = [1,2,3,4];
                        lorawan.send(&mut radio, &data, 1, false );
                    }
                    _ => (),
                }

            }
        }
    }
}
