# Mic input: bench findings for rev 11 (2026-11-02)

Bench: Jumperless V5 + AD3, MAX9814 clone board (capsule removed) at 50 dB, rev 10 board's K1 jacks.
Controlled source: a USB speaker 150 mm from both mics plays the same file (1 kHz at -6 and -26 dBFS, then speech).
Levels were read at the MAX9814 INPUT node (the raw mic, before the AGC) with a 1 kHz lock-in. Every direct condition
ran twice: the repeats agreed within 1.4% (BTECH) and 8% (capsule).

## Results

| Condition (input node, mV rms) | 1 kHz tone | speech | noise |
|---|---|---|---|
| 9x7 capsule, direct | 6.34 / 6.86 | 0.90 | 0.28 |
| 9x7 capsule through the 3.5 jack ring NC switch | 6.39 | 0.91 | 0.34 |
| same + 1.5k / 10 uF from capsule + to GND | 3.25 | 0.45 | 0.18 |
| BTECH lapel mic, direct | 0.70 / 0.71 | 0.18 | 0.12 |
| BTECH through the plugged jack, capsule + shunt on the NC side | 0.75 | 0.19 | 0.12 |

- The ring NC switch works: plug in = NC contact open, headset mic on the ring (~1.1 kohm DC); plug out = closed
  (18-37 ohm through the crosspoints). The switch path costs nothing, and parts on the NC (capsule) side do not touch
  the headset path. A partly inserted plug opens the switch without connecting the headset mic.
- Capsule polarity: the + side current-limits at ~0.22 mA (the FET), the reverse direction conducts (~900 ohm).
- The 1.5k / 10 uF shunt attenuates the internal capsule by 6 dB.
- The capsule delivers 19 dB (tone) / 16 dB (speech, noise subtracted) more than the BTECH at equal distance.
- Never compare levels after the MAX9814: its AGC sets the gain per condition (31 vs 36 dB at the "50 dB" setting).

## Why the BTECH is weak here: bias

Through the MAX9814's 2.2k from its ~2 V MICBIAS, the capsule sits at 1.62 V (0.24 mA), but the BTECH element draws
0.60 mA and sits at only 0.82 V: its FET is starved.

OpenVLM (`docs/reference/openvlm/VLMKW_V1R0_schematic.png`) biases MIC+ from the CM108's VBIAS = 4.5 V
(`docs/cm108/CM108_DataSheet_v1.6.pdf`, pin 25), decoupled 100 nF + 10 uF, through only 470 + 220 ohm (MICIN tapped
between them via 1 uF), with 10k from MIC+ to GND and 1 nF + 470 pF at MIC+ for RF. Gain comes from the codec
(+20 dB boost + 0-22.5 dB). Active K1 PTTs (Disco32) very likely depend on this stiff, high bias on the ring to power
their electronics and drive headsets (e.g. Peltor) that were never designed for it.

## Rev 11 direction

- Headset (ring) side: an OpenVLM-style bias, ~4.5 V, a few hundred ohm, well decoupled, plus an RF capacitor at the jack.
- Internal capsule on the ring NC contact, with its own level-matching shunt on the NC side.
- Gain from the amp/codec (MAX9814 or the ES8388's mic PGA, `docs/es8388/`).
- Not yet tested: the BTECH at a higher bias (planned: AD3 V+ 4.5 V through 1.5k, AC-coupled into the MAX9814).
