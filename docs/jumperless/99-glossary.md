# Glossary of Terms

## Basic Concepts

`net` = a group of all the `node`s that are connected together (enter `n` to see the list)

`node` = anything the crossbar array can connect to, which includes everything on the breadboard and Nano header, as well as the internal `special function` `node`s like `routable GPIO`, `ADC`s, `DAC`s

`row` = *kinda* the same thing as `node` but I generally use it to mean stuff on the breadboard (so special function things like `routable GPIO`, `ADC`s, `DAC`s that don't have a set location are excluded)

`rail` = I use this to refer to the 4 horizontal power rails on the top and bottom (`top_rail`, `bottom_rail`, `gnd`), I will never call a vertical `row` a `rail`. (I know they're columns but it's easier to say a lot)

`bridge` = a pair of exactly two `node`s (this is what you're making when you connect stuff with the probe, enter `b` to see the bridge array)

`path` = the set of crossbar connections needed to make a single `bridge`, so it can have multiple `hop`s if it doesn't have a direct connection and needs to make a `bounce` through an intermediate `chip` (enter `c` to see the crossbar array)

## Hardware

`chip` = shorthand for the CH446Qs specifically, lettered A-L. The first 8 (A-H) are considered "breadboard `chips`", and the last 4 (I-L) are considered "special function" chips (enter `c` to see their connections)

`menu` = I generally mean the onboard clickwheel `menu` when I say this (`click` the wheel to enter those and `scroll` around.) Sometimes I mean the `main menu` which is the list of single character command that gets presented over serial.


## Slots and Files

`slot` = one of **8** saved circuit configurations (slots 0-7) that you can switch between. Type `<` to cycle forward through slots or `<5` to jump straight to one, or on the clickwheel use `Slots` > `Load` > `0-7` (it previews each one as you scroll). `Slots` > `Save to` > `0-7` copies the active slot into another, and `Slots` > `Clear` > `0-7` empties one.

`slot file` = a YAML file on the filesystem that stores a complete circuit configuration including bridges, power settings, and colors. Located at `/slots/slotN.yaml` where N is 0-7. These files are human-readable and can be edited directly! See [State File Structure](06-config.md#state-file-structure) for what's in one.

`active slot` = the context that's actually loaded onto the hardware. Usually that's one of the numbered slots, but it can also be a project's `wiring.yaml` opened from the Files browser or with `z <project>`. Enter `Q` to see which. When you make connections with the probe, they're saved to whatever's active automatically.

## Slot Management Commands

- `<` = cycle to next slot (0→1→2...→7→0), or `<N` (e.g. `<5`) to jump straight to slot N. From a project file, a bare `<` goes to slot 0.
- `Q` = query which context is active. It prints `ACTIVE_SLOT:N` and then `ACTIVE_PATH:/slots/slotN.yaml`. With a project file loaded the slot is `-1` and the path is that file's.
- `Y` = print YAML (`Y0` plain, `Y1` colored with hex values, `Y2` colored blocks, which is the default)
- `S` = paste a YAML state back in, in the same format `Y` prints. End the paste with an empty line and it's applied right away.

## YAML Format

Slot files use YAML format with named nodes for readability. [State File Structure](06-config.md#state-file-structure) has a full example of one.

`dup` is an optional per-bridge override of how many parallel crosspoint paths get stacked for that one connection, like `- {n1: NANO_D5, n2: GP_1, dup: 4}`. Leave it out and you get the default from the `[routing]` `stack_paths` key (2), with `stack_rails` (3), `stack_gpio` (0), `stack_adcs` (0) and `stack_dacs` (0) covering those classes (the resistance math behind stacking is in [this Crowd Supply update](https://www.crowdsupply.com/architeuthis-flux/jumperless-v5/updates/a-farewell-to-campaigns)). `color:` is optional too, and takes a color name, `0xRRGGBB`, or `#RRGGBB`.

**Named nodes:** `NANO_D0-D13`, `NANO_A0-A7`, `GP_1-8` (or `RP_GPIO_1-8`), `TOP_RAIL`, `BOTTOM_RAIL`, `GND`, `DAC0`, `DAC1`, etc. (Note these differ from the MicroPython constants - `GPIO_1` and `DAC0_5V` won't parse in slot files.)

You can view and edit these files in the [File Manager](08-file-manager.md) or via USB Mass Storage mode (`U` command).

## Wokwi Import

`W` = import a circuit from the [Wokwi](https://wokwi.com) simulator into the active slot. `W 3` imports into slot 3 instead (written to its file, hardware untouched unless 3 is the active one), `W diagram.json` reads a diagram already on the filesystem, and `W diagram.json 3` does both. When you're pasting, it knows you're done when the closing brace arrives.

The whole walkthrough is on the [app page](03-app.md#importing-circuits-from-wokwi).

