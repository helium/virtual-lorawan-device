/*
    This module wraps the UdpSocket objects such that a user can
    run sending and receiving concurrently as tasks,
    receive downlink packets and send uplink packets easily
 */
use tokio::net::UdpSocket;
use tokio::net::udp::{RecvHalf, SendHalf};
use tokio::sync::mpsc;
use std::net::SocketAddr;

type ChannelMessage = (Vec<u8>, SocketAddr);

pub struct UdpRuntimeRx {
    sender: mpsc::Sender<ChannelMessage>,
    socket_recv: RecvHalf,
}

pub struct UdpRuntimeTx {
    sender: mpsc::Sender<ChannelMessage>,
    receiver: mpsc::Receiver<ChannelMessage>,
    socket_send: SendHalf,
}

pub struct UdpRuntime;

impl UdpRuntime {
    pub async fn new(host: SocketAddr) -> Result<(UdpRuntimeRx, UdpRuntimeTx), Box<dyn std::error::Error>> {
        let mut socket = UdpSocket::bind(&host).await?;
        // "connecting" filters for only frames from the server
        socket.connect("127.0.0.1:1680").await?;
        // send something so that server can know about us
        socket.send(&[0]).await?;
        let (sender, receiver) = mpsc::channel(100);
        let (mut socket_recv, socket_send) = socket.split();
        let sender_clone = sender.clone();
        Ok(
            (UdpRuntimeRx {
                sender,
                socket_recv,
            },
             UdpRuntimeTx {
                 receiver,
                 socket_send,
                 sender: sender_clone
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
    pub fn new_sender_channel(&self) -> mpsc::Sender<ChannelMessage> {
        self.sender.clone()
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            let msg = self.receiver.recv().await;
        }
    }
}
