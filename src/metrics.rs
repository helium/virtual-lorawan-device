use super::*;
use error::{Error, Result};
use hyper::{header::CONTENT_TYPE, Body, Request, Response};
use log::{debug, warn};
use prometheus::{register_counter_vec, register_histogram_vec};
use prometheus::{CounterVec, HistogramVec};
use prometheus::{Encoder, TextEncoder};
use tokio::sync::mpsc;

pub struct Sender {
    oui: String,
    sender: mpsc::Sender<InternalMessage>,
}

impl Sender {
    pub async fn send(&mut self, message: Message) -> Result<()> {
        let oui = self.oui.clone();
        match message {
            Message::JoinSuccess(t) => self.sender.send(InternalMessage::JoinSuccess(oui, t)).await,
            Message::JoinFail => self.sender.send(InternalMessage::JoinFail(oui)).await,
            Message::DataSuccess(t) => self.sender.send(InternalMessage::DataSuccess(oui, t)).await,
            Message::DataFail => self.sender.send(InternalMessage::DataFail(oui)).await,
        }
        .map_err(|_| Error::MetricsChannel)
    }
}

#[derive(Debug)]
pub enum Message {
    JoinSuccess(i64),
    JoinFail,
    DataSuccess(i64),
    DataFail,
}

pub struct Metrics {
    sender: mpsc::Sender<InternalMessage>,
}

#[derive(Debug)]
enum InternalMessage {
    JoinSuccess(String, i64),
    JoinFail(String),
    DataSuccess(String, i64),
    DataFail(String),
}

struct InternalMetrics {
    join_success_counter: CounterVec,
    join_fail_counter: CounterVec,
    data_success_counter: CounterVec,
    data_fail_counter: CounterVec,
    join_latency: HistogramVec,
    data_latency: HistogramVec,
}

impl Metrics {
    pub fn init() -> Metrics {
        let (sender, mut rx) = mpsc::channel(1024);

        let metrics = InternalMetrics {
            join_success_counter: register_counter_vec!(
                "join_success",
                "join success counter",
                &["oui"]
            )
            .unwrap(),
            join_fail_counter: register_counter_vec!("join_fail", "join fail counter", &["oui"])
                .unwrap(),
            data_success_counter: register_counter_vec!(
                "data_success",
                "data success counter",
                &["oui"]
            )
            .unwrap(),
            data_fail_counter: register_counter_vec!("data_fail", "data fail counter", &["oui"])
                .unwrap(),
            join_latency: register_histogram_vec!(
                "join_latency",
                "join latency histogram",
                &["oui"],
                vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5]
            )
            .unwrap(),
            data_latency: register_histogram_vec!(
                "data_latency",
                "data latency histogram",
                &["oui"],
                vec![0.01, 0.05, 0.1, 0.20, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]
            )
            .unwrap(),
        };

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Some(InternalMessage::JoinSuccess(label, t)) => {
                        let in_secs = (t as f64) / 1000000.0;
                        metrics
                            .join_latency
                            .with_label_values(&[&label])
                            .observe(in_secs);
                        metrics
                            .join_success_counter
                            .with_label_values(&[&label])
                            .inc();
                    }
                    Some(InternalMessage::JoinFail(label)) => {
                        metrics.join_fail_counter.with_label_values(&[&label]).inc()
                    }

                    Some(InternalMessage::DataSuccess(label, t)) => {
                        let in_secs = (t as f64) / 1000000.0;
                        metrics
                            .data_latency
                            .with_label_values(&[&label])
                            .observe(in_secs);
                        metrics
                            .data_success_counter
                            .with_label_values(&[&label])
                            .inc();
                    }
                    Some(InternalMessage::DataFail(label)) => {
                        metrics.data_fail_counter.with_label_values(&[&label]).inc()
                    }
                    None => warn!("Metrics receive channel returned None. Is closed?"),
                }
            }
        });
        Metrics { sender }
    }

    pub fn get_sender(&self, oui: &str) -> Sender {
        Sender {
            oui: oui.to_string(),
            sender: self.sender.clone(),
        }
    }

    pub async fn serve_req(_req: Request<Body>) -> Result<Response<Body>> {
        let encoder = TextEncoder::new();

        let metric_families = prometheus::gather();
        let mut buffer = vec![];
        let mut buffer_print = vec![];
        encoder.encode(&metric_families, &mut buffer).unwrap();
        encoder.encode(&metric_families, &mut buffer_print).unwrap();

        // Output current stats
        debug!("{}", String::from_utf8(buffer_print).unwrap());

        let response = Response::builder()
            .status(200)
            .header(CONTENT_TYPE, encoder.format_type())
            .body(Body::from(buffer))
            .unwrap();

        Ok(response)
    }
}
