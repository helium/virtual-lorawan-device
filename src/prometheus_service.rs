use hyper::{
    header::CONTENT_TYPE,
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use std::{fmt, sync::Mutex};
use tokio::sync::mpsc::{self, Receiver, Sender};

use prometheus::{Counter, Encoder, Gauge, HistogramVec, TextEncoder};

static mut SENDER: Option<Mutex<Sender<Message>>> = None;

#[derive(Debug)]
pub enum Message {
    JoinResponse,
    DownlinkResponse,
    HttpScrape(Sender<HttpData>),
}

#[derive(Debug)]
struct HttpData {
    format_type: String,
    data: Vec<u8>,
}

pub struct Prometheus {
    sender: Sender<Message>,
    receiver: Receiver<Message>,
}

#[derive(Debug)]
enum ServReqError {
    Hyper(hyper::Error),
    ChannelFull,
}

async fn serve_req(_req: Request<Body>) -> Result<Response<Body>, ServReqError> {
    // grab a copy from the mutex
    let mut sender = unsafe {
        if let Some(sender) = &SENDER {
            sender.lock().unwrap().clone()
        } else {
            panic!("Sender channel unintialized")
        }
    };

    let (http_sender, mut http_receiver) = mpsc::channel(100);
    sender.send(Message::HttpScrape(http_sender)).await?;

    let response = match http_receiver.recv().await {
        None => Response::builder()
            .status(408)
            .body(Body::from(format!("Failed to get Data")))
            .unwrap(),
        Some(data) => Response::builder()
            .status(200)
            .header(CONTENT_TYPE, data.format_type)
            .body(Body::from(data.data))
            .unwrap(),
    };

    Ok(response)
}

impl Prometheus {
    pub fn new() -> (Sender<Message>, Self) {
        let (sender, receiver) = mpsc::channel(100);

        (sender.clone(), Prometheus { sender, receiver })
    }

    async fn receiver_loop(mut receiver: Receiver<Message>) {
        let counter = register_counter!(opts!(
            "packet_sent_total",
            "Total number of packets sent",
            labels! {"handler" => "all",}
        ))
        .unwrap();

        let histogram = register_histogram_vec!(
            "packet_reponse_time",
            "The downlink response time in ms",
            &["handler"]
        )
        .unwrap();

        loop {
            let msg = receiver.recv().await;
            if let Some(msg) = msg {
                match msg {
                    Message::HttpScrape(mut response_channel) => {
                        let encoder = TextEncoder::new();
                        counter.inc();
                        let metric_families = prometheus::gather();
                        let mut buffer = vec![];
                        encoder.encode(&metric_families, &mut buffer).unwrap();
                        response_channel
                            .send(HttpData {
                                format_type: String::from(encoder.format_type()),
                                data: buffer,
                            })
                            .await
                            .unwrap();
                    }
                    _ => {
                        panic!("unimplemented");
                    }
                }
            }
        }
    }

    pub async fn run(self) -> Result<(), hyper::Error> {
        // initialize the mutex
        unsafe {
            SENDER = Some(Mutex::new(self.sender.clone()));
        }

        tokio::spawn(async move { Self::receiver_loop(self.receiver).await });

        let addr = ([0, 0, 0, 0], 9091).into();
        println!("Listening on http://{}", addr);

        let serve_future = Server::bind(&addr).serve(make_service_fn(|_| async {
            Ok::<_, hyper::Error>(service_fn(serve_req))
        }));

        serve_future.await
    }
}

impl From<hyper::Error> for ServReqError {
    fn from(err: hyper::Error) -> ServReqError {
        ServReqError::Hyper(err)
    }
}

impl From<mpsc::error::SendError<Message>> for ServReqError {
    fn from(err: mpsc::error::SendError<Message>) -> ServReqError {
        ServReqError::ChannelFull
    }
}

impl fmt::Display for ServReqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServReqError::Hyper(e) => write!(f, "ServReqError::Hyper({})", e),
            ServReqError::ChannelFull => write!(f, "ServReqError::ChannelFull"),
        }
    }
}

impl std::error::Error for ServReqError {}
