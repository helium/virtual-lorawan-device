use super::*;

use lorawan::default_crypto::DefaultFactory as LorawanCrypto;
use lorawan_device::{
    radio, region, Device, Event as LorawanEvent, JoinMode, Response as LorawanResponse,
};
use semtech_udp::client_runtime::DownlinkRequest;
use tokio::time::{sleep, Duration};
use udp_radio::UdpRadio;
pub(crate) use udp_radio::{mpsc, IntermediateEvent};
mod udp_radio;

pub struct VirtualDevice {
    label: String,
    device: Device<UdpRadio, LorawanCrypto, 512>,
    time: Instant,
    receiver: mpsc::Receiver<IntermediateEvent>,
    sender: mpsc::Sender<IntermediateEvent>,
    metrics_sender: metrics::Sender,
    rejoin_frames: u32,
    secs_between_transmits: u64,
    secs_between_join_transmits: u64,
}

pub struct PacketSender {
    sender: mpsc::Sender<IntermediateEvent>,
}

impl PacketSender {
    pub async fn send(&self, downlink: Box<DownlinkRequest>) -> Result {
        self.sender
            .send(IntermediateEvent::UdpRx(downlink))
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
        let region: region::Configuration = match region {
            settings::Region::US915 => region::US915::subband(2).into(),
            settings::Region::EU868 => region::EU868::default().into(),
        };

        let device: Device<udp_radio::UdpRadio, LorawanCrypto, 512> = Device::new(
            region,
            JoinMode::OTAA {
                deveui: credentials.deveui_cloned_into_buf()?,
                appeui: credentials.appeui_cloned_into_buf()?,
                appkey: credentials.appkey_cloned_into_buf()?,
            },
            radio,
            rand::random::<u32>,
        );

        Ok((
            PacketSender {
                sender: sender.clone(),
            },
            VirtualDevice {
                label,
                device,
                time,
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
                    // UdpRx processes the raw UDP frame and delays it if necessary
                    IntermediateEvent::UdpRx(frame) => {
                        let self_sender = self.sender.clone();
                        if let Some(scheduled_time) = frame.pull_resp.data.txpk.time.tmst() {
                            let time = self.time.elapsed().as_micros() as u32;
                            if scheduled_time > time {
                                let delay = scheduled_time - time;
                                tokio::spawn(async move {
                                    sleep(Duration::from_micros(delay as u64 + 50_000)).await;
                                    self_sender
                                        .send(IntermediateEvent::RadioEvent(frame, time as u64))
                                        .await
                                        .unwrap();
                                });
                            } else {
                                let time_since_scheduled_time = time - scheduled_time;
                                warn!(
                                    "{:8} UDP packet received after tx time by {} μs",
                                    self.label, time_since_scheduled_time
                                );
                            }
                        } else {
                            warn!(
                                "{:8} Unexpected! UDP packet to transmit radio packet immediately",
                                self.label
                            );
                        }
                        Ok(LorawanResponse::NoUpdate)
                    }
                    // at this level, the RadioEvent is being delivered in the appropriate window
                    IntermediateEvent::RadioEvent(frame, time_received) => {
                        frame
                            .pull_resp
                            .data
                            .txpk
                            .time
                            .tmst()
                            .map(|tmst| tmst as i64 - time_received as i64);
                        lorawan
                            .handle_event(LorawanEvent::RadioEvent(radio::Event::PhyEvent(frame)))
                    }
                }
            };
            //lorawan = new_state;
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
