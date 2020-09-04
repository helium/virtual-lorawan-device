use super::prometheus_service::Stat;
use super::{config, debugln, prometheus_service as prometheus, INSTANT};
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server as HyperServer,
};
use serde_derive::{Deserialize, Serialize};
use std::{collections::HashMap, convert::Infallible, str, time::Duration};
pub use tokio::{
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
    time::delay_for,
};

#[derive(Debug)]
pub enum UplinkMessage {
    Expect(UplinkExpect),
    ExpectTimeout(UplinkExpect),
    Received(UplinkedReceived),
}

#[derive(Debug, Clone)]
pub struct UplinkExpect {
    device: config::Device,
    t: u128,
    payload: Vec<u8>,
    fport: u8,
    fcnt: u32,
}

impl UplinkExpect {
    pub fn new(
        device: config::Device,
        t: u128,
        input_payload: &[u8],
        fport: u8,
        fcnt: u32,
    ) -> UplinkExpect {
        let mut payload = Vec::new();
        payload.extend(input_payload);
        UplinkExpect {
            device,
            t,
            payload,
            fport,
            fcnt,
        }
    }

    pub fn hash_key(&self) -> (String, String, u32) {
        (
            self.device.credentials().appeui().clone(),
            self.device.credentials().deveui().clone(),
            self.fcnt,
        )
    }
}

#[derive(Debug)]
pub struct UplinkedReceived {
    app_eui: String,
    dev_eui: String,
    payload: Vec<u8>,
    t: u128,
    fcnt: u32,
}

impl UplinkedReceived {
    fn from_http_uplink(data: &DataIn) -> UplinkedReceived {
        UplinkedReceived {
            app_eui: data.app_eui.clone(),
            dev_eui: data.dev_eui.clone(),
            payload: base64::decode(&data.payload).unwrap(),
            t: INSTANT.elapsed().as_millis(),
            fcnt: data.fcnt,
        }
    }

    fn hash_key(&self) -> (String, String, u32) {
        (self.app_eui.clone(), self.dev_eui.clone(), self.fcnt)
    }
}

#[derive(Debug)]
pub struct Server {
    receiver: Receiver<UplinkMessage>,
    sender: Sender<UplinkMessage>,
    prometheus: Option<Sender<prometheus::Message>>,
}

#[derive(Deserialize, Debug)]
struct DataIn {
    app_eui: String,
    dev_eui: String,
    devaddr: String,
    fcnt: u32,
    payload: String,
}

lazy_static! {
    // this sender allows the data_received function to dispatch messages
    // to the task which compares Expected and Received messages
    static ref SENDER: Mutex<Option<Sender<UplinkMessage >>> = Mutex::new(None);
}

async fn data_received(request: Request<Body>) -> Result<Response<Body>, Infallible> {
    let body = hyper::body::to_bytes(request).await.unwrap();
    let data: DataIn = serde_json::from_slice(&body).unwrap();

    let dev_eui_len = data.dev_eui.len();
    let dev_eui = &data.dev_eui[dev_eui_len - 4..];
    debugln!(
        "{}: Received via HTTP \t\t(FCntUp  ={},\t{:?})",
        dev_eui,
        data.fcnt,
        base64::decode(data.payload.clone()).unwrap()
    );

    let sender_mutex = &mut *SENDER.lock().await;

    if let Some(sender) = sender_mutex {
        sender
            .send(UplinkMessage::Received(UplinkedReceived::from_http_uplink(
                &data,
            )))
            .await
            .unwrap();
    }

    Ok(Response::new(Body::from("success")))
}

impl Server {
    pub async fn new(prometheus: Option<Sender<prometheus::Message>>) -> Server {
        let (sender, receiver) = mpsc::channel(100);

        let sender_mutex = &mut *SENDER.lock().await;
        *sender_mutex = Some(sender.clone());

        Server {
            receiver,
            sender,
            prometheus,
        }
    }

    pub fn get_sender(&self) -> Sender<UplinkMessage> {
        self.sender.clone()
    }

    pub async fn run(mut self, port: u16) -> Result<(), hyper::Error> {
        let addr = ([0, 0, 0, 0], port).into();
        println!("Listening on http://{}", addr);

        tokio::spawn(async move {
            let mut prometheus = self.prometheus;

            // we will use (AppEui,DevEui,FCnt) to track expected events
            // either Uplink occurs or Timeout occurs and fetches
            // the tracked expected event first
            let mut expected_tracker = HashMap::new();

            loop {
                let event = self.receiver.recv().await.unwrap();
                match event {
                    UplinkMessage::Expect(expected) => {
                        // track this expected event
                        expected_tracker.insert(expected.hash_key(), expected.clone());
                        let mut sender = self.sender.clone();
                        tokio::spawn(async move {
                            delay_for(Duration::from_secs(5)).await;
                            sender
                                .send(UplinkMessage::ExpectTimeout(expected.clone()))
                                .await
                                .unwrap();
                        });
                    }
                    UplinkMessage::ExpectTimeout(timeout) => {
                        // if hashmap returns item, it still exists
                        if let Some(expected) = expected_tracker.remove(&timeout.hash_key()) {
                            if let Some(sender) = &mut prometheus {
                                sender
                                    .send(prometheus::Message::Stat(
                                        expected.device,
                                        Stat::HttpUplinkTimeout,
                                    ))
                                    .await
                                    .unwrap();
                            }
                        }
                    }
                    UplinkMessage::Received(received) => {
                        // if hashmap returns item, timeout has not fired yet
                        if let Some(expected) = expected_tracker.remove(&received.hash_key()) {
                            if let Some(sender) = &mut prometheus {
                                let time = received.t - expected.t;

                                sender
                                    .send(prometheus::Message::Stat(
                                        expected.device,
                                        Stat::HttpUplink(time as u64),
                                    ))
                                    .await
                                    .unwrap();
                            }
                        }
                    }
                }
            }
        });

        // For every connection, we must make a `Service` to handle all
        // incoming HTTP requests on said connection.
        let make_svc = make_service_fn(|_conn| {
            // This is the `Service` that will handle the connection.
            // `service_fn` is a helper to convert a function that
            // returns a Response into a `Service`.
            async { Ok::<_, Infallible>(service_fn(data_received)) }
        });

        let serve_future = HyperServer::bind(&addr).serve(make_svc);

        serve_future.await
    }
}

#[derive(Debug)]
pub enum DownlinkMessage {
    Expect(DownlinkRecord),
    ExpectTimeout(DownlinkRecord),
    Received(DownlinkRecord),
}

#[derive(Debug, Clone)]
pub struct DownlinkExpect {
    device: config::Device,
    t: u128,
    payload: Vec<u8>,
    fport: u8,
    fcnt: u32,
}

pub struct Downlinker {
    device: config::Device,
    url: String,
    sender: Sender<DownlinkMessage>,
    receiver: Receiver<DownlinkMessage>,
    prometheus: Option<Sender<prometheus::Message>>,
}

#[derive(Serialize, Debug, Clone)]
pub struct DownlinkRequest {
    payload_raw: String,
    port: u8,
    confirmed: bool,
}

#[derive(Debug, Clone)]
pub struct DownlinkRecord {
    t: u128,
    request: DownlinkRequest,
}

use lorawan_encoding::parser::{DataHeader, DecryptedDataPayload, FRMPayload};
use std::convert::AsRef;

impl DownlinkRecord {
    pub fn from_request(req: &DownlinkRequest) -> DownlinkRecord {
        DownlinkRecord {
            t: INSTANT.elapsed().as_millis(),
            request: req.clone(),
        }
    }

    pub fn from_decrypted_data_payload<T: AsRef<[u8]>>(
        payload: &DecryptedDataPayload<T>,
    ) -> DownlinkRecord {
        let port = if let Some(port) = payload.f_port() {
            port
        } else {
            0
        };

        DownlinkRecord {
            t: INSTANT.elapsed().as_millis(),
            request: DownlinkRequest {
                payload_raw: if let Ok(FRMPayload::Data(data)) = &payload.frm_payload() {
                    base64::encode(data)
                } else {
                    "".to_string()
                },
                port,
                confirmed: false,
            },
        }
    }

    fn hash_key(&self) -> (String, u8) {
        (self.request.payload_raw.clone(), self.request.port)
    }
}

fn generate_random_payload() -> String {
    let data = [
        rand::random(),
        rand::random(),
        rand::random(),
        rand::random(),
        rand::random(),
        rand::random(),
        rand::random(),
        rand::random(),
        rand::random(),
        rand::random(),
    ];
    base64::encode(data)
}

fn generate_random_port() -> u8 {
    let mut port = rand::random();
    while port == 0 {
        port = rand::random();
    }
    port
}

impl Downlinker {
    pub fn new(
        device: config::Device,
        url: &str,
        prometheus: Option<Sender<prometheus::Message>>,
    ) -> Downlinker {
        let (sender, receiver) = mpsc::channel(100);

        Downlinker {
            device,
            url: url.to_string(),
            sender,
            receiver,
            prometheus,
        }
    }

    pub fn get_sender(&self) -> Sender<DownlinkMessage> {
        self.sender.clone()
    }

    pub async fn run(mut self) {
        let mut self_sender = self.get_sender();
        let url = self.url;

        // give a delay for the Join to happen
        delay_for(Duration::from_secs(10)).await;

        tokio::spawn(async move {
            loop {
                delay_for(Duration::from_secs(10)).await;
                // send a downlink
                let downlink = DownlinkRequest {
                    payload_raw: generate_random_payload(),
                    port: generate_random_port(),
                    confirmed: false,
                };

                let client = reqwest::Client::new();
                let request = client
                    .post(url.as_str())
                    .header("Content-Type", "application/json")
                    .json(&downlink);

                let response = request.send().await.unwrap();

                if response.status() != 200 {
                    panic!("Bad response status {}", response.status());
                } else {
                    self_sender
                        .send(DownlinkMessage::Expect(DownlinkRecord::from_request(
                            &downlink,
                        )))
                        .await
                        .unwrap();
                }
            }
        });

        let mut prometheus = self.prometheus;

        // we will use (AppEui,DevEui,FCnt) to track expected events
        // either Uplink occurs or Timeout occurs and fetches
        // the tracked expected event first
        let mut expected_tracker = HashMap::new();
        let device = self.device;

        loop {
            let event = self.receiver.recv().await.unwrap();
            match event {
                DownlinkMessage::Expect(expected) => {
                    // track this expected event
                    expected_tracker.insert(expected.hash_key(), expected.clone());
                    let mut sender = self.sender.clone();
                    tokio::spawn(async move {
                        delay_for(Duration::from_secs(5)).await;
                        sender
                            .send(DownlinkMessage::ExpectTimeout(expected.clone()))
                            .await
                            .unwrap();
                    });
                }
                DownlinkMessage::ExpectTimeout(timeout) => {
                    // if hashmap returns item, it still exists
                    if let Some(_expected) = expected_tracker.remove(&timeout.hash_key()) {
                        if let Some(sender) = &mut prometheus {
                            sender
                                .send(prometheus::Message::Stat(
                                    device.clone(),
                                    Stat::HttpDownlinkTimeout,
                                ))
                                .await
                                .unwrap();
                        }
                    }
                }
                DownlinkMessage::Received(received) => {
                    // if hashmap returns item, timeout has not fired yet
                    if let Some(expected) = expected_tracker.remove(&received.hash_key()) {
                        if let Some(sender) = &mut prometheus {
                            let time = received.t - expected.t;

                            sender
                                .send(prometheus::Message::Stat(
                                    device.clone(),
                                    Stat::HttpDownlink(time as u64),
                                ))
                                .await
                                .unwrap();
                        }
                    }
                }
            }
        }
    }
}
