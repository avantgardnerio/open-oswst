# Heltec WiFi LoRa 32 V4 — Pinout Reference

Transcribed from `docs/WiFi_LoRa_32_V4.2.0.pdf` (Rev 1.4, Sep 2025), §2 Pin Definition.
Board: ESP32-S3R2 + SX1262, 51.7 × 25.4 × 10.7mm, 2× 18-pin 2.54mm headers.

## Orientation

**Pin 1 of both headers is at the USB-C end.** Pin 18 is at the antenna/OLED end.
Viewed from the front (OLED facing you, USB-C down): **J3 is the left row, J2 is the right row.**

Confirmed empirically during bringup 2026-09-05, and matches the pin-layout figure on p.7.

> ⚠️ **Bench-wiring hazard.** J2 pin 1 is GND but **pin 2 is 5V**. An off-by-one on J2 puts 5V
> through your circuit onto a 3.3V-max GPIO. J3 is safer for hand-wiring — its pin 1 neighbors
> are 3V3. Symptom of getting this wrong: the OLED blanks on button press (rail brownout).
> Confirm the ground pin by continuity to the USB-C shell before powering on.

## Header J3 (left row)

| Pin | GPIO | Analog/RTC | Special function |
|---:|---|---|---|
| 18 | GPIO7 | ADC1_CH6, TOUCH7 | **V_FEM_Control** |
| 17 | GPIO6 | ADC1_CH5, TOUCH6 | — |
| 16 | GPIO5 | ADC1_CH4, TOUCH5 | — |
| 15 | GPIO4 | ADC1_CH3, TOUCH4 | — |
| 14 | GPIO3 | ADC1_CH2, TOUCH3 | — (⚠️ ESP32-S3 strapping: JTAG source select) |
| 13 | GPIO2 | ADC1_CH1, TOUCH2 | figure says `FEM_EN`, table does not — see note below |
| 12 | GPIO1 | ADC1_CH0, TOUCH1 | **VBAT_Read** ¹ |
| 11 | GPIO38 | — | GNSS_RX (FSPIWP, SUBSPIWP) |
| 10 | GPIO39 | — | GNSS_TX (MTCK) |
| 9 | GPIO40 | — | GNSS_Wakeup (MTDO) |
| 8 | GPIO41 | — | GNSS_PPS (MTDI) |
| 7 | GPIO42 | — | GNSS_RST (MTMS) |
| 6 | GPIO45 | — | ⚠️ ESP32-S3 strapping (VDD_SPI select) |
| 5 | GPIO46 | — | ⚠️ ESP32-S3 strapping (boot mode) |
| 4 | GPIO37 | — | **ADC_Ctrl** (SPIDQS, FSPIQ, SUBSPIQ) |
| 3 | 3V3 | Power | 3.3V supply ² |
| 2 | 3V3 | Power | 3.3V supply ² |
| 1 | GND | Ground | — |

¹ Per datasheet footnote: ADC1_CH0 reads the lithium battery voltage; **ADC_CTRL (GPIO37) must be
pulled high** to enable it. `VBAT = 100 / (100+390) * VADC_IN1`. Because it is gated, GPIO1 is
usable as an ordinary input — verified working as a button input 2026-09-05.

² ⚠️ J3 3V3 is a **2.7–3.5V input/output rail**. Never feed it raw LiPo. See the power notes.

### Note: GPIO2, FEM_EN, and a false alarm

Heltec's document disagrees with itself about GPIO2:

- The **pin-layout figure (p.7)** labels GPIO2 as **`FEM_EN`** (and separately labels GPIO7 as
  `VFEM_Ctrl`).
- The **pin-description table (p.9)** lists GPIO2 as only `ADC1_CH1, TOUCH2` — **no FEM_EN** —
  while it does list `V_FEM_Control` on GPIO7. That table names every other special function in
  red (GNSS_*, VBAT_Read, ADC Ctrl), so the omission is conspicuous.

**GPIO2 is usable as an ordinary pulled-up input.** Verified 2026-09-06 on a healthy board
(MAC `F8:5B:1B:A2:C6:2C`): a rotary encoder on GPIO3/2/1 produces all four quadrature states
and a correctly accumulating count.

The confusion arose because one board (MAC `90:70:69:85:F2:50`) reads GPIO2 **stuck low** — 240
transitions, never rising, even with nothing connected and `Pull::Up` set. That board had
survived a power-rail short from a mis-wired header hours earlier; the pin is damaged. The p.7
`FEM_EN` label made a plausible-sounding root cause for what was simply a dead input.

**Lesson: reproduce a pin failure on a second board before treating it as a design constraint.**
Note too that GPIO7 — which the table says carries FEM control — works fine as a pulled-up input,
so a FEM-related function does not by itself make a pin unusable.

## Header J2 (right row)

| Pin | GPIO | Analog/RTC | Special function |
|---:|---|---|---|
| 18 | GPIO19 | Analog, RTC | ⚠️ **USB_D-** (U1RTS, ADC2_CH8) |
| 17 | GPIO20 | Analog, RTC | ⚠️ **USB_D+** (U1CTS, ADC2_CH9) |
| 16 | GPIO21 | RTC | **OLED_RST** |
| 15 | GPIO26 | — | ⚠️ SPICS1 (SPI flash region) |
| 14 | GPIO48 | — | SPICLK_N_DIFF, SUBSPICLK_N_DIFF |
| 13 | GPIO47 | — | SPICLK_P_DIFF, SUBSPICLK_P_DIFF |
| 12 | GPIO33 | — | SPIIO4, FSPIHD, SUBSPIHD |
| 11 | GPIO34 | — | **VGNSS_Ctrl** (SPIIO5, FSPICS0) |
| 10 | GPIO35 | — | **LED** (SPIIO6, FSPID) |
| 9 | GPIO36 | — | **Vext_Ctrl** (SPIIO7, FSPICLK) — LOW = peripherals on |
| 8 | GPIO0 | — | **PRG button** |
| 7 | RST | Input | CHIP_PU, reset button |
| 6 | GPIO43 | — | ⚠️ U0TXD (serial console) |
| 5 | GPIO44 | — | ⚠️ U0RXD (serial console) |
| 4 | Ve | Power | 3.3V output (gated by Vext_Ctrl) |
| 3 | Ve | Power | 3.3V output (gated by Vext_Ctrl) |
| 2 | **5V** | Power | ⚠️ 5V supply |
| 1 | GND | Ground | — |

## Not on the headers

**LoRa (SX1262), internal:**

| GPIO | Function | GPIO | Function |
|---|---|---|---|
| GPIO8 | LoRa_NSS | GPIO12 | LoRa_RST |
| GPIO9 | LoRa_SCK | GPIO13 | LoRa_BUSY |
| GPIO10 | LoRa_MOSI | GPIO14 | DIO1 |
| GPIO11 | LoRa_MISO | | |

**Additional pins (§2.2.3):** GPIO18 = OLED_SCL · GPIO17 = OLED_SDA ·
GPIO16 = XTAL_32K_N · GPIO15 = XTAL_32K_P

**GNSS connector (SH1.25, 8-pin):** GND · GPIO34 (VGNSS_Ctrl) · 3V3 · GPIO39 (GNSS_TX) ·
GPIO38 (GNSS_RX) · GPIO40 (GNSS_Wakeup) · GPIO41 (GNSS_PPS) · GPIO42 (GNSS_RST)

## Our assignments (`pcb/board.py`)

| Net | GPIO | J3 pin | Status |
|---|---|---:|---|
| CHNL_A | GPIO7 | 18 | ✅ verified |
| CHNL_B | GPIO6 | 17 | ✅ verified |
| CHNL_SW | GPIO5 | 16 | ✅ verified on a bare Heltec; rev4 board has a solder bridge |
| MIC_OUT | GPIO4 | 15 | ADC1_CH3 |
| VOL_A | GPIO3 | 14 | ✅ verified |
| VOL_B | GPIO2 | 13 | ✅ verified (see GPIO2 note — one damaged board caused a false alarm) |
| VOL_SW | GPIO1 | 12 | ✅ verified |

| Net | GPIO | J2 pin |
|---|---|---:|
| LRC | GPIO48 | 14 |
| BCLK | GPIO47 | 13 |
| DIN | GPIO33 | 12 |
| PTT | GPIO0 | 8 |
| Ve | Ve | 3, 4 |

J3 3V3 (pins 2/3) is deliberately **left disconnected** — wiring it would back-feed the regulated
rail. J2 5V (pin 2) is likewise disconnected. See the power architecture notes.

**Free for reassignment:** GPIO38–42 (J3 pins 11–7, currently reserved for GNSS) and GPIO26
(J2 pin 15, SPI-flash region — risky). Avoid GPIO45/46 (strapping) and GPIO34/35/36 (driven
by the Heltec).
