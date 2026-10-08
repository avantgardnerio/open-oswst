//! What the server answers radios: who gets in (radios.toml, keys.rs), and
//! one arm per request (core's management.rs has them all)

use crate::firmware::{short_hex, Firmware};
use crate::keys::{self, Radio};
use crate::service::Service;
use open_oswst_core::management::{
    Key, Offer, Request, Response, CHUNK, IMAGE_CHANGED, NOT_ALLOWED,
};
use std::io;
use std::path::PathBuf;

pub struct Oswst {
    pub radios: PathBuf, // radios.toml
    pub firmware: Firmware,
}

impl Service for Oswst {
    type Request = Request;
    type Response = Response;
    type Peer = Radio;

    fn admit(&self, key: &Key) -> Result<Radio, String> {
        keys::find_radio(&self.radios, key)
    }

    fn handle(&self, radio: &Radio, request: Request) -> Response {
        match request {
            Request::Echo(bytes) => Response::Echo(bytes),
            Request::Hello {
                running_sha256,
                version,
                ..
            } => {
                println!(
                    "{} runs {} ({})",
                    radio,
                    version,
                    short_hex(&running_sha256)
                );
                self.offer(running_sha256)
            }
            Request::Chunk {
                sha256,
                offset,
                len,
            } => self.chunk(sha256, offset, len),
        }
    }

    fn refused(&self) -> Response {
        Response::Error(NOT_ALLOWED.into())
    }

    fn unreadable(&self, error: &io::Error) -> Response {
        Response::Error(format!("unreadable request: {}", error))
    }
}

impl Oswst {
    /// Our image, unless the radio runs it already
    fn offer(&self, running: [u8; 32]) -> Response {
        match self.firmware.image() {
            Ok(image) if image.sha256 == running => Response::UpToDate,
            Ok(image) => Response::Offer(Offer {
                version: image.version.clone(),
                size: image.bytes.len() as u32,
                sha256: image.sha256,
            }),
            Err(e) => Response::Error(e),
        }
    }

    fn chunk(&self, sha256: [u8; 32], offset: u32, len: u16) -> Response {
        let image = match self.firmware.image() {
            Ok(image) => image,
            Err(e) => return Response::Error(e),
        };
        if image.sha256 != sha256 {
            return Response::Error(IMAGE_CHANGED.into());
        }
        let start = offset as usize;
        let end = start + len.min(CHUNK) as usize;
        match image.bytes.get(start..end) {
            Some(bytes) => Response::Chunk(bytes.to_vec()),
            None => Response::Error("Chunk past the end".into()),
        }
    }
}
