use hyper::{self, body};
use hyper_tls;
use serde_derive;
use serde_json;
use serde::{Serialize, Deserialize};
use std::str;

/// The default timeout for API requests
pub const DEFAULT_TIMEOUT: u64 = 120;
/// The default base URL if none is specified.
pub const DEFAULT_BASE_URL: &str = "https://api.helium.io/v1";
//pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080/v1";

const DEFAULT_ROUTERS: [&str; 2] = [
    "112qB3YaH5bZkCnKA5uRH7tBtGNv2Y5B4smv1jsmvGUzgKT71QpE",
    "1124CJ9yJaHq4D6ugyPCDnSBzQik61C1BqD9VMh1vsUmjwt16HNB"
];

async fn fetch(path: &str) -> std::result::Result<body::Bytes, Box<dyn std::error::Error>>{
    let request_url = format!("{}{}", DEFAULT_BASE_URL, path);
    let https = hyper_tls::HttpsConnector::new();
    let client = hyper::Client::builder().build::<_, hyper::Body>(https);
    let res = client.get(request_url.parse().unwrap()).await?;
    // Concatenate the body stream into a single buffer...
    let body = hyper::body::to_bytes(res).await?;

    Ok(body)
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(tag = "type")]
pub enum Data {
    state_channel_open_v1(OpenStateChannel),
    state_channel_close_v1(ClosedStateChannel)
}


#[derive(Deserialize, Serialize, Debug)]
struct Meta {
    start_block: usize,
    end_block: usize,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct OpenStateChannel {
    time: usize,
    owner: String,
    oui: usize,
    nonce: usize,
    id: String,
    height: usize,
    hash: String,
    fee: usize,
    expire_within: usize
}

impl OpenStateChannel {
    pub fn remaining_blocks(&self, block_height: usize ) -> usize {
        self.height + self.expire_within - block_height
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct ClosedStateChannel {
    time: usize,
    state_channel: StateChannel,
    height: usize,
    hash: String,
    closer: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct StateChannel {
    summaries: Vec<Summaries>,
    state: String,
    root_hash: String,
    owner: String,
    nonce: usize,
    id: String,
    expire_at_block: usize
}

#[derive(Deserialize, Serialize, Debug)]
struct Summaries {
    owner: String,
    num_packets: usize,
    num_dcs: usize,
    location: String,
    client: String,
}

pub async fn fetch_block_height() -> std::result::Result<usize, Box<dyn std::error::Error>> {
    #[derive(Deserialize, Serialize, Debug)]
    struct Response {
        data: Data
    }
    #[derive(Deserialize, Serialize, Debug)]
    struct Data {
        height: usize
    }


    let body = fetch("/blocks/height").await?;

    let data_as_str = str::from_utf8(&body)?;
    let response: Response = serde_json::from_str(&data_as_str)?;


    Ok(response.data.height)
}

pub async fn fetch_open_channels() -> std::result::Result<Vec<OpenStateChannel>, Box<dyn std::error::Error>>{
    #[derive(Deserialize, Serialize, Debug)]
    pub struct StateChannelsResp {
        meta: Meta,
        data: Vec<Data>,
        cursor: Option<String>
    }

    let body = fetch("/accounts/112qB3YaH5bZkCnKA5uRH7tBtGNv2Y5B4smv1jsmvGUzgKT71QpE/activity").await?;
    let data_as_str = str::from_utf8(&body)?;
    let mut response: StateChannelsResp = serde_json::from_str(&data_as_str)?;

    let state_channels = response.data;

    // initial sort
    let mut close = Vec::new();
    let mut open = Vec::new();
    for state_channel in state_channels {
        match state_channel {
            Data::state_channel_open_v1(open_channel) => {
                open.push(open_channel);
            },
            Data::state_channel_close_v1(close_channel) => {
                close.push(close_channel);
            },

        }
    }

    let mut still_open = Vec::new();
    let len_close = close.len();
    // remove the open channels that have matching closed
    for open_channel in open {
        for (index, closed_channel) in close.iter().enumerate() {
            if closed_channel.state_channel.id == open_channel.id {
                break;
            }
            if index == len_close - 1 {
                still_open.push(open_channel);
                break;
            }
        }
    }

    // only necessary if we want historical
    // while let Some(cursor) = response.cursor {
    //     let body = fetch(format!("/accounts/112qB3YaH5bZkCnKA5uRH7tBtGNv2Y5B4smv1jsmvGUzgKT71QpE/activity?cursor={}", cursor).as_str()).await?;
    //     let data_as_str = str::from_utf8(&body)?;
    //     response = serde_json::from_str(&data_as_str)?;
    //     state_channels.append(&mut response.data);
    // }

    Ok(still_open)

}