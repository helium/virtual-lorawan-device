/*
    This module wraps the UdpSocket objects such that a user can
    run sending and receiving concurrently as tasks,
    receive downlink packets and send uplink packets easily
 */
use tokio::net::UdpSocket;
use tokio::net::udp::{RecvHalf, SendHalf};
use tokio::sync::mpsc::{self, Sender, Receiver};
use std::net::SocketAddr;
use semtech_udp;

pub type RxMessage = (Vec<u8>, SocketAddr);
pub type TxMessage = semtech_udp::Packet;

pub struct UdpRuntimeRx {
    sender: Sender<RxMessage>,
    socket_recv: RecvHalf,
}

pub struct UdpRuntimeTx {
    receiver: Receiver<TxMessage>,
    socket_send: SendHalf,
}

pub struct UdpRuntime;

impl UdpRuntime {
    pub async fn new(host: SocketAddr) -> Result<(Receiver<RxMessage>, UdpRuntimeRx, Sender<TxMessage>, UdpRuntimeTx), Box<dyn std::error::Error>> {
        let mut socket = UdpSocket::bind(&host).await?;
        // "connecting" filters for only frames from the server
        socket.connect("127.0.0.1:1680").await?;
        // send something so that server can know about us
        socket.send(&[0]).await?;
        let (rx_sender, rx_receiver) = mpsc::channel(100);
        let (tx_sender, tx_receiver) = mpsc::channel(100);

        let (socket_recv, socket_send) = socket.split();
        Ok(
            (
            rx_receiver,
            UdpRuntimeRx {
                sender: rx_sender,
                socket_recv,
            },
            tx_sender,
            UdpRuntimeTx {
                 receiver: tx_receiver,
                 socket_send,
             },
            )
        )
    }
}

impl UdpRuntimeRx {
    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = vec![0u8; 1024];
        loop {
            match self.socket_recv.recv_from(&mut buf).await {
                Ok( (n, sender) ) =>
                    self.sender.send((buf[..n].to_vec(), sender)).await?,
                Err(e) => {
                    return Err(e.into())
                }
            }
        }
    }
}

impl UdpRuntimeTx {
    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = vec![0u8; 1024];
        loop {
            let tx = self.receiver.recv().await;
            if let Some(data) = tx {
                data.serialize(&mut buf)?;
                self.socket_send.send(&buf).await?;
                buf.clear();
            }
        }
    }
}
