use crate::config::Device;
use hyper::{
    header::CONTENT_TYPE,
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use prometheus::{CounterVec, Encoder, HistogramVec, TextEncoder};
use std::{fmt, sync::Mutex};
pub use tokio::sync::mpsc::{self, Receiver, Sender};

static mut SENDER: Option<Mutex<Sender<Message>>> = None;

#[derive(Debug)]
pub enum Message {
    Stat(Device, Stat),
    HttpScrape(Sender<HttpData>),
}

#[derive(Debug)]
pub enum Stat {
    DownlinkResponse(u64),
    DownlinkTimeout,
    JoinResponse(u64),
    JoinTimeout,
}

#[derive(Debug)]
pub struct HttpData {
    format_type: String,
    data: Vec<u8>,
}

pub struct Prometheus {
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    stats: Stats,
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
            .body(Body::from("Failed to get Data".to_string()))
            .unwrap(),
        Some(data) => Response::builder()
            .status(200)
            .header(CONTENT_TYPE, data.format_type)
            .body(Body::from(data.data))
            .unwrap(),
    };

    Ok(response)
}

struct Stats {
    data_success: CounterVec,
    data_fail: CounterVec,
    data_response_buffer: HistogramVec,
    join_success: CounterVec,
    join_fail: CounterVec,
    join_response_buffer: HistogramVec,
}

pub struct PrometheusBuilder {
    production_devices: Vec<String>,
    staging_devices: Vec<String>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
}

impl PrometheusBuilder {
    pub fn new() -> PrometheusBuilder {
        let (sender, receiver) = mpsc::channel(100);

        PrometheusBuilder {
            production_devices: Vec::new(),
            staging_devices: Vec::new(),
            sender,
            receiver,
        }
    }

    pub fn get_sender(&self) -> Sender<Message> {
        self.sender.clone()
    }

    pub fn register(&mut self, device: &super::config::Device) {
        let label = format!("_{}", &device.credentials().deveui()[12..]);

        if device.oui() == 1 {
            self.production_devices.push(label);
        } else if device.oui() == 2 {
            self.staging_devices.push(label);
        } else {
            panic!("Invalid OUI");
        }
    }

    pub fn build(self) -> Prometheus {
        let data_success = register_counter_vec!(
            opts!("data_success", "Total number of packets sent"),
            &["oui"]
        )
        .unwrap();

        let data_fail =
            register_counter_vec!(opts!("data_fail", "Total number of fail packets"), &["oui"])
                .unwrap();

        let data_response_buffer = register_histogram_vec!(
            "data_response_buffer",
            "Time remaining before timeout",
            &["oui"],
            vec![0.1, 0.20, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]
        )
        .unwrap();

        let join_success = register_counter_vec!(
            opts!("join_success", "Total number of packets sent"),
            &["oui"]
        )
        .unwrap();

        let join_fail =
            register_counter_vec!(opts!("join_fail", "Total number of fail packets"), &["oui"])
                .unwrap();

        let join_response_buffer = register_histogram_vec!(
            "join_response_buffer",
            "Time remaining before rx timeout in secs",
            &["oui"],
            vec![0.1, 0.25, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5]
        )
        .unwrap();

        let stats = Stats {
            data_success,
            data_fail,
            data_response_buffer,
            join_success,
            join_fail,
            join_response_buffer,
        };

        Prometheus {
            sender: self.sender,
            receiver: self.receiver,
            stats,
        }
    }
}

impl Prometheus {
    async fn receiver_loop(stats: Stats, mut receiver: Receiver<Message>) {
        loop {
            let msg = receiver.recv().await;
            if let Some(msg) = msg {
                match msg {
                    Message::Stat(config, stat) => {
                        let oui = config.oui().to_string();
                        let label = [oui.as_str()];
                        match stat {
                            Stat::DownlinkResponse(t) => {
                                let in_seconds = t as f64 / 1000.0;
                                stats
                                    .data_response_buffer
                                    .with_label_values(&label)
                                    .observe(in_seconds);
                                stats.data_success.with_label_values(&label).inc();
                            }
                            Stat::DownlinkTimeout => {
                                stats.data_fail.with_label_values(&label).inc();
                            }
                            Stat::JoinResponse(t) => {
                                let in_seconds = t as f64 / 1000.0;
                                stats
                                    .join_response_buffer
                                    .with_label_values(&label)
                                    .observe(in_seconds);
                                stats.join_success.with_label_values(&label).inc();
                            }
                            Stat::JoinTimeout => {
                                stats.join_fail.with_label_values(&label).inc();
                            }
                        }
                    }
                    Message::HttpScrape(mut response_channel) => {
                        let encoder = TextEncoder::new();
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
                }
            }
        }
    }

    pub async fn run(self) -> Result<(), hyper::Error> {
        // initialize the mutex
        unsafe {
            SENDER = Some(Mutex::new(self.sender.clone()));
        }

        tokio::spawn(async move { Self::receiver_loop(self.stats, self.receiver).await });

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
    fn from(_err: mpsc::error::SendError<Message>) -> ServReqError {
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
