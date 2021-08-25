use super::*;

use lorawan_device::{radio, region, Device, Event as LorawanEvent, Response as LorawanResponse};

use lorawan_encoding::default_crypto::DefaultFactory as LorawanCrypto;
use udp_radio::UdpRadio;
pub(crate) use udp_radio::{IntermediateEvent, Receiver, Sender};

mod udp_radio;

pub struct VirtualDevice<'a> {
    device: Device<UdpRadio<'a>, LorawanCrypto>,
    receiver: Receiver<IntermediateEvent>,
    sender: Sender<IntermediateEvent>,
}

impl<'a> VirtualDevice<'a> {
    pub async fn new(
        instant: Instant,
        host: SocketAddr,
        mac_address: [u8; 8],
        credentials: Credentials,
    ) -> Result<VirtualDevice<'a>> {
        let (radio, receiver, sender) = UdpRadio::new(instant, mac_address, host).await;
        let device: Device<udp_radio::UdpRadio, LorawanCrypto> = Device::new(
            region::US915::subband(2).into(),
            radio,
            credentials.deveui_cloned_into_buf()?,
            credentials.appeui_cloned_into_buf()?,
            credentials.appkey_cloned_into_buf()?,
            rand::random::<u32>,
        );

        Ok(VirtualDevice {
            device,
            receiver,
            sender,
        })
    }

    pub async fn run(mut self) -> Result<()> {
        // Kickstart "activity" by trying to join
        self.sender
            .send(IntermediateEvent::NewSession)
            .await
            .unwrap();

        let mut time_remaining = None;
        let mut lorawan = self.device;
        loop {
            let event = self
                .receiver
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
                        info!("Sending packet on fport {}", fport);
                        lorawan.send(&data, fport, confirmed)
                    }
                    IntermediateEvent::UdpRx(frame, time_received) => {
                        time_remaining = match frame.data.txpk.tmst {
                            semtech_udp::StringOrNum::N(tmst) => {
                                Some(tmst as i64 - (time_received * 1000) as i64)
                            }
                            semtech_udp::StringOrNum::S(_) => None,
                        };
                        lorawan
                            .handle_event(LorawanEvent::RadioEvent(radio::Event::PhyEvent(frame)))
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
                            debug!("TimeoutRequest: {:?}", ms)
                        }
                        LorawanResponse::JoinSuccess => {
                            send_uplink = true;

                            if let Some(time_remaining) = time_remaining.take() {
                                let time_remaining_seconds = (time_remaining as f64) / 1000000.0;
                                METRICS
                                    .join_latency
                                    .with_label_values(&["1"])
                                    .observe(time_remaining_seconds);
                                info!(
                                    "Join success, time remaining in seconds: {}",
                                    time_remaining_seconds
                                );
                            }
                            METRICS.join_success_counter.inc();
                        }
                        LorawanResponse::ReadyToSend => {
                            send_uplink = true;
                            debug!("Ready to send")
                        }
                        LorawanResponse::DownlinkReceived(fcnt_down) => {
                            send_uplink = true;
                            if let Some(time_remaining) = time_remaining.take() {
                                let time_remaining_seconds = (time_remaining as f64) / 1000000.0;
                                METRICS
                                    .data_latency
                                    .with_label_values(&["1"])
                                    .observe(time_remaining_seconds);
                                info!(
                                "Downlink received with FCnt = {}, time remaining in seconds: {}",
                                fcnt_down,
                                time_remaining_seconds
                            )
                            }
                            METRICS.data_success_counter.inc();
                        }
                        LorawanResponse::NoAck => {
                            send_uplink = true;
                            METRICS.data_fail_counter.inc();
                            warn!("RxWindow expired, expected ACK to confirmed uplink not received")
                        }
                        LorawanResponse::NoJoinAccept => {
                            self.sender.send(IntermediateEvent::NewSession).await?;
                            METRICS.join_fail_counter.inc();
                            warn!("No Join Accept Received")
                        }
                        LorawanResponse::SessionExpired => {
                            self.sender.send(IntermediateEvent::NewSession).await?;
                            debug!("SessionExpired. Created new Session")
                        }
                        LorawanResponse::NoUpdate => {
                            debug!("NoUpdate")
                        }
                        LorawanResponse::UplinkSending(fcnt_up) => {
                            info!("Uplink with FCnt {}", fcnt_up)
                        }
                        LorawanResponse::JoinRequestSending => {
                            info!("Join Request Sending")
                        }
                    },
                    Err(err) => error!("Error {:?}", err),
                }
                send_uplink
            };
            if send_uplink {
                let mut fport = rand::random();
                while fport == 0 {
                    fport = rand::random();
                }
                self.sender
                    .send(IntermediateEvent::SendPacket(vec![1, 2, 3, 4], fport, true))
                    .await?;
            }
        }
    }
}
