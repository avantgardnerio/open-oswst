//! What the server answers radios: who gets in (radios.toml, keys.rs), and
//! one arm per request (core's management.rs has them all)

use crate::keys::{self, Radio};
use crate::service::Service;
use open_oswst_core::management::{Key, Request, Response, NOT_ALLOWED};
use std::io;
use std::path::PathBuf;

pub struct Oswst {
    pub radios: PathBuf, // radios.toml
}

impl Service for Oswst {
    type Request = Request;
    type Response = Response;
    type Peer = Radio;

    fn admit(&self, key: &Key) -> Result<Radio, String> {
        keys::find_radio(&self.radios, key)
    }

    fn handle(&self, _radio: &Radio, request: Request) -> Response {
        match request {
            Request::Echo(bytes) => Response::Echo(bytes),
        }
    }

    fn refused(&self) -> Response {
        Response::Error(NOT_ALLOWED.into())
    }

    fn unreadable(&self, error: &io::Error) -> Response {
        Response::Error(format!("unreadable request: {}", error))
    }
}
