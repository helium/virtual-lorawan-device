use super::*;

use lorawan_device::{radio, region, Device, Event as LorawanEvent, Response as LorawanResponse};
use lorawan_encoding::default_crypto::DefaultFactory as LorawanCrypto;
use semtech_udp::StringOrNum;
use tokio::time::{sleep, Duration};
use udp_radio::UdpRadio;
pub(crate) use udp_radio::{IntermediateEvent, Receiver, Sender};
mod udp_radio;

pub struct VirtualDevice<'a> {
    label: String,
    device: Device<UdpRadio<'a>, LorawanCrypto>,
    time: Instant,
    receiver: Receiver<IntermediateEvent>,
    sender: Sender<IntermediateEvent>,
    metrics_sender: metrics::Sender,
    rejoin_frames: u32,
}

impl<'a> VirtualDevice<'a> {
    pub async fn new(
        label: String,
        time: Instant,
        udp_runtime: &semtech_udp::client_runtime::UdpRuntime,
        credentials: Credentials,
        metrics_sender: metrics::Sender,
        rejoin_frames: u32,
        region: settings::Region,
    ) -> Result<VirtualDevice<'a>> {
        let (radio, receiver, sender) = UdpRadio::new(time, udp_runtime).await;
        let region: region::Configuration = match region {
            settings::Region::US915 => region::US915::subband(2).into(),
            settings::Region::EU868 => region::EU868::default().into(),
        };

        let device: Device<udp_radio::UdpRadio, LorawanCrypto> = Device::new(
            region,
            radio,
            credentials.deveui_cloned_into_buf()?,
            credentials.appeui_cloned_into_buf()?,
            credentials.appkey_cloned_into_buf()?,
            rand::random::<u32>,
        );

        Ok(VirtualDevice {
            label,
            device,
            time,
            receiver,
            sender,
            metrics_sender,
            rejoin_frames,
        })
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
                    // UdpRx processes the raw UDP frame and delays it if necessary
                    IntermediateEvent::UdpRx(frame) => {
                        let self_sender = self.sender.clone();
                        match &frame.data.txpk.tmst {
                            // we will hold the frame until the RxWindow begins
                            StringOrNum::N(n) => {
                                let scheduled_time = *n;
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
                            }
                            StringOrNum::S(s) => {
                                warn!("{:8} Unexpected! UDP packet sent with {:?}", self.label, s);
                            }
                        }
                        (lorawan, Ok(LorawanResponse::NoUpdate))
                    }
                    // at this level, the RadioEvent is being delivered in the appopriate window
                    IntermediateEvent::RadioEvent(frame, time_received) => {
                        time_remaining = match frame.data.txpk.tmst {
                            semtech_udp::StringOrNum::N(tmst) => {
                                Some(tmst as i64 - time_received as i64)
                            }
                            semtech_udp::StringOrNum::S(_) => None,
                        };
                        lorawan
                            .handle_event(LorawanEvent::RadioEvent(radio::Event::PhyEvent(frame)))
                    }
                }
            };
            lorawan = new_state;
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
                        self.sender
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
                            .await?;
                    }
                }
            }
        }
    }
}
