use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server as HyperServer
};
use serde_derive::{Deserialize, Serialize};
use std::{
    str,
    convert::Infallible
};

pub struct Server;


#[derive(Deserialize, Debug)]
struct DataIn {
    app_eui: String,
    dev_eui: String,
    app_key: Option<String>,
    devaddr: String,
    fcnt: usize,
    payload: String,
}

#[derive(Serialize, Debug)]
struct Downlink {
    payload_raw: String,
    port: usize,
    confirmed: bool,
}

async fn data_received(request: Request<Body>) -> Result<Response<Body>, Infallible> {
    let body = hyper::body::to_bytes(request).await.unwrap();
    let data: DataIn = serde_json::from_slice(&body).unwrap();
    println!("{:?}", data);

    let downlink = Downlink {
        payload_raw: data.payload.clone(),
        port: 3,
        confirmed: false,
    };

    Ok(Response::new(Body::from(serde_json::to_vec(&downlink).unwrap())))
}

impl Server {

    pub fn new() -> Server {
        Server {}
    }

    pub async fn run(self) -> Result<(), hyper::Error> {
        let addr = ([0, 0, 0, 0], 8888).into();
        println!("Listening on http://{}", addr);

        // For every connection, we must make a `Service` to handle all
        // incoming HTTP requests on said connection.
        let make_svc = make_service_fn(|_conn| {
            // This is the `Service` that will handle the connection.
            // `service_fn` is a helper to convert a function that
            // returns a Response into a `Service`.
            async { Ok::<_, Infallible>(service_fn(data_received)) }
        });

        let serve_future = HyperServer::bind(&addr).serve(make_svc);

        serve_future.await
    }
}