use super::{debugln, INSTANT, prometheus_service as prometheus};
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server as HyperServer,
};
use serde_derive::{Deserialize, Serialize};
use std::{convert::Infallible, str, sync::Mutex};
pub use tokio::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug)]
pub enum Message {
    ExpectUplink(ExpectUplink),
    ExpectTimeout(ExpectUplink),
    ReceivedUplink(ReceivedUplink),
}

#[derive(Debug)]
pub struct ExpectUplink {
    t: u128,
    payload: Vec<u8>,
    fport: u8,
    fcnt: u32,
}

impl ExpectUplink {
    pub fn new(t: u128, input_payload: &[u8], fport: u8, fcnt: u32) -> ExpectUplink {
        let mut payload = Vec::new();
        payload.extend(input_payload);
        ExpectUplink {
            t,
            payload,
            fport,
            fcnt
        }
    }
}

#[derive(Debug)]
pub struct ReceivedUplink {
    t: u64,
    fcnt: u32,
}

#[derive(Debug)]
pub struct Server {
    receiver: Receiver<Message>,
    sender: Sender<Message>,
    prometheus: Option<Sender<prometheus::Message>>,
}

#[derive(Deserialize, Debug)]
struct DataIn {
    app_eui: String,
    dev_eui: String,
    devaddr: String,
    fcnt: usize,
    payload: String,
}

#[derive(Serialize, Debug)]
struct Downlink {
    payload_raw: String,
    port: usize,
    confirmed: bool,
}


lazy_static! {
    // this sender allows the data_received function to dispatch messages
    // to the task which compares Expected and Received messages
    static ref SENDER: Mutex<Option<Sender<Message >>> = Mutex::new(None);
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

    Ok(Response::new(Body::from("success")))
}

impl Server {
    pub fn new(prometheus: Option<Sender<prometheus::Message>>) -> Server {
        let (sender, receiver) = mpsc::channel(100);
        unsafe {
            let sender_mutex = &mut *SENDER.lock().unwrap();
            *sender_mutex = Some(sender.clone());
        }
        Server {
            receiver,
            sender,
            prometheus,
        }
    }

    pub fn get_sender(&self) -> Sender<Message> {
        self.sender.clone()
    }

    pub async fn run(mut self, port: u16) -> Result<(), hyper::Error> {
        let addr = ([0, 0, 0, 0], port).into();
        println!("Listening on http://{}", addr);

        tokio::spawn(async move {
            loop {
                let event = self.receiver.recv().await.unwrap();
                println!("event! {:?}", event);
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
