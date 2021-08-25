use super::*;
use hyper::{header::CONTENT_TYPE, Body, Request, Response};
use log::{debug, warn};
use prometheus::{labels, opts, register_counter, register_histogram_vec};
use prometheus::{Counter, HistogramVec};
use prometheus::{Encoder, TextEncoder};
use tokio::sync::mpsc;

pub type Sender = mpsc::Sender<Message>;

pub enum Message {
    JoinSuccess(i64),
    JoinFail,
    DataSuccess(i64),
    DataFail,
}

pub struct Metrics {
    pub oui: String,
    pub join_success_counter: Counter,
    pub join_fail_counter: Counter,
    pub data_success_counter: Counter,
    pub data_fail_counter: Counter,
    pub join_latency: HistogramVec,
    pub data_latency: HistogramVec,
}

impl Metrics {
    pub fn run(oui: &str) -> Sender {
        let (tx, mut rx) = mpsc::channel(1024);
        let metrics = Metrics {
            oui: oui.to_string(),
            join_success_counter: register_counter!(opts!(
                "join_success",
                "join success counter",
                labels! {"oui" => oui}
            ))
            .unwrap(),
            join_fail_counter: register_counter!(opts!(
                "join_fail",
                "join fail counter",
                labels! {"oui" => oui}
            ))
            .unwrap(),
            data_success_counter: register_counter!(opts!(
                "data_success",
                "data success counter",
                labels! {"oui" => oui}
            ))
            .unwrap(),
            data_fail_counter: register_counter!(opts!(
                "data_fail",
                "data fail counter",
                labels! {"oui" => oui}
            ))
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
                    Some(Message::JoinSuccess(time_remaining)) => {
                        let time_remaining_seconds = (time_remaining as f64) / 1000000.0;
                        metrics
                            .join_latency
                            .with_label_values(&["1"])
                            .observe(time_remaining_seconds);
                        metrics.join_success_counter.inc();
                    }
                    Some(Message::JoinFail) => metrics.join_fail_counter.inc(),

                    Some(Message::DataSuccess(time_remaining)) => {
                        let time_remaining_seconds = (time_remaining as f64) / 1000000.0;
                        metrics
                            .data_latency
                            .with_label_values(&["1"])
                            .observe(time_remaining_seconds);
                        metrics.data_success_counter.inc();
                    }
                    Some(Message::DataFail) => metrics.data_fail_counter.inc(),
                    None => warn!("Metrics receive channel returned None. Is closed?"),
                }
            }
        });
        tx
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
