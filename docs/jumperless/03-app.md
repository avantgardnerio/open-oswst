# The App

## Installation guide

### The Jumperless App is now on PyPi!

The easiest way to get started is an isolated tool install:

```bash
uv tool install jumperless
# or: pipx install jumperless
```

Plain `pip install jumperless` works too, it just only updates the copy in that Python.

Then run it with:

```bash
jumperless
```

To upgrade, use the command that matches the install you have:

```bash
uv tool upgrade jumperless
pipx upgrade jumperless
pip install --upgrade jumperless
```

If the version still looks old, `which jumperless` will tell you which install you're actually running.

The app repo is at [https://github.com/Architeuthis-Flux/Jumperless-App](https://github.com/Architeuthis-Flux/Jumperless-App)

### Alternative: Download Pre-built Binaries

#### Find the latest release
[https://github.com/Architeuthis-Flux/JumperlessV5/releases/latest](https://github.com/Architeuthis-Flux/JumperlessV5/releases/latest)

The link above will magically lead you to the latest version, and will look something like `https://github.com/Architeuthis-Flux/JumperlessV5/releases/tag/5.7.11.0`

**At the bottom under Assets, download the Jumperless App for your OS**

### Windows

  - `Jumperless-Windows-x64.exe`
  
### macOS

  - `Jumperless-macOS.dmg` (universal, signed and notarized)
  
### Linux

  - `Jumperless-Linux-x86_64.AppImage` (if you're not sure which flavor of Linux, use this one)
  - `Jumperless-Linux-x64.tar.gz` as a fallback if the AppImage won't run

### Launcher

  - `Jumperless-Launcher-Windows.zip`, `Jumperless-Launcher-macOS.zip`, `Jumperless-Launcher-Linux.zip` (see Launch Scripts below)
  
### Python

  1. download `JumperlessWokwiBridge.py` and `requirements.txt`
  2. open your favorite terminal, navigate to the folder where you downloaded the two files above.
  3. `pip install -r requirements.txt` # run this command to install the needed Python libraries
  4. `python3 JumperlessWokwiBridge.py` # open the app, will update firmware if there's a newer version

---

Now that I've lifted my self-imposed ban on VT100 commands (for compatibility and me-spending-too-much-time-on-them reasons, but, YOLO), we've got colors now! 

<img width="749" alt="Screenshot 2025-05-29 at 10 01 15 PM" src="https://github.com/user-attachments/assets/a0fbbca6-ec16-4a0e-ac36-b4ed1f46663a" />


But that's like the *least* cool thing the new app can do, here's a list of what's new:

## What It Does


- **Firmware updating** should be pretty reliable when there's a new version (falls back to instructions for how to do it manually)
- **Command history and tab completion**, up arrows will go through past commands and are persistent after closing
- **Properly detects** which port is the main Jumperless Serial and which is routable UART
- **Arduino flashing from [Wokwi](https://wokwi.com/)** works once again and is a lot more solid
  - It installs [arduino-cli](https://github.com/arduino/arduino-cli) on first startup and uses it pull in libraries, compile, and flash an arduino Nano in the header
  - When a flash starts, the Jumperless itself connects the UART to the Nano's D0/D1 if they aren't already connected. That happens for any flasher (the app, Arduino IDE, arduino-cli) because the board just watches for the DTR reset pulse, and it's the `[serial_1] autoconnect_flashing` config setting, on by default. The connections are left in place afterwards, use `a` if you want them gone
  - [avrdude](https://github.com/avrdudes/avrdude) output is shown in real time (you'd be amazed how difficult this was)
- **Direct Wokwi circuit import** - Copy diagram.json from Wokwi and import it with the `W` command (see below)
- **No longer a janky pile of garbage**
## Local Arduino Sketch Support

**You can set a `slot` to point to a local Arduino sketch.ino file and it will flash if it detects a change** 

- If you don't like using Arduino IDE or Wokwi and prefer using `vim` or `emacs` or whatever, now you can let the app handle the flashing stuff and just edit an .ino file.
- In the app, type `menu` then `slots` and instead of entering a link to a Wokwi project, just give it a path to a file (this will be saved so you can unassign it and pick it later by name)
- (This one is so fucking sick) 

<img width="1330" alt="Screenshot 2025-05-29 at 9 16 14 AM" src="https://github.com/user-attachments/assets/766dbb09-254e-45c5-8f75-358684729907" />

## Launch Scripts

- The launcher bundles (`Jumperless-Launcher-Windows/macOS/Linux.zip`) are a tiny clickable app that installs or updates the `jumperless` PyPI package with uv and then runs it in the system default terminal (Terminal.app on macOS, Windows Terminal or cmd on Windows, gnome-terminal/konsole/xfce4-terminal/xterm on Linux)
- It doesn't kill or close other instances that happen to be open
- To run it in your favorite terminal emulator instead ([tabby](https://tabby.sh/), iTerm2, whatever), just open that terminal and type `jumperless` (or `python3 JumperlessWokwiBridge.py` if you downloaded the script)
- Linux people are no longer red-headed stepchildren, there's a single-file AppImage now for you nerds, with a tar.gz as a fallback

---

## Importing Circuits from Wokwi

You can design circuits in the [Wokwi online simulator](https://wokwi.com) and import them directly to your Jumperless with the `W` command, or use the Jumperless App and it'll pull it from your project automatically and live update. This has been around since the OG Jumperless, there's a demo in [this Crowd Supply update](https://www.crowdsupply.com/architeuthis-flux/jumperless-v5/updates/jumperless-probelessly).

### Direct Link Import

You can now just dump a Wokwi link into the app at any time and it'll work:

```
		Menu
~~~~~
	x = clear all connections
	+ = add connections
	- = remove connections

https://wokwi.com/projects/424432011346848769


Enter a name for this new project: cool project zone
✓ Saved 'cool project zone' to project library

✓ 'cool project zone' assigned to active slot 0
  URL: https://wokwi.com/projects/424432011346848769
  The project will start updating automatically
```

### How to manually Import from Wokwi

1. **Design your circuit** on [wokwi.com](https://wokwi.com)
2. **Click on the `diagram.json` tab** in the Wokwi editor
3. **Copy all the JSON content** (Ctrl+A, Ctrl+C or Cmd+A, Cmd+C)
4. **In Jumperless, type `W`** and press Enter
5. **Paste the JSON** (Ctrl+V or right-click → Paste)
6. The parser automatically detects when the JSON is complete and imports it!

### Supported Wokwi Components

- **Half breadboard** - Wokwi's breadboard maps directly to Jumperless rows. Top rows 1-30 are rows 1-30, bottom rows 1-30 are rows 31-60. The column letter is ignored, all five holes in a row are one node anyway. The top `+` rail maps to `TOP_RAIL`, the bottom `+` rail to `BOTTOM_RAIL`, and both `-` rails to `GND`
- **Arduino Nano** - D0-D13 and A0-A7 map to the Nano header nodes. GND maps to the `GND` net, 5V (or VCC) to `TOP_RAIL`, and 3V3 to `BOTTOM_RAIL`, so wires to those pins do make real connections. RESET, AREF, VIN and RX/TX are ignored
- **Logic Analyzer** - Channels map to GPIO: D0-7 → GPIO 1-8
- **Wire colors** - Wokwi wire colors preserved
- **Rail voltages** - Detected from text labels in Wokwi
- **VCC and GND Nodes** - VCC maps to `TOP_RAIL`, GND maps to `GND`

![LogicAnalyzerMappingV5](https://github.com/user-attachments/assets/3b7bd360-9703-4b0b-925a-aea8ed7e0526)

**Note:** The app still works with the OG Jumperless and those original mappings remain the same.

### Wire Color Mapping

**Wire colors will match the ones you set in Wokwi!** The new Wokwi parser sends the entire `diagram.json` from Wokwi and parses it on the Jumperless, which means color information gets preserved.

![wokwiColor-2](https://github.com/user-attachments/assets/e5607cf9-3a95-42f7-b67c-875ba23e2ee9)

![wokwiColor-1](https://github.com/user-attachments/assets/8a148940-60f8-4741-8905-6b9911ac1f21)

All Wokwi wire colors are preserved and displayed on the breadboard LEDs:

`red`, `orange`, `yellow`, `green`, `blue`, `violet`, `purple`, `magenta`, `pink`, `cyan`, `white`, `gray`, `grey`, `black`, `brown`, `limegreen`, `gold`

A couple of them land on the same LED color: `gold` looks the same as `orange`, and `pink` the same as `magenta`. A color name that isn't in the list shows up as a dim white.

**Note:** Black wires let the Jumperless auto-assign a color.

If you leave all the wires green (the default in Wokwi) or make a wire black, it'll just auto assign colors.

**About color assignment:** There is some weirdness because colors in Wokwi are applied to `bridges` (a pair of `nodes`) while color in the Jumperless gets assigned to `nets` (a collection of connected `nodes`). So if you have a bunch of things electrically connected together with different wire colors, it'll use the first colored wire it finds in that net and drop the rest.

### Rail Voltage Detection

Add a text label in your Wokwi diagram to specify rail voltages:

```
top rail 5.5V
bottom rail 3.5V
```

The Jumperless parser will automatically detect these values and set the rails accordingly

It's not picky about the format, `TOP RAIL = 5V`, `top_rail: 4.5V`, `toprail 3v3`, `bot rail 2.5V` and `BOTTOM_RAIL = -5V` all work, and negative voltages are fine. A label like `VCC = 5V` sets the top rail if there's no top rail label.

### Command Variants

```
W              # Paste JSON, save to active slot
W 5            # Paste JSON, save to slot 5
W /file.json   # Load from file, save to active slot
W /file.json 5 # Load from file, save to slot 5
```

### After Import

Use `<` to cycle through slots to activate your imported circuit, or `<5` to jump straight to slot 5. An import into the active slot is applied to the hardware immediately, an import into another slot just sits in that slot's file until you select it.

## Terminal Compatibility

Or you can use any terminal emulator you like, [iTerm2](https://iterm2.com/), [xTerm](https://invisible-island.net/xterm/), [Tabby](https://github.com/Eugeny/tabby), [Arduino IDE](https://www.arduino.cc/en/software/)'s Serial Monitor, whatever. The TUI is all handled from the Jumperless itself so it just needs something to print text. 