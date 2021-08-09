use prometheus::{labels, opts, register_counter, register_histogram_vec};
use prometheus::{Counter, HistogramVec};

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
    pub fn new(oui: &str ) -> Metrics {
        Metrics {
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
        }
    }
}
