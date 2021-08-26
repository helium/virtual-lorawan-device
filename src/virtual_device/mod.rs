use super::*;

use lorawan_device::{radio, region, Device, Event as LorawanEvent, Response as LorawanResponse};

use lorawan_encoding::default_crypto::DefaultFactory as LorawanCrypto;
use udp_radio::UdpRadio;
pub(crate) use udp_radio::{IntermediateEvent, Receiver, Sender};
mod udp_radio;

pub struct VirtualDevice<'a> {
    label: String,
    device: Device<UdpRadio<'a>, LorawanCrypto>,
    receiver: Receiver<IntermediateEvent>,
    sender: Sender<IntermediateEvent>,
    metrics_sender: metrics::Sender,
}

impl<'a> VirtualDevice<'a> {
    pub async fn new(
        label: String,
        instant: Instant,
        udp_runtime: &semtech_udp::client_runtime::UdpRuntime,
        credentials: Credentials,
        metrics_sender: metrics::Sender,
    ) -> Result<VirtualDevice<'a>> {
        let (radio, receiver, sender) = UdpRadio::new(instant, udp_runtime).await;
        let device: Device<udp_radio::UdpRadio, LorawanCrypto> = Device::new(
            region::US915::subband(2).into(),
            radio,
            credentials.deveui_cloned_into_buf()?,
            credentials.appeui_cloned_into_buf()?,
            credentials.appkey_cloned_into_buf()?,
            rand::random::<u32>,
        );

        Ok(VirtualDevice {
            label,
            device,
            receiver,
            sender,
            metrics_sender,
        })
    }

    pub async fn run(mut self) -> Result<()> {
        // Kickstart activity by trying to join
        self.sender
            .send(IntermediateEvent::NewSession)
            .await
            .unwrap();

        let mut time_remaining = None;
        let mut lorawan = self.device;
        let mut metrics_sender = self.metrics_sender;
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
                        // this will only be None if there is no session
                        if let Some(fcnt_up) = lorawan.get_fcnt_up() {
                            info!(
                                "{:8} sending packet fcnt = {} on fport {}",
                                self.label, fcnt_up, fport
                            );
                        }
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
                            debug!("{:8} TimeoutRequest: {:?}", self.label, ms)
                        }
                        LorawanResponse::JoinSuccess => {
                            send_uplink = true;
                            if let Some(time_remaining) = time_remaining.take() {
                                metrics_sender
                                    .send(metrics::Message::JoinSuccess(time_remaining))
                                    .await?;
                                info!(
                                    "{:8} join success, time remaining: {:4} ms",
                                    self.label,
                                    time_remaining / 1000
                                );
                            }
                        }
                        LorawanResponse::ReadyToSend => {
                            send_uplink = true;
                            debug!("{:8} ready to send", self.label)
                        }
                        LorawanResponse::DownlinkReceived(fcnt_down) => {
                            send_uplink = true;
                            if let Some(time_remaining) = time_remaining.take() {
                                metrics_sender
                                    .send(metrics::Message::DataSuccess(time_remaining))
                                    .await?;
                                info!(
                                    "{:8} downlink received with fcnt = {}, time remaining: {:4} ms",
                                    self.label,
                                    fcnt_down,
                                    time_remaining / 1000
                                )
                            }
                        }
                        LorawanResponse::NoAck => {
                            metrics_sender.send(metrics::Message::DataFail).await?;
                            send_uplink = true;
                            warn!("{:8} RxWindow expired, expected ACK to confirmed uplink not received", self.label)
                        }
                        LorawanResponse::NoJoinAccept => {
                            metrics_sender.send(metrics::Message::JoinFail).await?;
                            self.sender.send(IntermediateEvent::NewSession).await?;
                            warn!("{:8} No Join Accept Received", self.label)
                        }
                        LorawanResponse::SessionExpired => {
                            self.sender.send(IntermediateEvent::NewSession).await?;
                            debug!("{:8} SessionExpired. Created new Session", self.label)
                        }
                        LorawanResponse::NoUpdate => {
                            debug!("{:8} NoUpdate", self.label)
                        }
                        LorawanResponse::UplinkSending(fcnt_up) => {
                            info!("{:8} Uplink with FCnt {}", self.label, fcnt_up)
                        }
                        LorawanResponse::JoinRequestSending => {
                            info!("{:8} Join Request Sending", self.label)
                        }
                    },
                    // silent errors since we receive radio frames for other devices
                    Err(_err) => (),
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
