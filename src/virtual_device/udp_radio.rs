#![macro_use]
use crate::warn;

use lorawan_device::{radio, Timings};
use semtech_udp::client_runtime;
use semtech_udp::{push_data, Bandwidth, CodingRate, DataRate, SpreadingFactor, StringOrNum};
use std::{
    marker::PhantomData,
    time::{Duration, Instant},
};
pub use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::sleep;

#[derive(Debug)]
// I need some intermediate event because of Lifetimes
// maybe there's a cleaner way of doing this
pub enum IntermediateEvent {
    UdpRx(Box<semtech_udp::pull_resp::Packet>, u64),
    NewSession,
    Timeout(usize),
    SendPacket(Vec<u8>, u8, bool),
}

#[derive(Debug)]
pub enum Response {}

#[derive(Debug)]
pub struct UdpRadio<'a> {
    udp_sender: Sender<client_runtime::TxMessage>,
    lorawan_sender: Sender<IntermediateEvent>,
    rx_buffer: Buffer,
    time: Instant,
    settings: Settings,
    timeout_id: usize,
    phantom: PhantomData<&'a u8>,
    window_start: u32,
}

impl<'a> UdpRadio<'a> {
    pub async fn new(
        time: Instant,
        udp_runtime: &semtech_udp::client_runtime::UdpRuntime,
    ) -> (
        UdpRadio<'a>,
        tokio::sync::mpsc::Receiver<IntermediateEvent>,
        tokio::sync::mpsc::Sender<IntermediateEvent>,
    ) {
        let (mut udp_receiver, udp_sender) = (udp_runtime.subscribe(), udp_runtime.publish_to());

        let (lorawan_sender, lorawan_receiver) = mpsc::channel(100);
        let udp_lorawan_sender = lorawan_sender.clone();

        // this task receives downlinks and sends them to the lorawan layer as if a PHY radio
        // received the frame
        tokio::spawn(async move {
            loop {
                let event = udp_receiver.recv().await.unwrap();
                if let semtech_udp::Packet::Down(semtech_udp::Down::PullResp(pull_resp)) = event {
                    let udp_lorawan_sender = udp_lorawan_sender.clone();
                    match &pull_resp.data.txpk.tmst {
                        // here we will hold the frame until the RxWindow begins
                        StringOrNum::N(n) => {
                            let scheduled_time = n;
                            let time = time.elapsed().as_micros() as u64;
                            if scheduled_time > &time {
                                let delay = scheduled_time - time as u64;
                                tokio::spawn(async move {
                                    sleep(Duration::from_millis(delay + 50)).await;
                                    udp_lorawan_sender
                                        .send(IntermediateEvent::UdpRx(pull_resp, time))
                                        .await
                                        .unwrap();
                                });
                            } else {
                                let time_since_scheduled_time = time - scheduled_time;
                                warn!(
                                    "UDP packet received after tx time by {} ms",
                                    time_since_scheduled_time
                                );
                            }
                        }
                        StringOrNum::S(_) => {
                            warn!("Warning! UDP packet sent with \"immediate\"");
                        }
                    }
                }
            }
        });

        (
            UdpRadio {
                rx_buffer: Buffer::default(),
                time,
                settings: Settings::default(),
                udp_sender,
                timeout_id: 0,
                phantom: PhantomData::default(),
                lorawan_sender: lorawan_sender.clone(),
                window_start: 0,
            },
            lorawan_receiver,
            lorawan_sender,
        )
    }

    pub async fn timer(&mut self, future_time: u32) {
        let timeout_id = rand::random::<usize>();
        self.timeout_id = timeout_id;

        let elapsed = self.time.elapsed().as_millis() as u32;
        // only kick out the packet if its on time
        if future_time > elapsed {
            let delay = future_time - elapsed;
            let sender = self.lorawan_sender.clone();

            tokio::spawn(async move {
                sleep(Duration::from_millis(delay as u64)).await;
                sender
                    .send(IntermediateEvent::Timeout(timeout_id))
                    .await
                    .unwrap()
            });
            self.window_start = delay;
        }
    }

    pub fn most_recent_timeout(&mut self, timeout_id: usize) -> bool {
        self.timeout_id == timeout_id
    }
}
use heapless::Vec as HVec;
#[derive(Default, Debug)]
pub struct Buffer {
    data: HVec<u8, 255>,
}

impl lorawan_device::radio::PhyRxTxBuf for Buffer {
    fn clear(&mut self) {
        self.data.clear();
    }
    fn extend(&mut self, buf: &[u8]) {
        self.data.extend_from_slice(buf).unwrap();
    }
}

impl std::convert::AsMut<[u8]> for Buffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self.data.as_mut()
    }
}

impl std::convert::AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        self.data.as_ref()
    }
}

use lorawan_device::radio::PhyRxTxBuf;
use lorawan_device::radio::{
    Error as LoraError, Event as LoraEvent, Response as LoraResponse, RxQuality,
};

impl<'a> radio::PhyRxTx for UdpRadio<'a> {
    type PhyError = Error;
    type PhyResponse = Response;
    type PhyEvent = Box<semtech_udp::pull_resp::Packet>;
    type PhyBuf = Buffer;
    fn get_mut_radio(&mut self) -> &mut Self {
        self
    }

    fn get_received_packet(&mut self) -> &mut Buffer {
        &mut self.rx_buffer
    }

    fn handle_event(
        &mut self,
        event: LoraEvent<Self>,
    ) -> Result<LoraResponse<Self>, LoraError<Self>> {
        use semtech_udp::push_data::*;
        match event {
            radio::Event::TxRequest(tx_config, buffer) => {
                let size = buffer.data.len() as u64;
                let tmst = self.time.elapsed().as_micros() as u64;

                let settings = Settings::from(tx_config);
                let mut data = Vec::new();
                data.extend_from_slice(buffer.data.as_ref());
                let rxpk = RxPkV1 {
                    chan: 0,
                    codr: settings.get_codr(),
                    data,
                    datr: settings.get_datr(),
                    freq: settings.get_freq(),
                    lsnr: 5.5,
                    modu: semtech_udp::Modulation::LORA,
                    rfch: 0,
                    rssi: -112,
                    rssis: None,
                    size,
                    stat: semtech_udp::push_data::CRC::OK,
                    tmst,
                };
                let packet = push_data::Packet::from_rxpk(RxPk::V1(rxpk));

                if let Err(e) = self.udp_sender.try_send(packet.into()) {
                    panic!("UdpTx Queue Overflow! {}", e)
                }

                Ok(radio::Response::TxDone(
                    self.time.elapsed().as_millis() as u32
                ))
            }
            radio::Event::RxRequest(config) => {
                self.settings.rfconfig = config;
                Ok(radio::Response::Idle)
            }
            radio::Event::CancelRx => Ok(radio::Response::Idle),
            radio::Event::PhyEvent(packet) => {
                self.rx_buffer.clear();
                self.rx_buffer.extend(packet.data.txpk.data.as_slice());
                let ack = packet
                    .into_ack_for_gateway(semtech_udp::MacAddress::new(&[0, 0, 0, 0, 0, 0, 0, 0]));
                let sender = self.udp_sender.clone();
                // we are not in an async context so we must spawn this off
                tokio::task::spawn(async move { sender.send(ack.into()).await });
                Ok(LoraResponse::RxDone(RxQuality::new(-120, 5)))
            }
        }
    }
}

impl<'a> Timings for UdpRadio<'a> {
    fn get_rx_window_offset_ms(&self) -> i32 {
        20
    }
    fn get_rx_window_duration_ms(&self) -> u32 {
        100
    }
}

#[derive(Debug)]
pub enum Error {}

#[derive(Debug)]
struct Settings {
    rfconfig: radio::RfConfig,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            rfconfig: radio::RfConfig {
                frequency: 903000000,
                bandwidth: lorawan_device::radio::Bandwidth::_125KHz,
                spreading_factor: lorawan_device::radio::SpreadingFactor::_7,
                coding_rate: lorawan_device::radio::CodingRate::_4_5,
            },
        }
    }
}

impl From<radio::TxConfig> for Settings {
    fn from(txconfig: radio::TxConfig) -> Settings {
        Settings {
            rfconfig: txconfig.rf,
        }
    }
}

impl Settings {
    fn get_datr(&self) -> DataRate {
        DataRate::new(
            match self.rfconfig.spreading_factor {
                radio::SpreadingFactor::_7 => SpreadingFactor::SF7,
                radio::SpreadingFactor::_8 => SpreadingFactor::SF8,
                radio::SpreadingFactor::_9 => SpreadingFactor::SF9,
                radio::SpreadingFactor::_10 => SpreadingFactor::SF10,
                radio::SpreadingFactor::_11 => SpreadingFactor::SF11,
                radio::SpreadingFactor::_12 => SpreadingFactor::SF12,
            },
            match self.rfconfig.bandwidth {
                radio::Bandwidth::_125KHz => Bandwidth::BW125,
                radio::Bandwidth::_250KHz => Bandwidth::BW250,
                radio::Bandwidth::_500KHz => Bandwidth::BW500,
            },
        )
    }

    fn get_codr(&self) -> CodingRate {
        match self.rfconfig.coding_rate {
            radio::CodingRate::_4_5 => CodingRate::_4_5,
            radio::CodingRate::_4_6 => CodingRate::_4_6,
            radio::CodingRate::_4_7 => CodingRate::_4_7,
            radio::CodingRate::_4_8 => CodingRate::_4_8,
        }
    }

    fn get_freq(&self) -> f64 {
        self.rfconfig.frequency as f64 / 1_000_000.0
    }
}
