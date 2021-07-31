use prometheus::Counter;
use prometheus::{labels, opts, register_counter};

pub struct Metrics {
    pub join_success_counter: Counter,
}

impl Metrics {
    pub fn new(device_dev_eui: &str) -> Metrics {
        Metrics {
            join_success_counter: register_counter!(opts!(
                format!("join_success"),
                format!("joine success total"),
                labels! {"dev_eui" => device_dev_eui}
            ))
            .unwrap(),
        }
    }
}
