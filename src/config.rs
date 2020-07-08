use helium_console as console;
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
    pub fn from_console_device(oui: usize, device: console::Device) -> Device {
        Device {
            transmit_delay: 500,
            oui,
            credentials: Credentials {
                app_eui: device.app_eui().to_string(),
                app_key: device.app_key().to_string(),
                dev_eui: device.dev_eui().to_string(),
            },
        }
    }
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
        panic!("No {} found", path);
    }

    let contents = fs::read_to_string(path)?;
    let config: Config = serde_json::from_str(&contents)?;
    Ok(config)
}

#[derive(Deserialize, Serialize)]
struct ConsoleCredentials {
    pub staging: Option<String>,
    pub production: Option<String>,
}

pub struct ConsoleClients {
    pub staging: Option<console::client::Client>,
    pub production: Option<console::client::Client>,
}

fn key_verify(key: &str) -> Result<(), Box<dyn std::error::Error>> {
    let key_verify = base64::decode(key)?;
    if key_verify.len() != 32 {
        panic!("Invalid API key ipnut");
    }
    Ok(())
}

pub fn load_console_client(path: &str) -> Result<ConsoleClients, Box<dyn std::error::Error>> {
    if !Path::new(path).exists() {
        panic!("No {} found", path);
    }

    let contents = fs::read_to_string(path)?;
    let creds: ConsoleCredentials = serde_json::from_str(&contents)?;

    Ok(ConsoleClients {
        staging: if let Some(key) = creds.staging {
            key_verify(&key)?;
            Some(console::client::Client::new(
                console::client::Config::new_with_url(key, "https://staging-console.helium.com"),
            )?)
        } else {
            None
        },
        production: if let Some(key) = creds.production {
            key_verify(&key)?;
            Some(console::client::Client::new(console::client::Config::new(
                key,
            ))?)
        } else {
            None
        },
    })
}
