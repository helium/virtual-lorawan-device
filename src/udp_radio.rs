#![macro_use]
use super::{udp_runtime, debugln, INSTANT};
use heapless::consts::*;
use heapless::Vec as HVec;
use lorawan_device::{radio::*, Event as LorawanEvent, State as LorawanState, Timings};
use semtech_udp::{PacketData, PushData, RxPk, StringOrNum};
use tokio::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use tokio::time::delay_for;

#[derive(Debug)]
#[allow(dead_code)]
#[allow(clippy::large_enum_variant)]
pub enum RadioEvent {
    Rx(semtech_udp::PullResp),
    TxDone,
}

impl From<semtech_udp::PullResp> for  RadioEvent{
    fn from(rx: semtech_udp::PullResp) -> Self {
        RadioEvent::Rx(rx)
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Event {
    Rx(semtech_udp::PullResp),
    CreateSession,
    Timeout,
    SendPacket,
    Shutdown,
    TxDone
}

impl From<semtech_udp::PullResp> for  Event{
    fn from(rx: semtech_udp::PullResp) -> Self {
        Event::Rx(rx)
    }
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
    time: Instant
}

impl UdpRadioRuntime {
    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            if let Some(event) = self.receiver.recv().await {
                if let semtech_udp::PacketData::PullResp(data) = event.data {
                    let mut sender = self.lorawan_sender.clone();

                    match &data.txpk.tmst {
                        StringOrNum::N(n) => {
                            let scheduled_time = n/1000;
                            let time = self.time.elapsed().as_millis() as u64;
                            if scheduled_time > time  {
                                // make units the same
                                let delay = scheduled_time - time as u64;
                                tokio::spawn(async move {
                                    delay_for(Duration::from_millis(delay + 50 )).await;
                                    sender.send(data.into()).await.unwrap();
                                });
                            } else {
                                let time_since_scheduled_time = time - scheduled_time;
                                debugln!(
                                            "Warning! UDP packet received after tx time by {} ms",
                                            time_since_scheduled_time
                                        );
                            }
                        },
                        StringOrNum::S(s) => {
                            debugln!(
                                    "\tWarning! UDP packet sent with \"immediate\"");
                        }
                    }
                }
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
    pub time: Instant,
    window_start: u32,
}

impl UdpRadio {
    pub fn new(
        sender: Sender<udp_runtime::TxMessage>,
        receiver: Receiver<udp_runtime::RxMessage>,
        time: Instant,
    ) -> (Receiver<Event>, UdpRadioRuntime, Sender<Event>, UdpRadio) {
        let (lorawan_sender, lorawan_receiver) = mpsc::channel(100);
        let lorawan_sender_clone = lorawan_sender.clone();
        let lorawan_sender_another_clone = lorawan_sender.clone();
        let time_clone= time.clone();
        (
            lorawan_receiver,
            UdpRadioRuntime {
                receiver,
                lorawan_sender,
                time: time_clone
            },
            lorawan_sender_another_clone,
            UdpRadio {
                sender,
                lorawan_sender: lorawan_sender_clone,
                rx_buffer: HVec::new(),
                settings: Settings {
                    rfconfig: RfConfig::default(),
                },
                time,
                window_start: 0,
            },
        )
    }

    pub async fn timer(&mut self, future_time: u32) {
        let mut sender = self.lorawan_sender.clone();
        let delay =             future_time - self.time.elapsed().as_millis() as u32;

        tokio::spawn(async move {
            delay_for(Duration::from_millis(delay as u64)).await;
            sender.send(Event::Timeout).await.unwrap();
        });
        self.window_start = delay;
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

        if let Err(e) = self.sender.try_send(packet) {
            panic!("UdpTx Queue Overflow! {}", e)
        }

        // sending the packet pack to ourselves simulates a SX12xx DI0 interrupt
        if let Err(e) = self
            .lorawan_sender
            .try_send(Event::TxDone)
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
        match event {
            RadioEvent::TxDone => Some(PhyResponse::TxDone(self.time.elapsed().as_millis() as u32)),
            RadioEvent::Rx(pkt) =>
                match base64::decode(pkt.txpk.data) {
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
    }
}

impl Timings for UdpRadio {
    fn get_rx_window_offset_ms(&mut self) -> i32 {
        20
    }
    fn get_rx_window_duration_ms(&mut self) -> u32 {
        100
    }
}
