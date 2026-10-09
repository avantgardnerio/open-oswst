//! What the server answers radios: who gets in (radios.toml, keys.rs), and
//! one arm per request (core's management.rs has them all)

use crate::bundle::{short_hex, Published};
use crate::keys::{self, Radio};
use crate::service::Service;
use open_oswst_core::management::{
    Key, Offer, Request, Response, Sha256, CHUNK, IMAGE_CHANGED, NOT_ALLOWED,
};
use std::io;
use std::path::PathBuf;

pub struct Oswst {
    pub radios: PathBuf, // radios.toml
    pub published: Published,
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
            // Firmware from before bundles: the firmware alone
            Request::Hello {
                running_sha256,
                version,
                ..
            } => {
                println!(
                    "{} runs {} ({}), asks for firmware",
                    radio,
                    version,
                    short_hex(&running_sha256)
                );
                self.offer_firmware(running_sha256)
            }
            Request::Chunk {
                sha256,
                offset,
                len,
            } => self.firmware_chunk(sha256, offset, len),
            Request::CheckBundle {
                running_sha256,
                installed_sha256,
                version,
                ..
            } => {
                println!(
                    "{} runs {} ({}), installed bundle {}",
                    radio,
                    version,
                    short_hex(&running_sha256),
                    installed_sha256.as_ref().map_or("none".into(), short_hex)
                );
                self.offer_bundle(running_sha256, installed_sha256)
            }
            Request::BundleChunk {
                sha256,
                offset,
                len,
            } => self.bundle_chunk(sha256, offset, len),
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
    /// The bundle's firmware, unless the radio runs it already
    fn offer_firmware(&self, running: Sha256) -> Response {
        match self.published.bundle() {
            Ok(bundle) if bundle.firmware.sha256 == running => Response::UpToDate,
            Ok(bundle) => Response::Offer(Offer {
                version: bundle.firmware.version.clone(),
                size: bundle.firmware.bytes.len() as u32,
                sha256: bundle.firmware.sha256,
            }),
            Err(e) => Response::Error(e),
        }
    }

    fn firmware_chunk(&self, sha256: Sha256, offset: u32, len: u16) -> Response {
        match self.published.bundle() {
            Ok(bundle) if bundle.firmware.sha256 == sha256 => {
                chunk(&bundle.firmware.bytes, offset, len)
            }
            Ok(_) => Response::Error(IMAGE_CHANGED.into()),
            Err(e) => Response::Error(e),
        }
    }

    /// The bundle, unless the radio installed it last and still runs its
    /// firmware (a rollback, or a USB flash since, and it's offered again)
    fn offer_bundle(&self, running: Sha256, installed: Option<Sha256>) -> Response {
        match self.published.bundle() {
            Ok(bundle) if installed == Some(bundle.sha256) && bundle.firmware.sha256 == running => {
                Response::UpToDate
            }
            Ok(bundle) => Response::BundleOffer(Offer {
                version: bundle.firmware.version.clone(),
                size: bundle.bytes.len() as u32,
                sha256: bundle.sha256,
            }),
            Err(e) => Response::Error(e),
        }
    }

    fn bundle_chunk(&self, sha256: Sha256, offset: u32, len: u16) -> Response {
        match self.published.bundle() {
            Ok(bundle) if bundle.sha256 == sha256 => chunk(&bundle.bytes, offset, len),
            Ok(_) => Response::Error(IMAGE_CHANGED.into()),
            Err(e) => Response::Error(e),
        }
    }
}

/// `len` bytes of `bytes` from `offset`, at most CHUNK
fn chunk(bytes: &[u8], offset: u32, len: u16) -> Response {
    let start = offset as usize;
    let end = start + len.min(CHUNK) as usize;
    match bytes.get(start..end) {
        Some(bytes) => Response::Chunk(bytes.to_vec()),
        None => Response::Error("Chunk past the end".into()),
    }
}
