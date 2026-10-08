//! Noise (the `snow` crate) on the crypto ESP-IDF already links for WiFi:
//! mbedTLS's X25519, AES-256-GCM on the AES hardware and SHA-256 on the SHA
//! hardware. `snow` brings only the handshake logic, so the management
//! server's encryption (issue #34) costs almost no flash. The suite is
//! Noise_XK_25519_AESGCM_SHA256; the server runs `snow`'s own resolver.

use esp_idf_svc::sys::*;
use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::resolvers::CryptoResolver;
use snow::types::{Cipher, Dh, Hash, Random};

pub const PATTERN: &str = "Noise_XK_25519_AESGCM_SHA256";

const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;

pub struct MbedtlsResolver;

impl CryptoResolver for MbedtlsResolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        Some(Box::new(HardwareRandom))
    }

    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        match choice {
            DHChoice::Curve25519 => Some(Box::new(X25519::default())),
            _ => None,
        }
    }

    fn resolve_hash(&self, choice: &HashChoice) -> Option<Box<dyn Hash>> {
        match choice {
            HashChoice::SHA256 => Some(Box::new(Sha256::new())),
            _ => None,
        }
    }

    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        match choice {
            CipherChoice::AESGCM => Some(Box::new(AesGcm::default())),
            _ => None,
        }
    }
}

/// The hardware RNG: truly random while WiFi or Bluetooth runs (the RF
/// subsystem feeds it), which it always does by the time we connect
struct HardwareRandom;

impl Random for HardwareRandom {
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), snow::Error> {
        unsafe { esp_fill_random(dest.as_mut_ptr().cast(), dest.len()) };
        Ok(())
    }
}

/// For mbedTLS's blinding during point multiplication
unsafe extern "C" fn mbedtls_random(
    _state: *mut core::ffi::c_void,
    output: *mut core::ffi::c_uchar,
    len: usize,
) -> core::ffi::c_int {
    esp_fill_random(output.cast(), len);
    0
}

/// X25519 (RFC 7748) on mbedTLS's Curve25519. Keys are 32 bytes, little
/// endian, as everyone else writes them
#[derive(Default)]
struct X25519 {
    private: [u8; KEY_LEN],
    public: [u8; KEY_LEN],
}

/// mbedTLS's curve, a point and a number, freed when dropped
struct Curve {
    group: mbedtls_ecp_group,
    point: mbedtls_ecp_point,
    result: mbedtls_ecp_point,
    scalar: mbedtls_mpi,
}

impl Curve {
    /// Curve25519, with `private` (clamped) as the scalar
    fn with_scalar(private: &[u8; KEY_LEN]) -> Box<Curve> {
        unsafe {
            let mut curve: Box<Curve> = Box::new(core::mem::zeroed());
            mbedtls_ecp_group_init(&mut curve.group);
            mbedtls_ecp_point_init(&mut curve.point);
            mbedtls_ecp_point_init(&mut curve.result);
            mbedtls_mpi_init(&mut curve.scalar);
            mbedtls_ecp_group_load(
                &mut curve.group,
                mbedtls_ecp_group_id_MBEDTLS_ECP_DP_CURVE25519,
            );
            mbedtls_mpi_read_binary_le(&mut curve.scalar, private.as_ptr(), KEY_LEN);
            curve
        }
    }

    /// scalar times `point`, as 32 bytes. Err if mbedTLS refuses
    fn multiply(&mut self, point: Option<&[u8]>) -> Result<[u8; KEY_LEN], i32> {
        let mut out = [0u8; KEY_LEN];
        unsafe {
            let base: *const mbedtls_ecp_point = match point {
                Some(bytes) => {
                    check(mbedtls_ecp_point_read_binary(
                        &self.group,
                        &mut self.point,
                        bytes.as_ptr(),
                        bytes.len(),
                    ))?;
                    &self.point
                }
                None => &self.group.G,
            };
            check(mbedtls_ecp_mul(
                &mut self.group,
                &mut self.result,
                &self.scalar,
                base,
                Some(mbedtls_random),
                core::ptr::null_mut(),
            ))?;
            let mut written = 0;
            check(mbedtls_ecp_point_write_binary(
                &self.group,
                &self.result,
                MBEDTLS_ECP_PF_UNCOMPRESSED as i32,
                &mut written,
                out.as_mut_ptr(),
                KEY_LEN,
            ))?;
        }
        Ok(out)
    }
}

impl Drop for Curve {
    fn drop(&mut self) {
        unsafe {
            mbedtls_ecp_group_free(&mut self.group);
            mbedtls_ecp_point_free(&mut self.point);
            mbedtls_ecp_point_free(&mut self.result);
            mbedtls_mpi_free(&mut self.scalar);
        }
    }
}

fn check(code: i32) -> Result<(), i32> {
    if code == 0 {
        Ok(())
    } else {
        Err(code)
    }
}

/// RFC 7748's clamping: a multiple of 8, bit 254 set. mbedTLS refuses an
/// unclamped Curve25519 key
fn clamp(key: &mut [u8; KEY_LEN]) {
    key[0] &= 248;
    key[31] &= 127;
    key[31] |= 64;
}

impl Dh for X25519 {
    fn name(&self) -> &'static str {
        "25519"
    }

    fn pub_len(&self) -> usize {
        KEY_LEN
    }

    fn priv_len(&self) -> usize {
        KEY_LEN
    }

    fn set(&mut self, privkey: &[u8]) {
        self.private.copy_from_slice(&privkey[..KEY_LEN]);
        let mut clamped = self.private;
        clamp(&mut clamped);
        self.public = Curve::with_scalar(&clamped)
            .multiply(None)
            .unwrap_or_default();
    }

    fn generate(&mut self, rng: &mut dyn Random) -> Result<(), snow::Error> {
        let mut private = [0u8; KEY_LEN];
        rng.try_fill_bytes(&mut private)?;
        self.set(&private);
        Ok(())
    }

    fn pubkey(&self) -> &[u8] {
        &self.public
    }

    fn privkey(&self) -> &[u8] {
        &self.private
    }

    fn dh(&self, pubkey: &[u8], out: &mut [u8]) -> Result<(), snow::Error> {
        let mut clamped = self.private;
        clamp(&mut clamped);
        // RFC 7748: the top bit of a public key is ignored
        let mut public = [0u8; KEY_LEN];
        public.copy_from_slice(&pubkey[..KEY_LEN]);
        public[31] &= 127;
        let shared = Curve::with_scalar(&clamped)
            .multiply(Some(&public))
            .map_err(|_| snow::Error::Dh)?;
        out[..KEY_LEN].copy_from_slice(&shared);
        Ok(())
    }
}

/// AES-256-GCM on the AES hardware. Noise's nonce: 4 zero bytes, then
/// the 64-bit counter big endian
#[derive(Default)]
struct AesGcm {
    key: [u8; KEY_LEN],
}

/// A GCM context with `key` set, freed when dropped
struct Gcm(esp_gcm_context);

impl Gcm {
    fn new(key: &[u8; KEY_LEN]) -> Box<Gcm> {
        unsafe {
            let mut gcm: Box<Gcm> = Box::new(core::mem::zeroed());
            esp_aes_gcm_init(&mut gcm.0);
            esp_aes_gcm_setkey(
                &mut gcm.0,
                mbedtls_cipher_id_t_MBEDTLS_CIPHER_ID_AES,
                key.as_ptr(),
                (KEY_LEN * 8) as u32,
            );
            gcm
        }
    }
}

impl Drop for Gcm {
    fn drop(&mut self) {
        unsafe { esp_aes_gcm_free(&mut self.0) };
    }
}

fn gcm_nonce(nonce: u64) -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[4..].copy_from_slice(&nonce.to_be_bytes());
    bytes
}

impl Cipher for AesGcm {
    fn name(&self) -> &'static str {
        "AESGCM"
    }

    fn set(&mut self, key: &[u8; KEY_LEN]) {
        self.key = *key;
    }

    fn encrypt(&self, nonce: u64, authtext: &[u8], plaintext: &[u8], out: &mut [u8]) -> usize {
        let iv = gcm_nonce(nonce);
        let (text, tag) = out[..plaintext.len() + TAG_LEN].split_at_mut(plaintext.len());
        let code = unsafe {
            esp_aes_gcm_crypt_and_tag(
                &mut Gcm::new(&self.key).0,
                MBEDTLS_GCM_ENCRYPT as i32,
                plaintext.len(),
                iv.as_ptr(),
                iv.len(),
                authtext.as_ptr(),
                authtext.len(),
                plaintext.as_ptr(),
                text.as_mut_ptr(),
                TAG_LEN,
                tag.as_mut_ptr(),
            )
        };
        assert_eq!(code, 0, "AES-GCM encrypt failed");
        plaintext.len() + TAG_LEN
    }

    fn decrypt(
        &self,
        nonce: u64,
        authtext: &[u8],
        ciphertext: &[u8],
        out: &mut [u8],
    ) -> Result<usize, snow::Error> {
        let len = ciphertext
            .len()
            .checked_sub(TAG_LEN)
            .ok_or(snow::Error::Decrypt)?;
        let iv = gcm_nonce(nonce);
        let code = unsafe {
            esp_aes_gcm_auth_decrypt(
                &mut Gcm::new(&self.key).0,
                len,
                iv.as_ptr(),
                iv.len(),
                authtext.as_ptr(),
                authtext.len(),
                ciphertext[len..].as_ptr(),
                TAG_LEN,
                ciphertext.as_ptr(),
                out.as_mut_ptr(),
            )
        };
        if code == 0 {
            Ok(len)
        } else {
            Err(snow::Error::Decrypt)
        }
    }
}

/// SHA-256 on the SHA hardware
struct Sha256(Box<mbedtls_sha256_context>);

// The context is plain memory; snow uses it from one thread at a time
unsafe impl Send for Sha256 {}
unsafe impl Sync for Sha256 {}

impl Sha256 {
    fn new() -> Sha256 {
        let mut sha = Sha256(Box::new(unsafe { core::mem::zeroed() }));
        unsafe { mbedtls_sha256_init(&mut *sha.0) };
        sha.reset();
        sha
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        unsafe { mbedtls_sha256_free(&mut *self.0) };
    }
}

impl Hash for Sha256 {
    fn name(&self) -> &'static str {
        "SHA256"
    }

    fn block_len(&self) -> usize {
        64
    }

    fn hash_len(&self) -> usize {
        32
    }

    fn reset(&mut self) {
        unsafe { mbedtls_sha256_starts(&mut *self.0, 0) };
    }

    fn input(&mut self, data: &[u8]) {
        unsafe { mbedtls_sha256_update(&mut *self.0, data.as_ptr(), data.len()) };
    }

    fn result(&mut self, out: &mut [u8]) {
        unsafe { mbedtls_sha256_finish(&mut *self.0, out.as_mut_ptr()) };
    }
}

/// For src/bin/noise_test.rs: known answers for each primitive, then a whole
/// XK handshake between two ends in this one process and a message each
/// way, timed, with the heap it took
pub fn self_test() {
    let hex = |text: &str| -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    };

    // RFC 7748 section 6.1
    let mut alice = X25519::default();
    alice.set(&hex(
        "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
    ));
    let mut shared = [0u8; KEY_LEN];
    let bob_public = hex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f");
    let dh_ok = alice.dh(&bob_public, &mut shared).is_ok();
    let x25519_ok = dh_ok
        && alice.public[..]
            == hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")[..]
        && shared[..]
            == hex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742")[..];

    // FIPS 180-2: SHA-256("abc")
    let mut sha = Sha256::new();
    sha.input(b"abc");
    let mut digest = [0u8; 32];
    sha.result(&mut digest);
    let sha_ok =
        digest[..] == hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")[..];

    // NIST GCM test case 13: AES-256, zero key and IV, nothing to encrypt
    let gcm = AesGcm::default();
    let mut tag = [0u8; TAG_LEN];
    gcm.encrypt(0, &[], &[], &mut tag);
    let gcm_ok = tag[..] == hex("530f8afbc74536b9a963b4f1c4cb738b")[..];

    log::info!(
        "Noise self-test: X25519 {} SHA-256 {} AES-GCM {}",
        ok(x25519_ok),
        ok(sha_ok),
        ok(gcm_ok)
    );

    let heap_before = unsafe { esp_get_free_heap_size() };
    let started = unsafe { esp_timer_get_time() };
    match handshake() {
        Ok((least_heap, message)) => log::info!(
            "Noise self-test: XK handshake + 1 message each way {} ms (both ends), \
             heap {} B at most, got {:?}",
            (unsafe { esp_timer_get_time() } - started) / 1000,
            heap_before.saturating_sub(least_heap),
            message
        ),
        Err(e) => log::warn!("Noise self-test: handshake failed: {:?}", e),
    }
}

fn ok(passed: bool) -> &'static str {
    if passed {
        "ok"
    } else {
        "FAILED"
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
enum TestMessage {
    Hello { mac: [u8; 6], app_sha256: [u8; 32] },
    UpToDate,
}

/// The least free heap seen, and what the server got
fn handshake() -> Result<(u32, TestMessage), snow::Error> {
    let params: snow::params::NoiseParams = PATTERN.parse()?;
    let builder = || snow::Builder::with_resolver(params.clone(), Box::new(MbedtlsResolver));
    let server_keys = builder().generate_keypair()?;
    let radio_keys = builder().generate_keypair()?;

    let mut radio = builder()
        .local_private_key(&radio_keys.private)?
        .remote_public_key(&server_keys.public)?
        .build_initiator()?;
    let mut server = builder()
        .local_private_key(&server_keys.private)?
        .build_responder()?;

    let mut wire = [0u8; 256];
    let mut read = [0u8; 256];
    let mut least_heap = u32::MAX;
    let mut note_heap = || least_heap = least_heap.min(unsafe { esp_get_free_heap_size() });
    // XK: -> e, es   <- e, ee   -> s, se
    let len = radio.write_message(&[], &mut wire)?;
    server.read_message(&wire[..len], &mut read)?;
    note_heap();
    let len = server.write_message(&[], &mut wire)?;
    radio.read_message(&wire[..len], &mut read)?;
    note_heap();
    let len = radio.write_message(&[], &mut wire)?;
    server.read_message(&wire[..len], &mut read)?;
    note_heap();

    let mut radio = radio.into_transport_mode()?;
    let mut server = server.into_transport_mode()?;
    let hello = TestMessage::Hello {
        mac: [0xF8, 0x5B, 0x1B, 0xA5, 0xF0, 0x00],
        app_sha256: [7; 32],
    };
    let plain = postcard::to_allocvec(&hello).map_err(|_| snow::Error::Input)?;
    let len = radio.write_message(&plain, &mut wire)?;
    let got = server.read_message(&wire[..len], &mut read)?;
    let message: TestMessage =
        postcard::from_bytes(&read[..got]).map_err(|_| snow::Error::Input)?;
    let reply = postcard::to_allocvec(&TestMessage::UpToDate).map_err(|_| snow::Error::Input)?;
    let len = server.write_message(&reply, &mut wire)?;
    radio.read_message(&wire[..len], &mut read)?;
    note_heap();
    Ok((least_heap, message))
}
