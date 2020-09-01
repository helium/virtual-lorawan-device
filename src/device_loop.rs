#![macro_use]
use {
    super::{
        debugln, prometheus_service as prometheus,
        prometheus_service::Stat,
        udp_radio::{IntermediateEvent, UdpRadio},
        INSTANT,
    },
    lorawan_device::{
        self as lorawan, radio, Device as LorawanDevice, Event as LorawanEvent,
        Response as LorawanResponse,
    },
    lorawan_encoding::parser::FRMPayload,
    std::time::Duration,
    tokio::{
        sync::mpsc::{Receiver, Sender},
        time::delay_for,
    },
};

pub fn pretty_device(creds: &lorawan::Credentials) -> String {
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend(creds.deveui());
    bytes.reverse();
    let hex = hex::encode(&bytes);
    hex.to_uppercase()[12..].to_string()
}

pub async fn send_packet_or_new_join<C: lorawan_encoding::keys::CryptoFactory + Default>(
    lorawan_sender: &mut Sender<IntermediateEvent>,
    lorawan: &mut LorawanDevice<UdpRadio, C>,
    transmit_delay: u64,
    fcnt_before_rejoin: Option<usize>,
) {
    if let (Some(fcnt_threshold), Some(fcnt)) = (fcnt_before_rejoin, lorawan.get_fcnt_up()) {
        if fcnt >= fcnt_threshold as u32 {
            lorawan_sender
                .send(IntermediateEvent::NewSession)
                .await
                .unwrap();
            return;
        }
    }
    schedule_packet(lorawan_sender, lorawan, transmit_delay).await;
}

pub async fn schedule_packet<C: lorawan_encoding::keys::CryptoFactory + Default>(
    lorawan_sender: &mut Sender<IntermediateEvent>,
    lorawan: &mut LorawanDevice<UdpRadio, C>,
    transmit_delay: u64,
) {
    let delay = if lorawan.get_radio().jitter() {
        (super::get_random_u32() & 0x7F) as u64
    } else {
        0
    };

    let mut sender = lorawan_sender.clone();
    tokio::spawn(async move {
        delay_for(Duration::from_millis(transmit_delay as u64 + delay)).await;
        sender.send(IntermediateEvent::SendPacket).await.unwrap();
    });
}

pub async fn run<C: lorawan_encoding::keys::CryptoFactory + Default>(
    mut lorawan_receiver: Receiver<IntermediateEvent>,
    mut lorawan_sender: Sender<IntermediateEvent>,
    mut lorawan: LorawanDevice<UdpRadio, C>,
    mut prometheus: Option<Sender<prometheus::Message>>,
) -> Result<(), Box<dyn std::error::Error>> {
    lorawan_sender
        .try_send(IntermediateEvent::NewSession)
        .unwrap();

    loop {
        let device_ref = pretty_device(lorawan.get_credentials());
        let transmit_delay = lorawan.get_radio().config().transmit_delay();
        let fcnt_before_rejoin = lorawan.get_radio().config().fcnt_before_rejoin();

        if let Some(event) = lorawan_receiver.recv().await {
            let mut time = None;
            let (new_state, response) = match event {
                IntermediateEvent::NewSession => {
                    // if jitter is enabled, we'll delay 0-127 ms
                    let delay = if lorawan.get_radio().jitter() {
                        (super::get_random_u32() & 0x7F) as u64
                    } else {
                        0
                    };

                    delay_for(Duration::from_millis(1000 + delay as u64)).await;

                    debugln!("{}: Creating Session", device_ref);
                    let event = LorawanEvent::NewSessionRequest;
                    lorawan.handle_event(event)
                }
                IntermediateEvent::SendPacket => {
                    let data = [rand::random(), rand::random(), rand::random(), rand::random(), rand::random(), rand::random(), rand::random(), rand::random(), rand::random(), rand::random()];
                    let fcnt_up = lorawan.get_fcnt_up().unwrap();
                    debugln!("{}: Sending DataUp, FcntUp = {}", device_ref, fcnt_up);
                    lorawan.send(&data, 2, false)
                }
                IntermediateEvent::Rx(packet, time_received) => {
                    time = Some(time_received);
                    lorawan.handle_event(LorawanEvent::RadioEvent(radio::Event::PhyEvent(
                        packet.into(),
                    )))
                }
                IntermediateEvent::Timeout(id) => {
                    if lorawan.get_radio().most_recent_timeout(id) {
                        lorawan.handle_event(LorawanEvent::TimeoutFired)
                    } else {
                        (lorawan, Ok(LorawanResponse::NoUpdate))
                    }
                }
            };

            lorawan = new_state;
            let config = lorawan.get_radio().config().clone();
            match response {
                Ok(response) => match response {
                    LorawanResponse::TimeoutRequest(delay) => {
                        lorawan.get_radio().timer(delay).await;
                    }
                    LorawanResponse::NoJoinAccept => {
                        debugln!("{}: No JoinAccept Received", device_ref);
                        if let Some(ref mut sender) = prometheus {
                            sender
                                .send(prometheus::Message::Stat(config, Stat::JoinTimeout))
                                .await?
                        }
                        // if the Join Request failed try again
                        lorawan_sender
                            .send(IntermediateEvent::NewSession)
                            .await
                            .unwrap();
                    }
                    LorawanResponse::JoinSuccess => {
                        if let Some(t) = time {
                            debugln!(
                                "{}: JoinSuccess  [{} ms to spare] {:?}",
                                device_ref,
                                t,
                                lorawan.get_session_keys().unwrap()
                            );
                            if let Some(ref mut sender) = prometheus {
                                sender
                                    .send(prometheus::Message::Stat(config, Stat::JoinResponse(t)))
                                    .await?
                            }
                        }
                        schedule_packet(&mut lorawan_sender, &mut lorawan, transmit_delay).await;
                    }
                    LorawanResponse::NoUpdate => (),
                    LorawanResponse::NoAck => {
                        debugln!("{}: NoAck", device_ref);
                        if let Some(ref mut sender) = prometheus {
                            sender
                                .send(prometheus::Message::Stat(config, Stat::DownlinkTimeout))
                                .await?
                        }

                        send_packet_or_new_join(
                            &mut lorawan_sender,
                            &mut lorawan,
                            transmit_delay,
                            fcnt_before_rejoin,
                        )
                        .await;
                    }
                    LorawanResponse::ReadyToSend => {
                        debugln!(
                            "{}: No downlink received but none expected - ready to send again",
                            device_ref
                        );

                        send_packet_or_new_join(
                            &mut lorawan_sender,
                            &mut lorawan,
                            transmit_delay,
                            fcnt_before_rejoin,
                        )
                        .await;
                    }
                    LorawanResponse::DownlinkReceived(fcnt_down) => {
                        if let Some(t) = time {
                            debugln!(
                                "{}: Downlink received [{} ms to spare], FcntDown = {} ",
                                device_ref,
                                t,
                                fcnt_down
                            );

                            if let Some(downlink) = lorawan.take_data_downlink() {

                                if let Ok(FRMPayload::Data(data)) = downlink.frm_payload() {
                                    debugln!("Downlink Data Payload: {:?}", data);
                                }
                            }

                            if let Some(ref mut sender) = prometheus {
                                sender
                                    .send(prometheus::Message::Stat(
                                        config,
                                        Stat::DownlinkResponse(t),
                                    ))
                                    .await?
                            }
                        }

                        send_packet_or_new_join(
                            &mut lorawan_sender,
                            &mut lorawan,
                            transmit_delay,
                            fcnt_before_rejoin,
                        )
                        .await;
                    }
                    LorawanResponse::SessionExpired => {
                        lorawan_sender
                            .send(IntermediateEvent::NewSession)
                            .await
                            .unwrap();
                        // if let Some(t) = time {
                        //     debugln!(
                        //         "{}: Downlink received [{} ms to spare], FcntDown = {} ",
                        //         device_ref,
                        //         t,
                        //         fcnt_down
                        //     );
                        //
                        //     // if let Some(downlink) = lorawan.take_data_downlink() {
                        //     //     debugln!("DownlinkPayload: {:?}", downlink);
                        //     // }
                        //
                        //     if let Some(ref mut sender) = prometheus {
                        //         sender
                        //             .send(prometheus::Message::Stat(
                        //                 config,
                        //                 Stat::DownlinkResponse(t),
                        //             ))
                        //             .await?
                        //     }
                    }
                    LorawanResponse::JoinRequestSending => (),
                    LorawanResponse::UplinkSending(_) => (),
                },

                Err(err) => match err {
                    lorawan::Error::Radio(_) => (),
                    lorawan::Error::Session(e) => {
                        use lorawan::session::Error;
                        match e {
                            Error::RadioEventWhileIdle
                            | Error::RadioEventWhileWaitingForRxWindow => (),
                            _ => panic!("LoRaWAN Error Session {:?}\r\n", e),
                        }
                    }
                    lorawan::Error::NoSession(e) => {
                        use lorawan::no_session::Error;
                        match e {
                            Error::RadioEventWhileIdle
                            | Error::RadioEventWhileWaitingForJoinWindow => (),
                            _ => panic!("LoRaWAN Error NoSession {:?}\r\n", e),
                        }
                    }
                },
            }
        }
    }
}
