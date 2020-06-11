use std::net::SocketAddr;
mod udp_runtime;
use udp_runtime::UdpRuntime;
mod udp_radio;
use lorawan_device::{
    Device as LoRaWanDevice, Event as LoRaWanEvent, Request as LoRaWanRequest,
    State as LoRaWanState,
};
use rand::Rng;
use std::process;
use std::sync::Mutex;
use std::time::Duration;
use std::{thread, time};
use structopt::StructOpt;
use tokio::time::delay_for;
use udp_radio::UdpRadio;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Opt::from_args();
    if let Err(e) = run(cli).await {
        println!("error: {}", e);
        process::exit(1);
    }
    Ok(())
}
use std::str::FromStr;
async fn run(opt: Opt) -> Result<(), Box<dyn std::error::Error>> {
    let config = config::load_config(DEVICES_PATH)?;

    let device = config.devices[0].clone();

    if opt.sc_test && config.gateways.is_none() {
        panic!("Running State Channel test without gateways in config file isn't useful");
        // TODO: validate gateway fields
    }

    println!("Virtual device utility only supports one device for now");
    println!("{:#?}", device);

    let oui = device.oui();
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

    let my_address = SocketAddr::from(([0, 0, 0, 0], 1686));
    let host = SocketAddr::from_str(opt.host.as_str())?;

    let (receiver, sender, udp_runtime) = UdpRuntime::new(my_address, host).await?;

    tokio::spawn(async move {
        udp_runtime.run().await.unwrap();
    });

    // UdpRadio implements the LoRaWAN device Radio trait
    // use it by sending requested via the lorawan_sender
    let (mut lorawan_receiver, mut radio_runtime, mut lorawan_sender, mut radio) =
        UdpRadio::new(sender, receiver);

    tokio::spawn(async move {
        radio_runtime.run().await.unwrap();
    });

    loop {
        let mut lorawan: LoRaWanDevice<UdpRadio, udp_radio::RadioEvent> = LoRaWanDevice::new(
            device.credentials().deveui_cloned_into_buf()?,
            device.credentials().appeui_cloned_into_buf()?,
            device.credentials().appkey_cloned_into_buf()?,
            get_random_u32,
        );

        let open = if opt.sc_test {
            let mut open = fetch_open_channel(oui).await?;
            println!("{:#?}", open);
            let mut remaining_blocks = open.remaining_blocks().await?.1;
            println!("Expires in {}", remaining_blocks);

            // if it is closing soon, just wait for this one to close
            if remaining_blocks < 3 {
                open.block_until_closed_transaction(oui).await.unwrap();
                open = fetch_open_channel(oui).await?;
                println!("{:#?}", open);
                remaining_blocks = open.remaining_blocks().await?.1;
                println!("Expires in {}", remaining_blocks);
            }

            let sender_clone = lorawan_sender.clone();
            let threshold = open.close_height() as isize - 3;
            tokio::spawn(async move {
                signal_at_block_height(threshold, sender_clone)
                    .await
                    .unwrap();
            });
            Some(open)
        } else {
            None
        };

        let mut joined = false;
        lorawan_sender
            .try_send(udp_radio::Event::LoRaWAN(LoRaWanEvent::StartJoin))
            .unwrap();

        // initialize as 1 to account for the join
        let mut sent_packets: usize = 1;

        loop {
            if let Some(event) = lorawan_receiver.recv().await {
                let response = match event {
                    udp_radio::Event::Radio(radio_event) => {
                        lorawan.handle_radio_event(&mut radio, radio_event)
                    }
                    udp_radio::Event::LoRaWAN(lorawan_event) => {
                        lorawan.handle_event(&mut radio, lorawan_event)
                    }
                    udp_radio::Event::Shutdown => {
                        break;
                    }
                };

                if let Some(response) = response {
                    if let (Some(request), state) = (response.request(), response.state()) {
                        match request {
                            LoRaWanRequest::TimerRequest(delay) => {
                                radio.timer_request(state, delay);
                                match state {
                                    LoRaWanState::WaitingForWindow => {
                                        // for now we immediately fire timer
                                        lorawan_sender
                                            .send(udp_radio::Event::LoRaWAN(
                                                LoRaWanEvent::TimerFired,
                                            ))
                                            .await?;
                                    }
                                    LoRaWanState::InWindow => {
                                        // never timeout
                                    }
                                    _ => panic!("Shouldn't be here"),
                                }
                            }
                            LoRaWanRequest::Error => {
                                panic!("LoRawAN Device Stack threw Error!");
                            }
                        }
                    }

                    if let LoRaWanState::JoinedIdle = response.state() {
                        let time_til_window = radio.time_until_window_ms();
                        if time_til_window > 0 {
                            println!(
                                "Packet received, but waiting for window: {} ms (time to spare)",
                                time_til_window
                            );
                            delay_for(Duration::from_millis(time_til_window as u64)).await;
                        } else {
                            println!(
                                "Warning! UDP packet received after first window by {} ms",
                                -time_til_window
                            );
                        }

                        if !joined {
                            println!("Join Success!");
                            joined = true;
                        }

                        let additional_delay = device.transmit_delay();
                        //println!("Additional delay: {} ms", additional_delay);
                        delay_for(Duration::from_millis(additional_delay)).await;
                        let data = [1, 2, 3, 4];
                        //println!("Sending DataUp");
                        lorawan.send(&mut radio, &data, 1, true);
                        sent_packets += 1;
                    }
                }
            }
        }

        if let Some(open) = open {
            println!("Stopped sending packets. Waiting for close transaction");
            if let Some(gateways) = &config.gateways {
                let closed = open.block_until_closed_transaction(oui).await.unwrap();

                for summary in closed.summaries() {
                    for gateway in gateways {
                        if summary.client().as_str() == gateway {
                            println!("{:?}", summary);
                        }
                    }
                }
            }
        }
        println!("Packets sent by device: {}", sent_packets);

        // drain the receiver before looping
        while lorawan_receiver.try_recv().is_ok() {}
    }
}
