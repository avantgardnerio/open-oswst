# open-oswst

Stealth LoRa radio mesh where the primary design principle is "only speak when spoken to".

![open-oswst radio in 3d printed case](docs/in-case.jpg)

## Hardware

- **Board**: [Heltec WiFi LoRa 32 V4](https://heltec.org/project/wifi-lora-32-v4/) (ESP32-S3 + SX1262, 863-928 MHz)
- **MCU**: ESP32-S3 rev 0.2, 16MB flash, 338 KiB RAM
- **Radio**: Semtech SX1262 LoRa transceiver, 915 MHz ISM band
- **Display**: SSD1306 128x64 OLED (I2C)

### Pin Map

| Function | GPIO |
|---|---|
| PRG button (PTT) | 0 (active LOW, internal pull-up) |
| White LED | 35 |
| Vext power enable | 36 (LOW = on) |
| OLED SDA | 17 |
| OLED SCL | 18 |
| OLED RST | 21 |
| LoRa SCK | 9 |
| LoRa MOSI | 10 |
| LoRa MISO | 11 |
| LoRa NSS | 8 |
| LoRa RST | 12 |
| LoRa DIO1 | 14 |
| LoRa BUSY | 13 |

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

4. **Optional**: install `picocom` for serial monitoring:

```bash
sudo apt install picocom
```

### Build

```bash
. ~/export-esp.sh
cargo build
```

ESP-IDF v5.5.x is downloaded automatically by `esp-idf-sys` on first build (takes a while).

### Flash

The firmware uses a custom partition table with a dedicated `open-oswst` NVS partition for device config. Since `espflash flash` overwrites the partition table with its own, you must re-write ours after flashing:

```bash
. ~/export-esp.sh

# 1. Flash firmware
espflash flash -p /dev/ttyACM0 target/xtensa-esp32s3-espidf/debug/open-oswst

# 2. Overwrite partition table with ours (espflash clobbers it in step 1)
espflash write-bin -p /dev/ttyACM0 0x8000 target/xtensa-esp32s3-espidf/debug/partition-table.bin

# 3. Write device config (only needed on first flash or to change config)
espflash write-bin -p /dev/ttyACM0 0xfad000 ~/phy/open-oswst_nvs.bin
```

Monitor separately:

```bash
picocom /dev/ttyACM0 -b 115200
```

### Device Config (NVS)

Device configuration lives in a dedicated `open-oswst` NVS partition at `0xfad000` (12KB), separate from the system NVS (PHY cal, WiFi, etc).

Generate a config image from `nvs_open-oswst.csv`:

```bash
# Generate NVS image (edit nvs_open-oswst.csv to change values)
python3 .embuild/espressif/python_env/idf5.5_py3.13_env/lib/python3.13/site-packages/esp_idf_nvs_partition_gen/nvs_partition_gen.py \
    generate nvs_open-oswst.csv ~/phy/open-oswst_nvs.bin 0x3000

# Flash to board
espflash write-bin -p /dev/ttyACM0 0xfad000 ~/phy/open-oswst_nvs.bin
```

Current config keys (namespace `config`):

| Key | Type | Values | Default |
|-----|------|--------|---------|
| `repeater` | u8 | 0=endpoint, 1=repeater | 0 |

There is also `nvs_config.py` for read-modify-write of the *system* NVS partition (preserving PHY cal data etc), but the dedicated partition approach above is preferred.

## Current Behavior

- Simplex push-to-talk voice radio over LoRa
- Hold **PRG button** (GPIO 0) to talk — audio is captured, Codec2-encoded at 1200 bps, and streamed as 4-frame LoRa packets (160ms audio each)
- Release to listen — received audio is decoded and played through I2S speaker (MAX98357A)
- OLED shows mode (RX Listening / TX Streaming / RX Audio) with RSSI and SNR on receive
- CSMA with preamble-aware jitter for collision avoidance
- Per-device config via dedicated NVS partition (repeater mode flag)
- **Next up**: repeater mesh relay

## Key Implementation Notes

- **Framework**: esp-idf-svc 0.52.1 (std Rust, not bare-metal)
- **Async**: `block_on` + `embassy_futures::select` for zero-polling PTT/RX racing
- **LoRa driver**: `lora-phy` (upstream git, 3.0.2-alpha) with `GenericSx126xInterfaceVariant`
- **GPIO type erasure**: `degrade_input()`/`degrade_output()` required for lora-phy's generic interface
- **SPI async**: `CONFIG_SPI_MASTER_ISR_IN_IRAM` disabled in `sdkconfig.defaults`
- **defmt workaround**: `defmt-discard.x` linker script discards defmt sections that break ESP-IDF flash layout
- **Stack**: 65536 bytes for main task (Codec2 init needs large stack temporaries)

## Project Structure

```
src/main.rs          # Entry point, channels, NVS config read
src/radio.rs         # SPI + LoRa init, IRQ-driven RX/TX loop, CSMA
src/app.rs           # PTT, ADC, OLED, Codec2, I2S speaker
partitions.csv       # Custom partition table (adds open-oswst NVS)
nvs_open-oswst.csv     # Default device config values
nvs_config.py        # Tool for read-modify-write of system NVS
sdkconfig.defaults   # ESP-IDF config overrides
defmt-discard.x      # Linker script to discard defmt sections
rust-toolchain.toml  # Pins to "esp" toolchain channel
build.rs             # Links defmt-discard.x
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
