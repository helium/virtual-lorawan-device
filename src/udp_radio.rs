use super::udp_runtime;
use heapless::consts::*;
use heapless::Vec as HVec;
use lorawan_device::{radio::*, Event as LorawanEvent, State as LorawanState, Timings};
use semtech_udp::{PacketData, PushData, RxPk};
use tokio::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug)]
#[allow(dead_code)]
#[allow(clippy::large_enum_variant)]
pub enum RadioEvent {
    UdpRx(udp_runtime::RxMessage),
    TxDone,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Event {
    Radio(RadioEvent),
    CreateSession,
    Timeout,
    SendPacket,
    Shutdown,
}

impl Settings {
    fn get_datr(&self) -> String {
        format!(
            "{}{}",
            match self.rfconfig.spreading_factor {
                SpreadingFactor::_7 => "SF7",
                SpreadingFactor::_8 => "SF8",
                SpreadingFactor::_9 => "SF9",
                SpreadingFactor::_10 => "SF10",
                SpreadingFactor::_11 => "SF11",
                SpreadingFactor::_12 => "SF12",
            },
            match self.rfconfig.bandwidth {
                Bandwidth::_125KHZ => "BW125",
                Bandwidth::_250KHZ => "BW250",
                Bandwidth::_500KHZ => "BW500",
            }
        )
    }

    fn get_codr(&self) -> String {
        match self.rfconfig.coding_rate {
            CodingRate::_4_5 => "4/5",
            CodingRate::_4_6 => "4/6",
            CodingRate::_4_7 => "4/7",
            CodingRate::_4_8 => "4/8",
        }
        .to_string()
    }

    fn get_freq(&self) -> f64 {
        self.rfconfig.frequency as f64 / 1_000_000.0
    }
}

// Runtime translates UDP events into Device events
pub struct UdpRadioRuntime {
    receiver: Receiver<udp_runtime::RxMessage>,
    lorawan_sender: Sender<Event>,
}

impl UdpRadioRuntime {
    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            if let Some(event) = self.receiver.recv().await {
                self.lorawan_sender
                    .send(Event::Radio(RadioEvent::UdpRx(event)))
                    .await?;
            }
        }
    }
}

use std::time::Instant;

#[derive(Default)]
struct Settings {
    rfconfig: RfConfig,
}

pub struct UdpRadio {
    sender: Sender<udp_runtime::TxMessage>,
    lorawan_sender: Sender<Event>,
    rx_buffer: HVec<u8, U256>,
    settings: Settings,
    time: Instant,
    window_start: u32,
    window_close: u32,
}

impl UdpRadio {
    pub fn new(
        sender: Sender<udp_runtime::TxMessage>,
        receiver: Receiver<udp_runtime::RxMessage>,
    ) -> (Receiver<Event>, UdpRadioRuntime, Sender<Event>, UdpRadio) {
        let (lorawan_sender, lorawan_receiver) = mpsc::channel(100);
        let lorawan_sender_clone = lorawan_sender.clone();
        let lorawan_sender_another_clone = lorawan_sender.clone();

        (
            lorawan_receiver,
            UdpRadioRuntime {
                receiver,
                lorawan_sender,
            },
            lorawan_sender_another_clone,
            UdpRadio {
                sender,
                lorawan_sender: lorawan_sender_clone,
                rx_buffer: HVec::new(),
                settings: Settings {
                    rfconfig: RfConfig::default(),
                },
                time: Instant::now(),
                window_start: 0,
                window_close: 0,
            },
        )
    }

    pub fn timer(&mut self, delay: u32) {
        self.window_start = delay;
    }

    pub fn time_until_window_ms(&self) -> isize {
        println!("{}", self.time.elapsed().as_millis());
        let time = self.time.elapsed().as_millis() as isize;
        (self.window_start as isize - time)
    }
}

impl PhyRxTx for UdpRadio {
    type PhyEvent = RadioEvent;

    fn send(&mut self, buffer: &mut [u8]) {
        let size = buffer.len() as u64;
        let data = base64::encode(buffer);
        let tmst = self.time.elapsed().as_micros() as u64;

        let mut packet = Vec::new();
        packet.push({
            RxPk {
                chan: 0,
                codr: self.settings.get_codr(),
                data,
                datr: self.settings.get_datr(),
                freq: self.settings.get_freq(),
                lsnr: 5.5,
                modu: "LORA".to_string(),
                rfch: 0,
                rssi: -112,
                size,
                stat: 1,
                tmst,
            }
        });
        let rxpk = Some(packet);

        let packet =
            semtech_udp::Packet::from_data(PacketData::PushData(PushData { rxpk, stat: None }));

        println!("sending packet!");

        if let Err(e) = self.sender.try_send(packet) {
            panic!("UdpTx Queue Overflow! {}", e)
        }

        // sending the packet pack to ourselves simulates a SX12xx DI0 interrupt
        if let Err(e) = self
            .lorawan_sender
            .try_send(Event::Radio(RadioEvent::TxDone))
        {
            panic!("LoRaWAN Queue Overflow! {}", e)
        }
    }

    fn get_received_packet(&mut self) -> &mut HVec<u8, U256> {
        &mut self.rx_buffer
    }

    fn configure_tx(&mut self, config: TxConfig) {
        self.settings.rfconfig = config.rf;
    }

    fn configure_rx(&mut self, config: RfConfig) {
        self.settings.rfconfig = config;
    }

    fn set_rx(&mut self) {
        // normaly, this would configure the radio,
        // but the UDP port is always running concurrently
    }

    fn handle_phy_event(&mut self, event: RadioEvent) -> Option<PhyResponse> {
        println!("Handling PhyEvent {:?}", event);
        match event {
            RadioEvent::TxDone => Some(PhyResponse::TxDone(self.time.elapsed().as_micros() as u32)),
            RadioEvent::UdpRx(pkt) => match pkt.data {
                semtech_udp::PacketData::PullResp(pull_data) => {
                    let txpk = pull_data.txpk;
                    match base64::decode(txpk.data) {
                        Ok(data) => {
                            self.rx_buffer.clear();
                            for el in data {
                                if let Err(e) = self.rx_buffer.push(el) {
                                    panic!("Error pushing data into rx_buffer {}", e);
                                }
                            }
                            Some(PhyResponse::RxDone(RxQuality::new(-115, 4)))
                        }
                        Err(e) => panic!("Semtech UDP Packet Decoding Error {}", e),
                    }
                }
                // this will probably cause an issue once we get rid of confirmed downlinks
                semtech_udp::PacketData::PushAck => Some(PhyResponse::Busy),
                semtech_udp::PacketData::PullAck => Some(PhyResponse::Busy),
                _ => panic!("Unhandled packet type: {:?}", pkt.data),
            },
        }
    }
}

impl Timings for UdpRadio {
    fn get_rx_window_offset_ms(&mut self) -> isize {
        0
    }
    fn get_rx_window_duration_ms(&mut self) -> usize {
        100
    }
}
