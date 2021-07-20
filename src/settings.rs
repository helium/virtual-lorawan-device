use super::{PathBuf, Result};
use config::{Config, File};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Debug)]
pub struct Settings {
    pub host: String,
    pub devices: HashMap<String, Device>,
}

impl Settings {
    /// Load Settings from a given path. Settings are loaded from a default.toml
    /// file in the given path, followed by merging in an optional settings.toml
    /// in the same folder.
    pub fn new(path: &PathBuf) -> Result<Settings> {
        let mut c = Config::new();
        let default_file = path.join("default.toml");
        // Load default config and merge in overrides
        c.merge(File::with_name(default_file.to_str().expect("file name")))?;
        let settings_file = path.join("settings.toml");
        if settings_file.exists() {
            c.merge(File::with_name(settings_file.to_str().expect("file name")))?;
        }
        c.try_into().map_err(|e| e.into())
    }
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Device {
    mac: String,
    pub credentials: Credentials,
}

impl Device {
    pub fn mac_cloned_into_buf(&self) -> Result<[u8; 8]> {
        let vec = hex::decode(&self.mac)?;
        Ok([
            vec[7], vec[6], vec[5], vec[4], vec[3], vec[2], vec[1], vec[0],
        ])
    }
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Credentials {
    pub app_eui: String,
    pub app_key: String,
    pub dev_eui: String,
}

impl Credentials {
    pub fn appeui_cloned_into_buf(&self) -> Result<[u8; 8]> {
        let vec = hex::decode(&self.app_eui)?;
        Ok([
            vec[7], vec[6], vec[5], vec[4], vec[3], vec[2], vec[1], vec[0],
        ])
    }

    pub fn deveui_cloned_into_buf(&self) -> Result<[u8; 8]> {
        let vec = hex::decode(&self.dev_eui)?;
        Ok([
            vec[7], vec[6], vec[5], vec[4], vec[3], vec[2], vec[1], vec[0],
        ])
    }
    pub fn appkey_cloned_into_buf(&self) -> Result<[u8; 16]> {
        let vec = hex::decode(&self.app_key)?;
        Ok([
            vec[0], vec[1], vec[2], vec[3], vec[4], vec[5], vec[6], vec[7], vec[8], vec[9],
            vec[10], vec[11], vec[12], vec[13], vec[14], vec[15],
        ])
    }
}
