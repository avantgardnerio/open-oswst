//! Heltec V4 front-end module (FEM) probe — tells a V4.2 (GC1109) from a V4.3
//! (KCT8103L), and shows how much the FEM does for RX. Never transmits, so
//! it's safe with the Air Buddy attached.
//!
//! Unplug the VOL and CHNL harnesses first: this drives GPIO2 (VOL_B) and
//! GPIO5 (CHNL_SW), and reads GPIO5's pulls.
//!
//! 1. Pull test: read GPIO5 with pull-up then pull-down, FEM off and on.
//!    A floating pin (V4.2) follows the pull; the KCT8103L's CTX input may not.
//! 2. RSSI test: hold PTT on another radio. We rotate through three FEM states
//!    every few seconds and print each state's average RSSI/SNR:
//!    - OFF: GPIO7/GPIO2 LOW (what the app does today)
//!    - ON-LNA: FEM powered + enabled, GPIO5 LOW (V4.3: RX through LNA)
//!    - ON-BYP: FEM powered + enabled, GPIO5 HIGH (V4.3: RX bypass)
//!
//!    V4.3: ON-LNA beats ON-BYP by 10+ dB. V4.2: those two match.
//!
//! Pins per Meshtastic's heltec_v4 variant.h: GPIO7 = FEM LDO power,
//! GPIO2 = CSD chip enable, GPIO46 = GC1109 CPS (PA mode, LOW for RX),
//! GPIO5 = KCT8103L CTX (LOW = RX LNA, HIGH = RX bypass).
//!
//! Build & flash: cargo build --bin fem_probe && espflash flash -p <PORT> --partition-table target/xtensa-esp32s3-espidf/debug/partition-table.bin target/xtensa-esp32s3-espidf/debug/fem_probe

use embassy_futures::join::join;
use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Instant, Timer};
use esp_idf_svc::hal::gpio::{AnyIOPin, Output, PinDriver, Pull};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::task::block_on;
use open_oswst::devices::radio;

/// How long each FEM state is held during the RSSI test.
const DWELL: Duration = Duration::from_secs(3);
/// Packets this soon after a switch are dropped — the LDO may still be settling,
/// or the packet started under the previous state.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy)]
enum FemState {
    Off,
    OnLna,
    OnBypass,
}

impl FemState {
    const ALL: [FemState; 3] = [FemState::Off, FemState::OnLna, FemState::OnBypass];

    fn name(self) -> &'static str {
        match self {
            FemState::Off => "OFF   ",
            FemState::OnLna => "ON-LNA",
            FemState::OnBypass => "ON-BYP",
        }
    }
}

struct Fem {
    power: PinDriver<'static, Output>,
    csd: PinDriver<'static, Output>,
    cps: PinDriver<'static, Output>,
    ctx: PinDriver<'static, Output>,
}

impl Fem {
    fn set(&mut self, state: FemState) {
        let on = !matches!(state, FemState::Off);
        self.cps.set_low().unwrap(); // RX mode on the GC1109
        self.ctx
            .set_level(matches!(state, FemState::OnBypass).into())
            .unwrap();
        self.power.set_level(on.into()).unwrap();
        self.csd.set_level(on.into()).unwrap();
    }
}

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("fem_probe starting, MAC {}", mac_string());

    let p = Peripherals::take().unwrap();

    // FEM power and enable, held off while we check GPIO5's pulls.
    let mut power = PinDriver::output(p.pins.gpio7).unwrap();
    let mut csd = PinDriver::output(p.pins.gpio2).unwrap();
    let mut cps = PinDriver::output(p.pins.gpio46).unwrap();
    power.set_low().unwrap();
    csd.set_low().unwrap();
    cps.set_low().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let gpio5: AnyIOPin<'static> = p.pins.gpio5.into();
    let gpio5 = pull_test("FEM off", gpio5);

    power.set_high().unwrap();
    csd.set_high().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut gpio5 = pull_test("FEM on ", gpio5);

    // Drive HIGH and read back: a pull-down resistor lets the pin go HIGH, a
    // hard short to GND doesn't. Kept brief, and ON-BYP is skipped on a short
    // so we don't fight it for the whole test.
    // SAFETY: the driver is dropped before the pin is used again.
    let shorted = {
        let mut drv = PinDriver::input_output(unsafe { gpio5.reborrow() }, Pull::Floating).unwrap();
        drv.set_high().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let high = drv.is_high();
        drv.set_low().unwrap();
        !high
    };
    log::info!(
        "Drive test: GPIO5 driven HIGH reads back {} -> {}",
        (!shorted) as u8,
        if shorted {
            "hard short to GND, skipping ON-BYP"
        } else {
            "resistive load (not a short)"
        }
    );

    let mut fem = Fem {
        power,
        csd,
        cps,
        ctx: PinDriver::output(gpio5).unwrap(),
    };
    fem.set(FemState::Off);

    let radio_p = radio::Peripherals {
        spi: p.spi2,
        sck: p.pins.gpio9.into(),
        mosi: p.pins.gpio10.into(),
        miso: p.pins.gpio11.into(),
        nss: p.pins.gpio8.into(),
        reset: p.pins.gpio12.into(),
        dio1: p.pins.gpio14.into(),
        busy: p.pins.gpio13.into(),
    };

    log::info!(
        "RSSI test: hold PTT on another radio. Rotating FEM state every {}s",
        DWELL.as_secs()
    );
    block_on(async {
        let radio = radio::init(radio_p).await;
        join(radio, rssi_test(&mut fem, shorted)).await;
    });
}

/// Read GPIO5 with pull-up then pull-down and log both. Returns the pin.
fn pull_test(label: &str, pin: AnyIOPin<'static>) -> AnyIOPin<'static> {
    let mut levels = [false; 2];
    let mut pin = pin;
    for (i, pull) in [Pull::Up, Pull::Down].into_iter().enumerate() {
        // SAFETY: the reborrowed driver is dropped each iteration, before the
        // next one is made and before the pin is returned.
        let drv = PinDriver::input(unsafe { pin.reborrow() }, pull).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        levels[i] = drv.is_high();
    }
    let verdict = match levels {
        [true, false] => "follows the pull (floating or weak load)",
        [true, true] => "held HIGH",
        [false, false] => "held LOW",
        [false, true] => "inverted?!",
    };
    log::info!(
        "Pull test, {}: GPIO5 pull-up={} pull-down={} -> {}",
        label,
        levels[0] as u8,
        levels[1] as u8,
        verdict
    );
    pin
}

async fn rssi_test(fem: &mut Fem, skip_bypass: bool) {
    // Per state: packet count, RSSI sum, SNR sum — accumulated across rounds.
    let mut totals = [(0u32, 0i32, 0i32); 3];
    let mut round = 0u32;
    loop {
        round += 1;
        for (i, state) in FemState::ALL.into_iter().enumerate() {
            if skip_bypass && matches!(state, FemState::OnBypass) {
                continue;
            }
            fem.set(state);
            let start = Instant::now();
            let end = start + DWELL;
            let (mut n, mut rssi, mut snr) = (0u32, 0i32, 0i32);
            while let Either::First(pkt) = select(radio::RX_CHAN.receive(), Timer::at(end)).await {
                if start.elapsed() < SETTLE {
                    continue;
                }
                n += 1;
                rssi += pkt.rssi as i32;
                snr += pkt.snr as i32;
            }
            let t = &mut totals[i];
            t.0 += n;
            t.1 += rssi;
            t.2 += snr;
            if n > 0 {
                log::info!(
                    "round {} {}: {} pkts, rssi {:.1} snr {:.1}",
                    round,
                    state.name(),
                    n,
                    rssi as f32 / n as f32,
                    snr as f32 / n as f32
                );
            } else {
                log::info!("round {} {}: no packets", round, state.name());
            }
        }

        log::info!("=== totals after {} rounds ===", round);
        for (state, (n, rssi, snr)) in FemState::ALL.into_iter().zip(totals) {
            if n > 0 {
                log::info!(
                    "  {}: {} pkts, rssi {:.1} snr {:.1}",
                    state.name(),
                    n,
                    rssi as f32 / n as f32,
                    snr as f32 / n as f32
                );
            } else {
                log::info!("  {}: no packets", state.name());
            }
        }
    }
}

fn mac_string() -> String {
    let mut mac = [0u8; 6];
    unsafe {
        esp_idf_svc::sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_svc::sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        );
    }
    mac.iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(":")
}
