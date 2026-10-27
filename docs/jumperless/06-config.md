# Config File

To change any persistent settings that apply to the Jumperless as a whole, there's a `config` file. There are three ways to poke at it, pick whichever fits your mood:

1. **The interactive editor** - enter a bare `` ` `` (backtick) and get a full menu with arrow keys, descriptions, and live-updating values
2. **Copy / paste** - print the config with `~`, copy a line, change the value, paste it back
3. **Edit the file** - it's just `config.txt` on the filesystem, edit it however you like; those edits get read at the next boot

## The Interactive Editor

Enter a single `` ` `` (backtick, nothing else) into the terminal and you get a config menu instead of a config file:

![The interactive config editor's category list](assets/config-tui-categories.png)

The categories are organized by what you're actually trying to mess with (`Probe`, `Clickwheel`, `Measurement`, `Display`...), with the plumbing (`Calibration`, `Hardware`) further down, and `All` at the top if you'd rather scroll one flat list of everything.

The keys:

- `up` / `down` - move (the pane on the right describes whatever you're on: what it does, its range, its default)
- `enter` - open a category, or highlight the value of the option you're on (text options like `startup_message` go straight to typing)
- `left` / `right` - step a highlighted value, cycle its choices, or flip it on/off; while you're just moving around, `left` is back and `right` opens/selects like `enter`
- `e` - type an exact value
- `d` - reset the highlighted option to its default
- `enter` again - done with the value
- `left` or `q` - back / exit

`j` / `k` move too, `PgUp` / `PgDn` page, `Home` / `End` jump to the ends, and `esc` backs out like `left`.

![Inside the Probe category](assets/config-tui-probe.png)

**Most changes apply live.** Every edit goes through the same machinery as the paste path, so the board reacts while you're still in the menu - hit `enter` on `led_brightness` and hold `right`, and the breadboard gets brighter under your finger, change the OLED font and it redraws, tweak the menu FX and the next transition wears it. The exceptions only take effect on the next boot: the `[hardware]` keys, `probe.use_pio_button`, `probe.led_on_button_pin`, `clickwheel.encoder_pio`, and the `[usb_audio]` keys (turning `usb_audio.enabled` on live re-enumerates USB and drops the serial ports for about 2 seconds).

![The Display category - brightness sliders that apply as you arrow through them](assets/config-tui-display.png)

There's also a `Menu FX Tuner` category at the bottom that opens a live tuner for the click-menu frame transitions (it drives the real breadboard menu while you play with it), a `Reset to defaults` that resets everything except calibration and hardware identity, and an `Exit` row that closes the menu (`left` and `q` do too).

## Viewing Config.txt

If you'd rather see the whole thing at once, enter `~` to print the config. 

```jython
~

copy / edit / paste any of these lines 

into the main menu to change a setting

Jumperless Config:

`[config] firmware_version = 5.7.11.0;

`[hardware] generation = 5;
`[hardware] revision = 7;
`[hardware] probe_revision = 5;
`[hardware] psram_installed = false;
`[hardware] psram_app_size_kb = 2048;

`[probe] auto_connect = on;
`[probe] power_source = dac0_first;
`[probe] use_pio_button = true;
`[probe] led_on_button_pin = true;
`[probe] led_refresh_us = 0;
`[probe] pad_max = 4056;
`[probe] pad_min = 15;
`[probe] pad_max_measure = 4111;
`[probe] pad_max_measure_gpio = 4110;
`[probe] pad_min_measure = 10;
`[probe] switch_threshold_high = 1.2000;
`[probe] switch_threshold_low = 0.9000;
`[probe] switch_select_max_ma = 0.0000;
`[probe] switch_blink_hold_pct = 50;
`[probe] measure_voltage = 3.3671;
`[probe] current_zero = 2.1667;
`[probe] min_valid_reading = 85;
`[probe] droop_v0 = 3.3500;
`[probe] droop_ohms = 169.6479;
`[probe] pad_ohms = 5.0000;

`[clickwheel] encoder_pio = auto;
`[clickwheel] rail_click_adjust = oled_only;
`[clickwheel] part_walk = z;
`[clickwheel] fx_type = glow;
`[clickwheel] fx_duration_ms = 160;
`[clickwheel] fx_tint = 0x00;
`[clickwheel] fx_density = 128;

`[measurement] net_currents = 1;
`[measurement] current_flow = conventional;
`[measurement] show_probe_current = 0;
`[measurement] crosspoint_resistance = 40.0000;

`[terminal] colors = true;
`[terminal] line_buffering = true;

`[undo] persist = true;
`[undo] max_saved_actions = 256;

`[dacs] limit_max = 8.00;
`[dacs] limit_min = -8.00;

`[debug] file_parsing = false;
`[debug] net_manager = false;
`[debug] nets_to_chips = false;
`[debug] nets_to_chips_alt = false;
`[debug] probing = false;
`[debug] arduino = 0;
`[debug] show_node_errors = true;
`[debug] probe_switch_stats = false;
`[debug] probe_switch_agree = false;
`[debug] net_voltage_scan = false;
`[debug] net_scan_pair_taps = 1;

`[routing] stack_paths = 2;
`[routing] stack_rails = 3;
`[routing] stack_dacs = 0;
`[routing] stack_gpio = 0;
`[routing] stack_adcs = 0;
`[routing] part_safety = off;

`[slots] boot_mode = last_active;
`[slots] boot_slot = 0;

`[calibration] top_rail_zero = 1655;
`[calibration] top_rail_spread = 18.4000;
`[calibration] bottom_rail_zero = 1639;
`[calibration] bottom_rail_spread = 19.0000;
`[calibration] dac_0_zero = 1655;
`[calibration] dac_0_spread = 18.2300;
`[calibration] dac_1_zero = 1629;
`[calibration] dac_1_spread = 19.0300;
`[calibration] adc_0_zero = 8.9969;
`[calibration] adc_0_spread = 18.0563;
`[calibration] adc_1_zero = 8.9903;
`[calibration] adc_1_spread = 18.0659;
`[calibration] adc_2_zero = 8.9993;
`[calibration] adc_2_spread = 18.0637;
`[calibration] adc_3_zero = 8.9826;
`[calibration] adc_3_spread = 18.0345;
`[calibration] adc_4_zero = 0.0000;
`[calibration] adc_4_spread = 4.9151;
`[calibration] adc_7_zero = 8.8184;
`[calibration] adc_7_spread = 17.7676;

`[logo_pads] top_guy = uart_tx;
`[logo_pads] bottom_guy = uart_rx;
`[logo_pads] building_pad_top = isense_pos;
`[logo_pads] building_pad_bottom = isense_neg;
`[logo_pads] top_guy_idle = off;
`[logo_pads] bottom_guy_idle = off;
`[logo_pads] building_pad_top_idle = off;
`[logo_pads] building_pad_bottom_idle = off;

`[display] lines_wires = wires;
`[display] menu_brightness = -10;
`[display] led_brightness = 10;
`[display] rail_brightness = 55;
`[display] special_net_brightness = 20;
`[display] net_color_mode = rainbow;

`[serial_1] function = passthrough;
`[serial_1] baud_rate = 115200;
`[serial_1] print_passthrough = 0;
`[serial_1] connect_on_boot = 0;
`[serial_1] lock_connection = 0;
`[serial_1] autoconnect_flashing = 1;
`[serial_1] async_passthrough = true;
`[serial_1] tag_parsing = enabled;
`[serial_1] flash_reset_type = avr;

`[serial_2] function = micropython;
`[serial_2] baud_rate = 115200;
`[serial_2] print_passthrough = 0;
`[serial_2] connect_on_boot = 0;
`[serial_2] lock_connection = 0;

`[top_oled] enabled = 1;
`[top_oled] i2c_address = 0x3C;
`[top_oled] width = 128;
`[top_oled] height = 32;
`[top_oled] rotation = 0;
`[top_oled] connection_type = i2c0;
`[top_oled] sda_pin = 4;
`[top_oled] scl_pin = 5;
`[top_oled] gpio_sda = 134;
`[top_oled] gpio_scl = 135;
`[top_oled] sda_row = -1;
`[top_oled] scl_row = -1;
`[top_oled] connect_on_boot = 1;
`[top_oled] lock_connection = 0;
`[top_oled] show_in_terminal = off;
`[top_oled] font = Eurostl;
`[top_oled] startup_message = images/bubbleJumpThin.bin;

`[usb_cdc] ignore_dtr = false;

`[usb_audio] enabled = false;
`[usb_audio] left = 0;
`[usb_audio] right = 1;
`[usb_audio] rate = 16000;
`[usb_audio] full_scale = 8.00;
`[usb_audio] dc_block = true;
```


This is just a file on your filesystem called `config.txt` and you can edit that file directly too. It only gets read at boot, so your edits land on the next reboot - and the firmware rewrites `config.txt` from its live settings whenever a setting changes, so edit it with the board idle and reboot afterwards. If you want the change right now, use the editor or paste the line back in.

![config.txt open in a text editor on a computer, with the Jumperless mounted as a drive](https://github.com/user-attachments/assets/bb6a9d29-3d85-46e4-99e7-ae023c4be754)

The `[calibration]` numbers come from `Calibration` > `DACs Calib` on the click wheel. It runs by itself on first startup, and you can run it again if the DAC or ADC voltages look off (it sets a bunch of DAC voltages and reads them back with the INA219 to work out the zero and spread for each one). The probe's own calibration (`pad_min`, `pad_max*`, the switch thresholds) is in `[probe]` - the [Calibration](05.8-calibration.md) page covers what sets each one.

## Config Help

There's also a `help` you can get to by entering `~help`

```jython
~help

                              Read config 
                          ~ = show current config
                     ~names = show names for settings
                   ~numbers = show numbers for settings
                 ~[section] = show specific section (e.g. ~[routing])

                              Write config 
`[section] setting = value; = enter config settings (pro tip: copy/paste setting from ~ output and just change the value)
                          ` = open the interactive config menu (arrow keys)

                              Reset config
                     `reset = reset to defaults (keeps calibration and hardware version)
            `reset_hardware = reset hardware settings (keeps calibration)
         `reset_calibration = reset calibration settings (keeps hardware version)
                 `reset_all = reset to defaults and clear all settings
         `force_first_start = clears everything to factory settings and runs first startup calibration
                 `self_test = run the hardware self test (non-destructive)
          `self_test_report = re-print the stored self test report

                              Help
                         ~? = show this help
```

(Small quirk: `~?` currently shows a shorter one-line command card instead; `~help` is the one that prints all of the above.)

## States vs Config

States are now saved as YAML and we did away with the old text file format. Each slot's YAML file holds one complete circuit - connections, colors, rail and DAC voltages, GPIO setup. 

**State vs Config - What's the Difference?**

- **State** stores things relevant to the currently loaded slot - connections, wire colors, rail voltages, GPIO settings. These change when you switch slots.
- **Config** (config.txt) contains hardware-wide settings that apply to the entire Jumperless regardless of which slot is active.

Rail voltages, GPIO settings, and other circuit-specific parameters now go with the state (in the YAML file) rather than config, so each slot can have its own power supply and GPIO configuration.

## State File Structure

For things specific to the current `state` of the Jumperless, there's a YAML file that contains all the connections, colors (optional), `rail` / `DAC` voltages, `GPIO` directions and pulls, stuff like that. The idea is this defines a complete setup of a particular circuit that can be switched between in different `slots`. 

The Jumperless boots into whichever `slot` was last active (`[slots] boot_mode = last_active`); set `boot_mode = fixed_slot` and `boot_slot = N` if you'd rather it always come up in slot N. You can switch to other `slots` with `<` (cycle through them) or `<5` (jump straight to slot 5), or by selecting one with the `click menu` with `Slots` > `Load` > `0-7` (it will show a preview of each one.) To save a copy of the currently `active slot`; 
`Slots` > `Save to` > `0-7` will save a copy of the `active slot` to another `slot` and also make that target slot the `active`. `Slots` > `Clear` > `0-7` deletes a `slot`'s file.


Enter `Y` to print the current state (`Y0` plain, `Y1` colored hex, `Y2` colored blocks). It prints the YAML itself, with a `Time since last change` line on top if you have unsaved edits. `S` loads a pasted state back in, in the same format.

```jython
Y

version: 2
sourceOfTruth: bridges

bridges:
  - {n1: 38, n2: 44}
  - {n1: 21, n2: 28}
  - {n1: 48, n2: 55}
  - {n1: 4, n2: 2, dup: 2}

nets:
  - {num: 4, nodes: [DAC_0, BUF_IN], name: "DAC 0", anim: true}
  - {num: 6, nodes: [38, 44], color: pink}
  - {num: 7, nodes: [21, 28], color: blue}
  - {num: 8, nodes: [48, 55], color: green}
  - {num: 9, nodes: [4, 2], color: amber}

power:
  topRail: 3.30
  bottomRail: 2.50
  dac0: 3.30
  dac1: 0.00

config:
  routing: {stackPaths: 2, stackRails: 3, stackDacs: 0, railPriority: 1}
  gpio:
    direction:    [1,1,1,1,1,1,1,1,1,1]
    pulls:        [0,0,0,0,0,0,0,0,0,0]
    pwmFrequency: [1.00,1.00,1.00,1.00,1.00,1.00,1.00,1.00,1.00,1.00]
    pwmDutyCycle: [0.50,0.50,0.50,0.50,0.50,0.50,0.50,0.50,0.50,0.50]
    pwmEnabled:   [0,0,0,0,0,0,0,0,0,0]
    readFloating: [0,0,0,0,0,0,0,0,0,0]
  uart: {txFunction: 0, rxFunction: 1}
  oled: {connected: false, lockConnection: false}
  bcd: {pins: 0, value: 0}
```

`dup:` only shows up on connections where you asked for a particular stack count. If you've placed parts, the file also carries a `parts:` section, and a `guideProgress:` line while you're partway through a guided project.

## Source of Truth

Because the information in here is *sort of* redundant (the connections could be computed from just the `bridges` or the `nets` section on their own), there's a field called `sourceOfTruth` which is the section that actually gets parsed and then the other section is written with the computed values. (I haven't done much testing on changing this to `nets` so I'd probably just leave it on `bridges` for now.)

There's some weirdness with how colors are applied, since the `source of truth` is the bridges, and the things that actually get colored are the `nets`, it'll take the colors from the `bridges` (if specified) and try to apply them to the `nets`. But since nets always have a single color (to show that they're connected), if you have `bridges` with different colors in the same `net`, it'll just pick one (don't ask me exactly how the logic chooses, idk.) 

If you specify a color to a `net` even with `sourceOfTruth: bridges` it should respect the `net` assignment over the `bridge` assignment.

## Switching Between Saved Circuits (Slots)

The Jumperless has **8 slots** (0-7) where you can save different circuit configurations. Think of them like presets or save files.

**Quick slot cycling:**
- Type `<` in the terminal to cycle to the next slot

**Other slot commands:**
- `<5` - Jump straight to slot 5
- `Q` - Query which slot is currently active

To browse the slots and see a preview of each one, use the `click menu`: `Slots` > `Load` > `0-7`.

When you make connections with the probe, they're automatically saved to whichever slot is currently active. See the [Glossary](99-glossary.md) for more details about slots.

## Editing State Files

You can edit the YAML slot files and the board will pick up your changes - see [Editing Slot Files](08-file-manager.md#editing-slot-files).
