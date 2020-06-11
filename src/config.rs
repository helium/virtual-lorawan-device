use serde_derive::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Credentials {
    app_eui: String,
    app_key: String,
    dev_eui: String,
}

impl Credentials {
    pub fn appeui_cloned_into_buf(&self) -> Result<[u8; 8], Box<dyn std::error::Error>> {
        let vec = hex::decode(&self.app_eui)?;
        Ok([
            vec[7], vec[6], vec[5], vec[4], vec[3], vec[2], vec[1], vec[0],
        ])
    }
    pub fn deveui_cloned_into_buf(&self) -> Result<[u8; 8], Box<dyn std::error::Error>> {
        let vec = hex::decode(&self.dev_eui)?;
        Ok([
            vec[7], vec[6], vec[5], vec[4], vec[3], vec[2], vec[1], vec[0],
        ])
    }
    pub fn appkey_cloned_into_buf(&self) -> Result<[u8; 16], Box<dyn std::error::Error>> {
        let vec = hex::decode(&self.app_key)?;
        Ok([
            vec[0], vec[1], vec[2], vec[3], vec[4], vec[5], vec[6], vec[7], vec[8], vec[9],
            vec[10], vec[11], vec[12], vec[13], vec[14], vec[15],
        ])
    }
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Device {
    transmit_delay: usize,
    oui: usize,
    credentials: Credentials,
}

impl Device {
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }
    pub fn transmit_delay(&self) -> u64 {
        self.transmit_delay as u64
    }
}

#[derive(Deserialize, Serialize)]
pub struct Config {
    pub devices: Vec<Device>,
    pub gateways: Option<Vec<String>>,
}

pub fn load_config(path: &str) -> Result<Config, Box<dyn std::error::Error>> {
    if !Path::new(path).exists() {
        panic!("No lorawan-devices.json found");
    }

    let contents = fs::read_to_string(path)?;
    let config: Config = serde_json::from_str(&contents)?;
    Ok(config)
}
