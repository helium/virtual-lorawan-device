use semtech_udp::client_runtime::UdpRuntime;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Instant;
use structopt::StructOpt;

mod udp_radio;
use udp_radio::UdpRadio;

use lorawan_device::{radio, region, Device, Event as LorawanEvent, Response as LorawanResponse};
use lorawan_encoding::default_crypto::DefaultFactory as LorawanCrypto;

#[derive(Debug)]
// I need some intermediate event because of Lifetimes
// maybe there's a cleaner way of doing this
pub enum IntermediateEvent {
    UdpRx(Box<semtech_udp::pull_resp::Packet>, u64),
    NewSession,
    Timeout(usize),
    SendPacket(Vec<u8>, u8, bool),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mac_address = [0, 0, 0, 0, 4, 3, 2, 1];
    let cli = Opt::from_args();
    let host = SocketAddr::from_str(cli.host.as_str())?;
    let instant = Instant::now();


    let (radio, mut lorawan_receiver, lorawan_sender) =    UdpRadio::new(instant, mac_address, host).await;

    // OTAA Credentials (DEVEUI/APPEUI = 0807060504030201)
    let deveui = [0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8];
    let appeui = [0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8];
    let appkey = [
        0xFB, 0x4B, 0x19, 0xE9, 0xF8, 0xD4, 0xB1, 0x50, 0x35, 0x76, 0xCE, 0x9B, 0xD8, 0x79,
        0x9C, 0xD3,
    ];

    let mut lorawan: Device<udp_radio::UdpRadio, LorawanCrypto> = Device::new(
        region::US915::subband(2).into(),
        radio,
        deveui,
        appeui,
        appkey,
        rand::random::<u32>,
    );

    lorawan_sender
        .send(IntermediateEvent::NewSession)
        .await
        .unwrap();

    loop {
        let event = lorawan_receiver
            .recv()
            .await
            .expect("Channel unexpectedly closed");
        let (new_state, response) = {
            match event {
                IntermediateEvent::NewSession => {
                    lorawan.handle_event(LorawanEvent::NewSessionRequest)
                }
                IntermediateEvent::Timeout(id) => {
                    if lorawan.get_radio().most_recent_timeout(id) {
                        lorawan.handle_event(LorawanEvent::TimeoutFired)
                    } else {
                        (lorawan, Ok(LorawanResponse::NoUpdate))
                    }
                }
                IntermediateEvent::SendPacket(data, fport, confirmed) => {
                    println!("Sending packet {}", data.len());
                    lorawan.send(&data, fport, confirmed)
                }
                IntermediateEvent::UdpRx(frame, _) => {
                    lorawan.handle_event(LorawanEvent::RadioEvent(radio::Event::PhyEvent(frame)))
                }
            }
        };
        lorawan = new_state;
        let send_uplink = {
            let mut send_uplink = false;
            match response {
                Ok(response) => match response {
                    LorawanResponse::TimeoutRequest(ms) => {
                        lorawan.get_radio().timer(ms).await;
                        println!("TimeoutRequest: {:?}", ms)
                    }
                    LorawanResponse::JoinSuccess => {
                        send_uplink = true;
                        println!("Join success")
                    }
                    LorawanResponse::ReadyToSend => {
                        send_uplink = true;
                        println!("Ready to send")
                    }
                    LorawanResponse::DownlinkReceived(fcnt_down) => {
                        send_uplink = true;
                        println!("Downlink received with FCnt = {}", fcnt_down)
                    }
                    LorawanResponse::NoAck => {
                        send_uplink = true;
                        println!("RxWindow expired, expected ACK to confirmed uplink not received\r\n")
                    }
                    LorawanResponse::NoJoinAccept => {
                        lorawan_sender
                            .send(IntermediateEvent::NewSession)
                            .await
                            .unwrap();

                        println!("No Join Accept Received")
                    }
                    LorawanResponse::SessionExpired => {
                        lorawan_sender
                            .send(IntermediateEvent::NewSession)
                            .await
                            .unwrap();

                        println!("SessionExpired. Created new Session")
                    }
                    LorawanResponse::NoUpdate => {
                        println!("NoUpdate")
                    }
                    LorawanResponse::UplinkSending(fcnt_up) => {
                        println!("Uplink with FCnt {}", fcnt_up)
                    }
                    LorawanResponse::JoinRequestSending => {
                        println!("Join Request Sending")
                    }
                },
                Err(err) => println!("Error {:?}", err),
            }
            send_uplink
        };
        if send_uplink {
            let mut fport = rand::random();
            while fport == 0 {
                fport = rand::random();
            }
            lorawan_sender
                .send(IntermediateEvent::SendPacket(vec![1, 2, 3, 4], fport, true))
                .await
                .unwrap();
        }
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "virtual-lorawan-device", about = "LoRaWAN test device utility")]
pub struct Opt {
    #[structopt(short, long, default_value = "127.0.0.1:1680")]
    pub host: String,
}
