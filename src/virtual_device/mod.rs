use super::*;

use lorawan::default_crypto::DefaultFactory as LorawanCrypto;
use lorawan_device::region::Configuration;
use lorawan_device::{radio, Device, Event as LorawanEvent, JoinMode, Response as LorawanResponse};
use semtech_udp::client_runtime::DownlinkRequest;
use tokio::time::{sleep, Duration};
use udp_radio::UdpRadio;
pub(crate) use udp_radio::{mpsc, IntermediateEvent};
mod udp_radio;

pub struct VirtualDevice {
    label: String,
    device: Device<UdpRadio, LorawanCrypto, rand::rngs::OsRng, 512>,
    receiver: mpsc::Receiver<IntermediateEvent>,
    sender: mpsc::Sender<IntermediateEvent>,
    metrics_sender: metrics::Sender,
    rejoin_frames: u32,
    secs_between_transmits: u64,
    secs_between_join_transmits: u64,
}

#[derive(Clone)]
pub struct PacketSender {
    sender: mpsc::Sender<IntermediateEvent>,
}

impl PacketSender {
    pub async fn send(&self, downlink: Box<DownlinkRequest>, delayed_for: u64) -> Result {
        self.sender
            .send(IntermediateEvent::RadioEvent(downlink, delayed_for))
            .await
            .map_err(Error::SendingDownlinkToUdpRadio)
    }
}

impl VirtualDevice {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        label: String,
        time: Instant,
        client_tx: ClientTx,
        credentials: Credentials,
        metrics_sender: metrics::Sender,
        rejoin_frames: u32,
        secs_between_transmits: u64,
        secs_between_join_transmits: u64,
        region: settings::Region,
    ) -> Result<(PacketSender, VirtualDevice)> {
        let (radio, receiver, sender) = UdpRadio::new(time, client_tx).await;

        let region = match region {
            settings::Region::AU915 => lorawan_device::Region::AU915,
            settings::Region::AS923_1 => lorawan_device::Region::AS923_1,
            settings::Region::AS923_2 => lorawan_device::Region::AS923_2,
            settings::Region::AS923_3 => lorawan_device::Region::AS923_3,
            settings::Region::AS923_4 => lorawan_device::Region::AS923_4,
            settings::Region::EU433 => lorawan_device::Region::EU433,
            settings::Region::EU868 => lorawan_device::Region::EU868,
            settings::Region::US915 => lorawan_device::Region::US915,
        };

        let device: Device<UdpRadio, LorawanCrypto, rand::rngs::OsRng, 512> = Device::new(
            Configuration::new(region),
            JoinMode::OTAA {
                deveui: credentials.deveui_cloned_into_buf()?,
                appeui: credentials.appeui_cloned_into_buf()?,
                appkey: credentials.appkey_cloned_into_buf()?,
            },
            radio,
            rand::rngs::OsRng,
        );

        Ok((
            PacketSender {
                sender: sender.clone(),
            },
            VirtualDevice {
                label,
                device,
                receiver,
                sender,
                metrics_sender,
                rejoin_frames,
                secs_between_transmits,
                secs_between_join_transmits,
            },
        ))
    }

    pub async fn run(mut self) -> Result<()> {
        // stagger the starts slightly
        let random = rand::random::<u64>() % 1000;
        sleep(Duration::from_millis(random)).await;

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
            let response = {
                match event {
                    IntermediateEvent::NewSession => {
                        lorawan.handle_event(LorawanEvent::NewSessionRequest)
                    }
                    IntermediateEvent::Timeout(id) => {
                        if lorawan.get_radio().most_recent_timeout(id) {
                            lorawan.handle_event(LorawanEvent::TimeoutFired)
                        } else {
                            Ok(LorawanResponse::NoUpdate)
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
                    // at this level, the RadioEvent is being delivered in the appropriate window
                    IntermediateEvent::RadioEvent(frame, _time_received) => lorawan
                        .handle_event(LorawanEvent::RadioEvent(radio::Event::PhyEvent(frame))),
                }
            };
            let (send_uplink, confirmed) = {
                let (mut send_uplink, mut confirmed) = (false, true);
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

                                if let Some(session) = lorawan.get_session_keys() {
                                    info!(
                                        "{:8} join success, time remaining: {:4} ms, {:?}",
                                        self.label,
                                        time_remaining / 1000,
                                        session
                                    )
                                }
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
                            confirmed = false;
                            warn!("{:8} RxWindow expired, expected ACK to confirmed uplink not received", self.label)
                        }
                        LorawanResponse::NoJoinAccept => {
                            metrics_sender.send(metrics::Message::JoinFail).await?;
                            let duration = Duration::from_secs(self.secs_between_join_transmits);
                            let sender = self.sender.clone();
                            tokio::spawn(async move {
                                sleep(duration).await;
                                sender.send(IntermediateEvent::NewSession).await.unwrap();
                            });
                            warn!(
                                "{:8} No Join Accept Received, sending again in: {} ms",
                                self.label,
                                duration.as_millis()
                            )
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
                (send_uplink, confirmed)
            };
            if send_uplink {
                if let Some(fcnt_up) = lorawan.get_fcnt_up() {
                    if fcnt_up > self.rejoin_frames {
                        self.sender.send(IntermediateEvent::NewSession).await?;
                    } else {
                        let mut fport = rand::random();
                        while fport == 0 {
                            fport = rand::random();
                        }

                        let sender = self.sender.clone();
                        let duration = Duration::from_secs(self.secs_between_transmits);
                        tokio::spawn(async move {
                            sleep(duration).await;
                            sender
                                .send(IntermediateEvent::SendPacket(
                                    vec![
                                        rand::random(),
                                        rand::random(),
                                        rand::random(),
                                        rand::random(),
                                    ],
                                    fport,
                                    confirmed,
                                ))
                                .await
                                .unwrap();
                        });
                    }
                }
            }
        }
    }
}
