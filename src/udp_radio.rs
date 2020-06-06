use heapless::consts::*;
use heapless::Vec as HVec;
use super::udp_runtime::{RxMessage, TxMessage};
use tokio::sync::mpsc::Sender;
use base64;
use semtech_udp::{PacketData, PushData, RxPk};
use lorawan_device::{Radio, radio::*};

struct Settings {
    bw: Bandwidth,
    sf: SpreadingFactor,
    cr: CodingRate,
    freq: u32
}

impl Settings {
    fn get_datr(&self) -> String {
        format!("{}{}",
            match self.sf {
                SpreadingFactor::_7 => "SF7",
                SpreadingFactor:: _8 => "SF8",
                SpreadingFactor::_9 => "SF9",
                SpreadingFactor::_10 => "SF10",
                SpreadingFactor::_11 => "SF11",
                SpreadingFactor::_12 => "SF12",
            },
                match self.bw {
                    Bandwidth::_125KHZ => "BW125",
                    Bandwidth::_250KHZ => "BW250",
                    Bandwidth::_500KHZ => "BW500",
                }
        )
    }

    fn get_codr(&self) -> String {
        match self.cr {
            CodingRate::_4_5 => "4/5",
            CodingRate::_4_6 => "4/6",
            CodingRate::_4_7 => "4/7",
            CodingRate::_4_8 => "4/8",
        }.to_string()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            bw: Bandwidth::_125KHZ,
            sf: SpreadingFactor::_10,
            cr: CodingRate::_4_5,
            freq: 902300000,
        }
    }
}

pub struct UdpRadio {
    sender: Sender<TxMessage>,
    rx_buffer: HVec<u8, U256>,
    settings: Settings
}


impl UdpRadio {
    pub fn new(sender: Sender<TxMessage>) -> UdpRadio {
        UdpRadio {
            sender,
            rx_buffer: HVec::new(),
            settings: Settings::default()
        }
    }
}

impl Radio for UdpRadio {
    type Event = RxMessage;

    fn send(&mut self, buffer: &mut [u8]) {
        let size = buffer.len() as u64;
        let data = base64::encode(buffer);

        let mut packet = Vec::new();
        packet.push({
            RxPk {
                chan: 0,
                codr: self.settings.get_codr(),
                data,
                datr: self.settings.get_datr(),
                freq: 0.0,
                lsnr: 5.5,
                modu: "LORA".to_string(),
                rfch: 5,
                rssi: -112,
                size,
                stat: 1,
                tmst: 32,
            }
        });
        let rxpk = Some(packet);

        let packet = semtech_udp::Packet {
            random_token: 0xAB,
            gateway_mac: None,
            data: PacketData::PushData(PushData{
                rxpk,
                stat: None,
            })
        };

        if let Err(e) = self.sender.try_send(packet) {
            panic!("UdpTx Queue Overflow! {}", e)
        }

    }

    fn set_frequency(&mut self, frequency_mhz: u32) {
        self.settings.freq = frequency_mhz;
    }

    fn get_received_packet(&mut self) -> &mut HVec<u8, U256> {
        &mut self.rx_buffer
    }

    fn configure_tx(
        &mut self,
        _power: i8,
        bandwidth: Bandwidth,
        spreading_factor: SpreadingFactor,
        coderate: CodingRate,
    ) {
        self.settings.bw = bandwidth;
        self.settings.sf = spreading_factor;
        self.settings.cr = coderate;
    }

    fn configure_rx(
        &mut self,
        bandwidth: Bandwidth,
        spreading_factor: SpreadingFactor,
        coderate: CodingRate)
    {
        self.settings.bw = bandwidth;
        self.settings.sf = spreading_factor;
        self.settings.cr = coderate;
    }

    fn set_rx(&mut self) {

    }

    fn handle_event(&mut self, _event: Self::Event) -> State {
        State::TxDone
    }
}