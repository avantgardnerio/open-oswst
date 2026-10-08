//! What the server answers radios: one arm per request (core's
//! management.rs has them all)

use crate::service::Service;
use open_oswst_core::management::{Request, Response};
use std::io;
use std::net::SocketAddr;

pub struct Oswst;

impl Service for Oswst {
    type Request = Request;
    type Response = Response;

    fn handle(&self, _peer: SocketAddr, request: Request) -> Response {
        match request {
            Request::Echo(bytes) => Response::Echo(bytes),
        }
    }

    fn unreadable(&self, error: &io::Error) -> Response {
        Response::Error(format!("unreadable request: {}", error))
    }
}
