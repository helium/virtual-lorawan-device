/*
   This module wraps the UdpSocket objects such that a user can
   run sending and receiving concurrently as tasks,
   receive downlink packets and send uplink packets easily
*/
use semtech_udp;
use std::net::SocketAddr;
use tokio::net::udp::{RecvHalf, SendHalf};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::{self, Receiver, Sender};

pub type RxMessage = semtech_udp::Packet;
pub type TxMessage = semtech_udp::Packet;

struct UdpRuntimeRx {
    sender: Sender<RxMessage>,
    socket_recv: RecvHalf,
}

struct UdpRuntimeTx {
    receiver: Receiver<TxMessage>,
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

    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let (rx, tx, mut poll_sender) = self.split();

        // udp_runtime_rx reads from the UDP port
        // and sends packets to the receiver channel
        tokio::spawn(async move {
            if let Err(e) = rx.run().await {
                panic!("UdpRuntimeRx threw error: {}", e)
            }
        });

        // udp_runtime_tx writes to the UDP port
        // by receiving packets from the sender channel
        tokio::spawn(async move {
            if let Err(e) = tx.run().await {
                panic!("UdpRuntimeTx threw error: {}", e)
            }
        });

        // spawn a timer for telling tx to send a PullReq frame
        tokio::spawn(async move {
            loop {
                delay_for(Duration::from_millis(100)).await;

                let foo = [
                    0x12, 0x45, 0x32, 0x42, 0x33, 0x00, 0x00, 0x12, 0x23, 0x32, 0x3, 0x3,
                ];

                let packet = semtech_udp::Packet {
                    random_token: 0x00,
                    gateway_mac: Some(semtech_udp::gateway_mac(&foo)),
                    data: semtech_udp::PacketData::PullData,
                };

                if let Err(e) = poll_sender.send(packet).await {
                    panic!("UdpRuntime error from sending PullData {}", e)
                }
                println!("Pulling");
            }
        });

        Ok(())
    }

    pub async fn new(
        host: SocketAddr,
    ) -> Result<(Receiver<RxMessage>, Sender<TxMessage>, UdpRuntime), Box<dyn std::error::Error>>
    {
        let mut socket = UdpSocket::bind(&host).await?;
        // "connecting" filters for only frames from the server
        socket.connect("127.0.0.1:1680").await?;
        // send something so that server can know about us
        socket.send(&[0]).await?;
        let (rx_sender, rx_receiver) = mpsc::channel(100);
        let (tx_sender, tx_receiver) = mpsc::channel(100);

        let tx_sender_clone = tx_sender.clone();
        let (socket_recv, socket_send) = socket.split();
        Ok((
            rx_receiver,
            tx_sender,
            UdpRuntime {
                rx: UdpRuntimeRx {
                    sender: rx_sender,
                    socket_recv,
                },
                tx: UdpRuntimeTx {
                    receiver: tx_receiver,
                    socket_send,
                },
                poll_sender: tx_sender_clone,
            },
        ))
    }
}

use std::time::Duration;
use tokio::time::delay_for;

impl UdpRuntimeRx {
    pub async fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = vec![0u8; 1024];
        loop {
            match self.socket_recv.recv_from(&mut buf).await {
                Ok((n, _)) => {
                    let packet = semtech_udp::Packet::parse(&mut buf[0..n], n)?;
                    self.sender.send(packet).await?
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl UdpRuntimeTx {
    pub async fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = vec![0u8; 1024];
        loop {
            let tx = self.receiver.recv().await;
            if let Some(data) = tx {
                let n = data.serialize(&mut buf)? as usize;
                self.socket_send.send(&buf[..n]).await?;
                buf.clear();
            }
        }
    }
}
