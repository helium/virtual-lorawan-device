use hyper::{self, body};
use serde::{Deserialize, Serialize};
use std::str;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::delay_for;

/// The default base URL if none is specified.
pub const DEFAULT_BASE_URL: &str = "https://api.helium.io/v1";
//pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080/v1";

const DEFAULT_ROUTERS: [&str; 2] = [
    "112qB3YaH5bZkCnKA5uRH7tBtGNv2Y5B4smv1jsmvGUzgKT71QpE",
    "1124CJ9yJaHq4D6ugyPCDnSBzQik61C1BqD9VMh1vsUmjwt16HNB",
];

async fn fetch(path: &str) -> std::result::Result<body::Bytes, Box<dyn std::error::Error>> {
    let request_url = format!("{}{}", DEFAULT_BASE_URL, path);
    let https = hyper_tls::HttpsConnector::new();
    let client = hyper::Client::builder().build::<_, hyper::Body>(https);
    let res = client.get(request_url.parse().unwrap()).await?;
    // Concatenate the body stream into a single buffer...
    let body = hyper::body::to_bytes(res).await?;

    Ok(body)
}

// must allow camel case to use json type tag
#[allow(non_camel_case_types)]
#[derive(Deserialize, Serialize, Debug)]
#[serde(tag = "type")]
pub enum Data {
    state_channel_open_v1(OpenStateChannel),
    state_channel_close_v1(ClosedStateChannel),
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
    expire_within: usize,
}

pub async fn wait_for_new_block(
    cur_height: usize,
) -> std::result::Result<usize, Box<dyn std::error::Error>> {
    println!("Waiting for new block, current height {}", cur_height);

    // fetching state channels is expensive, so hits block height until change
    let mut new_block_height = fetch_block_height().await?;
    while new_block_height == cur_height {
        delay_for(Duration::from_millis(1000)).await;
        new_block_height = fetch_block_height().await?;
    }
    println!();
    Ok(new_block_height)
}

use super::udp_radio;
//
// pub async fn signal_at_block_height<'a>(
//     threshold: isize,
//     mut sender: mpsc::Sender<udp_radio::Event<'a>>,
// ) -> std::result::Result<(), Box<dyn std::error::Error>> {
//     let mut cur_height = fetch_block_height().await? as isize;
//     println!(
//         "Will signal at {}. Currently at {}. {} more blocks to go",
//         threshold,
//         cur_height,
//         threshold - cur_height
//     );
//     while cur_height < threshold {
//         let new_height = fetch_block_height().await? as isize;
//         if new_height != cur_height {
//             cur_height = new_height;
//         };
//         delay_for(Duration::from_millis(1000)).await;
//     }
//     sender.send(udp_radio::Event::Shutdown).await?;
//     Ok(())
// }

impl OpenStateChannel {
    pub fn close_height(&self) -> usize {
        self.height + self.expire_within
    }

    pub async fn remaining_blocks(
        &self,
    ) -> std::result::Result<(usize, isize), Box<dyn std::error::Error>> {
        let cur_height = fetch_block_height().await?;
        let remaining_blocks = self.close_height() as isize - cur_height as isize;
        Ok((cur_height, remaining_blocks))
    }

    pub async fn block_until_closed_transaction(
        &self,
        oui: usize,
    ) -> std::result::Result<ClosedStateChannel, Box<dyn std::error::Error>> {
        let mut block_height = fetch_block_height().await?;

        loop {
            let state_channels = fetch_recent_state_channels(oui).await?;
            for txn in state_channels {
                if let Data::state_channel_close_v1(close_channel) = txn {
                    if close_channel.state_channel.id == self.id {
                        return Ok(close_channel);
                    }
                }
            }
            block_height = wait_for_new_block(block_height).await?;
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ClosedStateChannel {
    time: usize,
    state_channel: StateChannel,
    height: usize,
    hash: String,
    closer: String,
}

impl ClosedStateChannel {
    pub fn summaries(&self) -> &Vec<Summaries> {
        &self.state_channel.summaries
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct StateChannel {
    summaries: Vec<Summaries>,
    state: String,
    root_hash: String,
    owner: String,
    nonce: usize,
    id: String,
    expire_at_block: usize,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Summaries {
    owner: String,
    num_packets: usize,
    num_dcs: usize,
    location: String,
    pub client: String,
}

impl Summaries {
    pub fn client(&self) -> &String {
        &self.client
    }
}

pub async fn fetch_block_height() -> std::result::Result<usize, Box<dyn std::error::Error>> {
    #[derive(Deserialize, Serialize, Debug)]
    struct Response {
        data: Data,
    }
    #[derive(Deserialize, Serialize, Debug)]
    struct Data {
        height: usize,
    }
    let body = fetch("/blocks/height").await?;
    let data_as_str = str::from_utf8(&body)?;
    let response: Response = serde_json::from_str(&data_as_str)?;

    Ok(response.data.height)
}

async fn fetch_recent_state_channels(
    oui: usize,
) -> std::result::Result<Vec<Data>, Box<dyn std::error::Error>> {
    if oui == 0 || oui > 2 {
        panic!("Invalid OUI");
    }

    #[derive(Deserialize, Serialize, Debug)]
    pub struct StateChannelsResp {
        meta: Meta,
        data: Vec<Data>,
        cursor: Option<String>,
    }

    let body = fetch(format!("/accounts/{}/activity", DEFAULT_ROUTERS[oui - 1]).as_str()).await?;
    let data_as_str = str::from_utf8(&body)?;
    let response: StateChannelsResp = serde_json::from_str(&data_as_str)?;
    Ok(response.data)
}

pub async fn fetch_open_channel(
    oui: usize,
) -> std::result::Result<OpenStateChannel, Box<dyn std::error::Error>> {
    let state_channels = fetch_recent_state_channels(oui).await?;
    // initial sort
    let mut close = Vec::new();
    let mut open = Vec::new();
    for state_channel in state_channels {
        match state_channel {
            Data::state_channel_open_v1(open_channel) => {
                open.push(open_channel);
            }
            Data::state_channel_close_v1(close_channel) => {
                close.push(close_channel);
            }
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

    assert!(still_open.len() <= 2);

    Ok(if still_open.len() == 2 {
        let open = still_open.pop().unwrap();
        let pending = still_open.pop().unwrap();
        // we've assumed that they're ordered in a way so we can pop to get open and pending
        // if we're wrong, the open one won't be 1 nonce less than the pending one
        assert_eq!(open.nonce + 1, pending.nonce);
        open
    } else {
        still_open.pop().unwrap()
    })
}
