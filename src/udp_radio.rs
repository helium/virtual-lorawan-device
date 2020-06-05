use heapless::consts::*;
use heapless::Vec as HVec;
use super::udp_runtime::{RxMessage, TxMessage};
use tokio::sync::mpsc::{self, Sender, Receiver};

use lorawan_device::{Radio, radio::*};
pub struct UdpRadio {
    sender: Sender<TxMessage>,
    rx_buffer: HVec<u8, U256>,
}


impl UdpRadio {
    pub fn new(sender: Sender<TxMessage>) -> UdpRadio {
        UdpRadio {
            sender,
            rx_buffer: HVec::new()
        }
    }
}

impl Radio for UdpRadio {
    type Event = RxMessage;

    fn send(&mut self, buffer: &mut [u8]) {

    }

    fn set_frequency(&mut self, frequency_mhz: u32) {

    }
    fn get_received_packet(&mut self) -> &mut HVec<u8, U256> {
        &mut self.rx_buffer
    }

    fn configure_tx(
        &mut self,
        power: i8,
        bandwidth: Bandwidth,
        spreading_factor: SpreadingFactor,
        coderate: CodingRate,
    ) {

    }

    fn configure_rx(
        &mut self,
        bandwidth: Bandwidth,
        spreading_factor: SpreadingFactor,
        coderate: CodingRate)
    {}

    fn set_rx(&mut self) {
    }

    fn handle_event(&mut self, event: Self::Event) -> State {
        State::TxDone
    }
}