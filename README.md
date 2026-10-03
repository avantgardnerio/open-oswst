# open-oswst

Stealth LoRa radio mesh where the primary design principle is "only speak when spoken to": a simplex push-to-talk voice radio over
LoRa, with flood repeaters, that transmits nothing until someone presses PTT.

![open-oswst radio in 3d printed case](docs/in-case.jpg)

## Hardware

- **Board**: [Heltec WiFi LoRa 32 V4.3](https://heltec.org/project/wifi-lora-32-v4/) (ESP32-S3 + SX1262, 863-928 MHz)
- **MCU**: ESP32-S3 rev 0.2, 16MB flash, ~380 KiB RAM, no PSRAM
- **Radio**: Semtech SX1262 LoRa transceiver, 915 MHz ISM band, behind the board's KCT8103L front-end module (PA + LNA), then an
  external "Air Buddy" amplifier. The SX1262 runs at 6 dBm, which should give roughly 1 W out of the Air Buddy (estimated from datasheets,
  not measured)
- **GPS**: L76K on the Heltec's GNSS connector (UTC time and position, logged and shown on screen)
- **Display**: SSD1306 128x64 OLED (I2C)
- **Audio**: MAX9814 electret mic (ADC), MAX98357A I2S class-D speaker amp

The board is a V4.3. All the boards we've tested are; a V4.2 (GC1109 front end) would need different FEM pins and TX power.

### Pin Map

`src/board.rs` is the one place pins are assigned; this table mirrors it.

| Function | GPIO | Notes |
|---|---|---|
| PTT (PRG button) | 0 | Active LOW, internal pull-up |
| VOL encoder A / B / SW | 3 / 6 / 45 | GPIO45 is a strapping pin, safe as a switch to GND |
| Mic (ADC1) | 4 | MAX9814 out |
| Speaker I2S BCLK / DIN / WS | 47 / 33 / 48 | MAX98357A |
| OLED SDA / SCL / RST | 17 / 18 / 21 | |
| Vext power enable | 36 | LOW = on (OLED) |
| LoRa SCK / MOSI / MISO / NSS | 9 / 10 / 11 / 8 | SPI2 |
| LoRa RST / DIO1 / BUSY | 12 / 14 / 13 | |
| FEM power / enable (CSD) | 7 / 2 | Held HIGH while running |
| FEM CTX | 5 | HIGH only during TX (LOW = RX through the LNA) |
| GPS UART RX / TX | 39 / 38 | ESP side. Meshtastic's variant file comments have these the other way round |
| GPS power / reset / wake | 34 / 42 / 40 | Power is active LOW |
| Battery sense | 1 (+ 37 enables the divider) | Free for this on the newest PCB rev; not used by the firmware yet |

## Dev Environment Setup

### Prerequisites

1. **Install Rust + Xtensa toolchain** via [espup](https://github.com/esp-rs/espup):

```bash
cargo install espup
espup install
```

2. **Create an ESP environment export script** (`~/export-esp.sh`):

```bash
export LIBCLANG_PATH="$HOME/.rustup/toolchains/esp/xtensa-esp32-elf-clang/esp-20.1.1_20250829/esp-clang/lib"
export PATH="$HOME/.rustup/toolchains/esp/xtensa-esp-elf/esp-15.2.0_20250920/xtensa-esp-elf/bin:$PATH"
```

The exact paths may vary — check `~/.rustup/toolchains/esp/` after `espup install`.

3. **Install flashing tools**:

```bash
cargo install espflash cargo-espflash ldproxy
```

4. **Python tools** (log pulling, PCB and case generation) live in a venv built with Python 3.13:

```bash
python3.13 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

### Build

```bash
. ~/export-esp.sh
cargo build                      # firmware (repo root, ESP32-S3)
(cd core && cargo test)          # app logic unit tests, on this PC
(cd desktop && cargo run)        # desktop build (a skeleton so far)
```

ESP-IDF v5.5.x is downloaded automatically by `esp-idf-sys` on first build (takes a while).

### Flash

Over USB, use the script. It flashes every connected board at once, and passes the three things a bare `espflash flash` gets wrong:

```bash
. ~/export-esp.sh && cargo build && scripts/flash-all.sh
```

- **our bootloader** (`--bootloader`): it switches the flash to QIO. espflash's bundled one is DIO
- **our partition table** (`--partition-table`): espflash's default drops the `storage` partition and the OTA slots
- **`--erase-parts otadata`**: otherwise a board last updated over WiFi keeps booting the other slot

Over WiFi, see [In the Field](#in-the-field).

Serial ports aren't stable between plug-ins: identify boards by MAC (the ESP32-S3's USB serial number), not by `ttyACM` number. To capture a
boot log from the first line, `python3 scripts/boot-log.py <PORT>`. Never flash while another program holds the port.

### Device Config

Each board's settings are a text file on its storage: `/data/config.toml`. On the newest PCB rev the on-device menu (click the VOL knob)
sets the mode and rewrites the file.

```toml
mode = "repeater"        # normal | repeater | echo

[[wifi]]                 # networks to join, tried in order
ssid = "Starlink"
password = "..."
```

`scripts/provision.py` writes it over USB, from `boards.toml` in the repo root (kept out of git: it holds WiFi passwords; copy
`boards.example.toml` to start). It flashes the firmware too, and **erases the board's storage**, logs included: pull them first.

```bash
.venv/bin/python scripts/provision.py                      # every connected board
.venv/bin/python scripts/provision.py A4:CB:8F:A2:0F:5C    # just this one
```

The boot log shows the result, e.g. `Config: mode=Echo, 1 WiFi network(s)`.

### Logs

Every boot writes a text log to `/data/log/NNNN.txt` on the board's LittleFS `storage` partition (~11.6 MB). Lines are buffered in RAM and
only written to flash after 3 s with no radio traffic, because a flash write stalls the chip for up to ~18 ms. To copy them off a board:

```bash
.venv/bin/python scripts/pull-logs.py <PORT>     # ~80 s; writes logs/<MAC>-<time>/
```

After an echo test, join the handheld's and the echo station's logs into a per-transmission table and a map (OpenStreetMap/satellite,
one circle per transmission coloured by how well each direction got through, plus GeoJSON for QGIS):

```bash
.venv/bin/python scripts/range-report.py <handheld log> <echo station log>
```

## In the Field

Every radio has the field hotspot (Starlink) in its `config.toml` and stays on WiFi, so in the field the laptop does everything over
WiFi: find the radios, read their status and logs, change their config, update their firmware. USB is the fallback.

### Laptop setup (once, at home)

```bash
sudo apt install avahi-utils curl    # avahi-browse finds the radios; Ubuntu resolves *.local out of the box
```

Plus the build tools above (`espup`, `espflash`, `~/export-esp.sh`), so firmware can be built and saved as an image in the field.

### Find the radios

Join the laptop to the same network as the radios, then:

```bash
avahi-browse -rtp _oswst._tcp | grep "^="    # one line per radio: name, host, IP, port
```

Each radio is `oswst-XXXX.local`, from the last 4 hex digits of its MAC (A4:CB:8F:A2:0F:5C is `oswst-0f5c`). A radio joins WiFi ~7 s
after boot. If a radio doesn't show up:

- `getent hosts oswst-0f5c.local`: asks for one radio by name
- check that avahi-browse ran at all. Don't pipe its errors into grep: if it isn't installed, `2>&1 | grep` prints nothing,
  which looks exactly like "no radios"
- last resort, scan the subnet for the API:
  `for i in $(seq 1 254); do curl -s -m 1 http://192.168.0.$i/status | grep -q mac && echo 192.168.0.$i & done; wait`

### Talk to a radio

The HTTP API (`src/http.rs`), port 80, no password (the WiFi password is the security):

```bash
R=oswst-0f5c.local
curl http://$R/status                         # name, MAC, firmware (git hash), OTA slot, mode, uptime, heap
curl http://$R/logs                           # list the log files
curl -O http://$R/logs/0007.txt               # fetch one: a 25 KB log in under 0.1 s (vs ~80 s over USB)
curl http://$R/config                         # read config.toml
curl -X PUT --data-binary @config.toml http://$R/config    # replace it (checked first); applies on reboot
curl -X POST http://$R/reboot
curl -X POST http://$R/wifi/off               # WiFi off until the next power cycle (for timing-sensitive tests)
```

`/wifi/off` isn't saved: every boot starts with WiFi on again, so a power cycle always gets the radio back on the network. Once it's
off, the radio can't be reached over WiFi until then.

### Update firmware over WiFi (OTA)

Build, save an app image, and post it. The radio writes it to its spare slot and reboots into it (~2 MB in 10–13 s, back up ~4 s
later). `/status` then shows the new git hash and the other slot.

```bash
. ~/export-esp.sh && cargo build
espflash save-image --chip esp32s3 target/xtensa-esp32s3-espidf/debug/open-oswst app.bin
curl --data-binary @app.bin http://oswst-0f5c.local/ota
```

Every radio found:

```bash
for host in $(avahi-browse -rtp _oswst._tcp | grep "^=" | cut -d';' -f7 | sort -u); do
    curl --data-binary @app.bin http://$host/ota
done
```

### USB fallback

When a radio isn't on WiFi:

- **Flash:** `scripts/flash-all.sh`. It passes our bootloader, our partition table and `--erase-parts otadata`, which a bare
  `espflash flash` gets wrong.
- **Pull logs:** `scripts/pull-logs.py` (~80 s).
- **Watch a boot:** `scripts/boot-log.py`.

Identify boards by MAC, not by `ttyACM` number, and never flash while another program holds the port.

## Current Behavior

- **Voice**: simplex push-to-talk. Hold PTT to talk: audio is Codec2-encoded at 1200 bps and streamed as 4-frame LoRa packets (160 ms of
  audio each, SF7/125 kHz). Release to listen: received packets are reordered, decoded and played through the speaker. A lost packet
  costs one 160 ms gap, and a corrupt one is dropped rather than resetting playback
- **Modes**: normal; **repeater** (relays every new packet it hears, flood style); **echo** (records a transmission, then plays it back
  over the air, for range testing alone). Saved in NVS
- **Screen**: short MAC and UTC time, GPS position, RX/TX state, RSSI and SNR of received audio, and volume
- **Controls**: VOL knob turns volume; clicking it opens the menu (Lock, Mode). Needs the newest PCB rev's wiring (see Pin Map)
- **Logs**: persistent per-boot text logs including a GPS fix every 10 s, so field tests can be mapped afterwards
- **Collision avoidance**: CSMA with preamble-aware jitter
- **Next up**: making several repeaters work on one channel (faster TX turnaround, CAD before transmitting, SF6, and experiments with
  synchronised relaying), then frequency hopping and encryption

## Key Implementation Notes

- **Framework**: esp-idf-svc 0.52.1 (std Rust, not bare-metal)
- **Async**: `block_on` + `embassy_futures::select`; the app sleeps until an event (packet, PTT, speaker, knob) or a 250 ms housekeeping tick
- **LoRa driver**: `lora-phy` (lora-rs git main) with `GenericSx126xInterfaceVariant`; DIO2 drives the FEM's TX/RX path, GPIO5 its RX LNA
- **Hardware-free app**: everything above the drivers is in `core/` behind small device traits, so it runs and is tested on a PC
- **Storage**: LittleFS (`joltwallet/littlefs` component) mounted at `/data`, chosen over FAT for power-cut safety
- **SPI async**: `CONFIG_SPI_MASTER_ISR_IN_IRAM` disabled in `sdkconfig.defaults`
- **defmt workaround**: `defmt-discard.x` linker script discards defmt sections that break ESP-IDF flash layout
- **Stack**: 65536 bytes for main task (Codec2 init needs large stack temporaries)

## Project Structure

Three Cargo workspaces. Each opens as its own IDE project: the firmware builds for the ESP32-S3, the other two for this PC.

```
src/                 # Firmware (repo root workspace, xtensa)
  main.rs            #   Brings up the board and runs the core app on it
  board.rs           #   The one place pins are assigned
  devices/           #   ESP drivers: radio, fem, gps, mic, speaker, screen, encoder, ptt, settings, storage
  bin/               #   Bringup tests (fem_probe, gps_test, storage_test, batt_probe, loopbacks, ...)
core/                # Hardware-free app: its own workspace, so `cargo test` runs on this PC
  src/app.rs         #   Event loop, screens, housekeeping
  src/rx_buffer.rs   #   Reorders received packets for playback (unit tested)
  src/codec.rs       #   Codec2 thread
  src/echo.rs, mode.rs, menu.rs, packet.rs, logger.rs
  src/devices/       #   Device interfaces: radio/speaker/screen channels, gps/mic/ptt/knob/settings traits
desktop/             # Desktop build on virtual devices (skeleton so far)
scripts/             # boot-log.py, pull-logs.py, flash-all.sh, ...
pcb/                 # PCB definition (Python DSL) and generated KiCad board
models/              # Parametric case (build123d) and fit check
partitions.csv       # Partition table: two OTA app slots, LittleFS storage (/data)
boards.example.toml  # Per-board settings for scripts/provision.py (copy to boards.toml)
sdkconfig.defaults   # ESP-IDF config overrides
```

## Bill of Materials

### Handset (~$122)

| Component                                                                                       | Part                                                       | Qty | Est. Cost |
|-------------------------------------------------------------------------------------------------|------------------------------------------------------------|-----|-----------|
| [MCU + LoRa + GPS](https://www.amazon.com/dp/B0GCD9W6JZ?ref=ppx_yo2ov_dt_b_fed_asin_title&th=1) | Heltec WiFi LoRa 32 V4                                     | 1   | $42       |
| [RF amplifier](https://www.amazon.com/dp/B0DQBJVR91?ref=ppx_yo2ov_dt_b_fed_asin_title)          | AB-IOT-868 "Air Buddy" (902-928 MHz, bidirectional TX+LNA) | 1   | $26       |
| [Speaker amp](https://www.amazon.com/dp/B0DPJRLMDJ?ref=ppx_yo2ov_dt_b_fed_asin_title)           | MAX98357A I2S Class D breakout                             | 1   | $3        |
| [Speaker](https://www.amazon.com/dp/B0BTP67F81?ref=ppx_yo2ov_dt_b_fed_asin_title&th=1)          | 8 ohm 1W mini (JST-PH1.25mm)                               | 1   | $2.50     |
| [Microphone](https://www.amazon.com/dp/B0B7SP6GYX?ref=ppx_yo2ov_dt_b_fed_asin_title)            | MAX9814 electret with AGC (analog out)                     | 1   | $4        |
| [Antenna](https://www.amazon.com/dp/B0D47HVCK7?ref=ppx_yo2ov_dt_b_fed_asin_title&th=1)          | 5 dBi SMA whip (915 MHz)                                   | 1   | $6        |
| [Battery](https://www.amazon.com/dp/B0FPCWFFYB?ref=ppx_yo2ov_dt_b_fed_asin_title)               | YELUFT 1S 103450 3.7V 2000mAh LiPo (JST 1.25)              | 1   | $24       |
| [encoder](https://www.amazon.com/dp/B07F24TRYG?ref=ppx_yo2ov_dt_b_fed_asin_title)               | EC11 rotary encoder w/ push button (Taiss)                 | 2   | $1.25     |
| [Power switch](https://www.amazon.com/dp/B0DN69L9SG?ref=ppx_yo2ov_dt_b_fed_asin_title&th=1)     | SS12F15 mini slide switch (SPDT)                           | 1   | $0.30     |
| [PTT button](https://www.amazon.com/dp/B015X34IP6?ref=ppx_yo2ov_dt_b_fed_asin_title&th=1)       | Ulincos U16A1 16mm metal momentary, flush                  | 1   | $8        |
| [Speaker jack](https://www.amazon.com/dp/B01ASF0LW8?ref=ppx_yo2ov_dt_b_fed_asin_title)          | 2.5mm TRS chassis mount (Calrad 30-714 or similar)         | 1   | $0.66     |
| [Mic/PTT jack](https://www.amazon.com/dp/B00ZYWJ1DG?ref=ppx_yo2ov_dt_b_fed_asin_title)          | 3.5mm TRS chassis mount w/ switch (CESS)                   | 1   | $2        |
| [F2F SMA](https://www.amazon.com/dp/B0FB3R5WRL?ref=ppx_yo2ov_dt_b_fed_asin_title&th=1)          | Female to Female SMA                                       | 1 | $1        |

## Related Projects

Voice over LoRa is rare. This is what we found as of October 2026, with our own status stated the same way as everyone else's.

| Project | What it does | Repeating | Licence | Status |
|---|---|---|---|---|
| **open-oswst** (this) | Live push-to-talk, Codec2 1200 bps, handheld with case and PCB | Flood repeater mode exists, but more than one repeater degrades audio; making multi-repeater flooding work is the current focus | None (US 902–928 MHz ISM) | Live voice works handheld to handheld; field-tested to ~600 m through suburbs (GPS-logged). No encryption or frequency hopping yet |
| [QMesh](https://github.com/faydr/QMesh) ([project](https://hackaday.io/project/161491-qmesh-a-lora-based-voice-mesh-network), [paper](https://cdn.hackaday.io/files/1614916909230944/TAPR%20DCC%202020%20Paper.pdf)) | Live voice, Codec2, 160 ms frames | **Synchronised flooding:** every node retransmits at once in TDMA slots, with extra FEC and deliberate timing/frequency offsets so the copies don't destructively collide, plus per-packet hopping. Reported 99% packet reception with 2–3 simultaneous retransmitters | Amateur (US Technician) | **Ahead of us on radio techniques.** STM32 dev boards + custom shield. No commits since December 2022 |
| [Mesh-Talkie](https://github.com/WIH4/Mesh-Talkie) | Planned PTT handheld, ESP32-S3 + SX1262 (our hardware), AES-256 | Planned flood mesh with TTL | None (EU SRD) | Design documents only: no hardware or firmware yet |
| [esp32_loradv](https://github.com/sh123/esp32_loradv) | Codec2/Opus digital-voice walkie-talkie | None (point to point) | Amateur (70 cm) | Hobby project |
| Meshtastic, [MeshCore](https://github.com/RipeStore/meshcorebuilder), [MeshTRX](https://github.com/StanislavButkovsky/meshtrx) | Text mesh messengers | Yes, for text | None | Mature for text; voice is recorded clips forwarded later, not live PTT |
| [dudmuck/lora_codec2](https://github.com/dudmuck/lora_codec2), [Lora-Voice-Image-Text](https://github.com/chicodog530/Lora-Voice-Image-Text), [ESP32_Codec2](https://github.com/deulis/ESP32_Codec2) | Codec2-over-LoRa demos | None | — | Demos |

As far as we can tell, nobody else has live push-to-talk voice over LoRa working on handhelds **without a licence**. The hard part, several repeaters sharing one channel, is unsolved here; QMesh has gone furthest on it, and under an amateur licence.

## License

TBD
