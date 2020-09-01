/*
   This module wraps the UdpSocket objects such that a user can
   run sending and receiving concurrently as tasks,
   receive downlink packets and send uplink packets easily
*/
use semtech_udp::{parser::Parser, pull_data, Down, MacAddress, Packet, SerializablePacket, Up};
use std::net::SocketAddr;
use tokio::net::udp::{RecvHalf, SendHalf};
use tokio::net::UdpSocket;
use tokio::sync::{
    broadcast,
    mpsc::{self, Receiver, Sender},
};

pub type RxMessage = semtech_udp::Packet;
pub type TxMessage = semtech_udp::Packet;

struct UdpRuntimeRx {
    sender: broadcast::Sender<RxMessage>,
    udp_sender: Sender<TxMessage>,
    socket_recv: RecvHalf,
}

#[derive(Debug)]
pub enum Error {
    SemtechUdpSerialize(semtech_udp::Error),
    SemtechUdpDeserialize(semtech_udp::parser::Error),
    SendError(tokio::sync::mpsc::error::SendError<TxMessage>),
}

impl From<semtech_udp::parser::Error> for Error {
    fn from(err: semtech_udp::parser::Error) -> Error {
        Error::SemtechUdpDeserialize(err)
    }
}

impl From<tokio::sync::mpsc::error::SendError<TxMessage>> for Error {
    fn from(err: tokio::sync::mpsc::error::SendError<TxMessage>) -> Error {
        Error::SendError(err)
    }
}

impl From<semtech_udp::Error> for Error {
    fn from(err: semtech_udp::Error) -> Error {
        Error::SemtechUdpSerialize(err)
    }
}

struct UdpRuntimeTx {
    receiver: Receiver<TxMessage>,
    sender: Sender<TxMessage>,
    socket_send: SendHalf,
}

pub struct UdpRuntime {
    rx: UdpRuntimeRx,
    tx: UdpRuntimeTx,
    poll_sender: Sender<TxMessage>,
}

impl UdpRuntime {
    fn split(self) -> (UdpRuntimeRx, UdpRuntimeTx, Sender<TxMessage>) {
        (self.rx, self.tx, self.poll_sender)
    }

    pub fn publish_to(&self) -> Sender<TxMessage> {
        self.tx.sender.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RxMessage> {
        self.rx.sender.subscribe()
    }

    pub async fn run(self) -> Result<(), Error> {
        let (rx, tx, mut poll_sender) = self.split();

        // udp_runtime_rx reads from the UDP port
        // and sends packets to the receiver channel
        tokio::spawn(async move {
            if let Err(e) = rx.run().await {
                panic!("UdpRuntimeRx threw error: {:?}", e)
            }
        });

        // udp_runtime_tx writes to the UDP port
        // by receiving packets from the sender channel
        tokio::spawn(async move {
            if let Err(e) = tx.run().await {
                panic!("UdpRuntimeTx threw error: {:?}", e)
            }
        });

        // spawn a timer for telling tx to send a PullReq frame
        tokio::spawn(async move {
            loop {
                let packet = pull_data::Packet::default();
                if let Err(e) = poll_sender.send(packet.into()).await {
                    panic!("UdpRuntime error from sending PullData {}", e)
                }

                delay_for(Duration::from_millis(10000)).await;
            }
        });

        Ok(())
    }

    pub async fn new(
        local: SocketAddr,
        host: SocketAddr,
    ) -> Result<UdpRuntime, Box<dyn std::error::Error>> {
        let mut socket = UdpSocket::bind(&local).await?;
        // "connecting" filters for only frames from the server
        socket.connect(host).await?;
        socket.send(&[0]).await?;
        let (rx_sender, _) = broadcast::channel(100);
        let (tx_sender, tx_receiver) = mpsc::channel(100);

        let (socket_recv, socket_send) = socket.split();
        Ok(UdpRuntime {
            rx: UdpRuntimeRx {
                sender: rx_sender,
                udp_sender: tx_sender.clone(),
                socket_recv,
            },
            poll_sender: tx_sender.clone(),
            tx: UdpRuntimeTx {
                receiver: tx_receiver,
                sender: tx_sender,
                socket_send,
            },
        })
    }
}

use std::time::Duration;
use tokio::time::delay_for;

impl UdpRuntimeRx {
    pub async fn run(mut self) -> Result<(), Error> {
        let mut buf = vec![0u8; 1024];
        loop {
            match self.socket_recv.recv(&mut buf).await {
                Ok(n) => {
                    let packet = semtech_udp::Packet::parse(&buf[0..n], n)?;
                    match packet {
                        Packet::Up(_) => panic!("Should not be receiving any up packets"),
                        Packet::Down(down) => match down.clone() {
                            Down::PullResp(pull_resp) => {
                                // send downlinks to LoRaWAN layer
                                self.sender.send(pull_resp.clone().into()).unwrap();
                                // provide ACK
                                self.udp_sender.send(pull_resp.into_ack().into()).await?;
                            }
                            Down::PullAck(_) | Down::PushAck(_) => (),
                        },
                    }
                }
                Err(e) => {
                    println!("Socket receive error: {}", e);
                }
            }
        }
    }
}

impl UdpRuntimeTx {
    pub async fn run(mut self) -> Result<(), Error> {
        let mut buf = vec![0u8; 1024];
        let mac: [u8; 8] = [170, 85, 90, 0, 0, 0, 0, 5];
        loop {
            let tx = self.receiver.recv().await;
            if let Some(mut data) = tx {
                match &mut data {
                    Packet::Up(ref mut up) => {
                        up.set_gateway_mac(MacAddress::new(&mac));

                        match up {
                            Up::PushData(ref mut push_data) => {
                                push_data.random_token = super::get_random_u32() as u16;
                            }
                            Up::PullData(_) => (),
                            Up::TxAck(ref mut tx_ack) => {
                                tx_ack.random_token = super::get_random_u32() as u16;
                            }
                        }
                    }
                    Packet::Down(_) => panic!("Should not be sending any down packets"),
                }

                let n = data.serialize(&mut buf)? as usize;

                if let Err(e) = self.socket_send.send(&buf[..n]).await {
                    println!("Socket error: {}", e);
                }
            }
        }
    }
}
