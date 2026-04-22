# Loraudio

Voice over LoRa with repeaters.

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

### Flash & Monitor

```bash
. ~/export-esp.sh
espflash flash -p /dev/ttyACM0 --monitor target/xtensa-esp32s3-espidf/debug/loraudio
```

Or flash and monitor separately:

```bash
espflash flash -p /dev/ttyACM0 target/xtensa-esp32s3-espidf/debug/loraudio
picocom /dev/ttyACM0 -b 115200
```

## Current Behavior

- Boots into **RX mode** — OLED displays "RX Listening"
- Hold **PRG button** (GPIO 0) to transmit — sends "LORAUDIO #N" packets every 200ms
- Release button to return to RX
- Received packets display on OLED with RSSI and SNR

## Key Implementation Notes

- **Framework**: esp-idf-svc 0.52.1 (std Rust, not bare-metal)
- **Async**: `block_on` + `embassy_futures::select` for zero-polling PTT/RX racing
- **LoRa driver**: `lora-phy` 3.0.1 with `GenericSx126xInterfaceVariant`
- **GPIO type erasure**: `degrade_input()`/`degrade_output()` required for lora-phy's generic interface
- **SPI async**: `CONFIG_SPI_MASTER_ISR_IN_IRAM` disabled in `sdkconfig.defaults`
- **defmt workaround**: `defmt-discard.x` linker script discards defmt sections that break ESP-IDF flash layout
- **Stack**: 16384 bytes for main task (LoRa + async overhead)

## Project Structure

```
src/main.rs          # All application code (PTT loop, OLED, LoRa)
sdkconfig.defaults   # ESP-IDF config overrides
defmt-discard.x      # Linker script to discard defmt sections
rust-toolchain.toml  # Pins to "esp" toolchain channel
build.rs             # Links defmt-discard.x
```

## License

TBD
