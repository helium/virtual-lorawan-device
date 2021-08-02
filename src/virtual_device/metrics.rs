use prometheus::Counter;
use prometheus::{labels, opts, register_counter};

pub struct Metrics {
    pub join_success_counter: Counter,
    pub join_fail_counter: Counter,
}

impl Metrics {
    pub fn new(oui: &str, device_dev_eui: &str) -> Metrics {
        Metrics {
            join_success_counter: register_counter!(opts!(
                "join_success",
                "join success counter",
                labels! {"oui" => oui,
                "dev_eui" => device_dev_eui}
            ))
            .unwrap(),
            join_fail_counter: register_counter!(opts!(
                "join_fail",
                "join fail counter",
                labels! {"oui" => oui,
                "dev_eui" => device_dev_eui}
            ))
            .unwrap(),
        }
    }
}
