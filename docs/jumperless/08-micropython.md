# MicroPython

This guide covers how to write, load, and run Python scripts that control Jumperless hardware using the embedded MicroPython interpreter.

<!-- ## Table of Contents

1. [Quick Start](#quick-start)
2. [Hardware Control Functions](#hardware-control-functions)
3. [Writing Python Scripts](#writing-python-scripts)
4. [Loading and Running Scripts](#loading-and-running-scripts)
5. [REPL (Interactive Mode)](#repl-interactive-mode)
6. [File Management](#file-management)
7. [Examples and Demos](#examples-and-demos)
8. [Troubleshooting](#troubleshooting) -->

If you just want an overview of all the available calls, check out the [**MicroPython API Reference**](09.5-micropythonAPIreference.md)

For stuff that's not Jumperless-specific, check out the [MicroPython Docs](https://docs.micropython.org/en/latest/index.html)

## Now you can live code with [JumperIDE](https://ide.jumperless.org/)!
Holy shit I should have done this years ago

Seriously, this is *such* a better experience than using the onboard text editor and REPL, you should play with it right now

Go to [https://ide.jumperless.org/](https://ide.jumperless.org/) and press the connect button.

![JumperIDE before connecting](assets/verify/jumperide-landing.png)

Choose the 3rd Jumperless port in that list (Windows may not put them in order, so if nothing happens, try the other ones) and click Connect

<img width="1304" height="1250" alt="Screenshot 2025-12-08 at 6 13 46 PM" src="https://github.com/user-attachments/assets/a9ea53fa-86fd-46b0-839d-eeaa32606454" />

Then open some examples (this update should overwrite the examples with the new ones) and hit the Run / Stop button

![gpio_basics.py running in JumperIDE](assets/verify/jumperide-run.png)

Press it again to Stop. If you make changes, hit the green Save button next to it (it takes a second and the script should be stopped.)

![an edited file waiting to be saved](assets/verify/jumperide-save.png)

**Serial terminal.** The `Serial Terminal` tab is port 1, the board's main menu, so you get the REPL and the menu side by side. Click `Connect` in that tab, choose the 1st Jumperless port this time, and type `m`.

![the Serial Terminal tab showing the main menu](assets/verify/jumperide-terminal.png)

**API reference.** The book icon opens the MicroPython API docs on the right, and if you have `Go To Clicked Function` checked, the docs should jump to whatever function you click in your code.

![the API reference following a click on adc_get](assets/verify/jumperide-api-ref.png)

**JumperNet Registry.** The Jumperless icon in the sidebar is the JumperNet Registry, scripts people have shared from their own boards. It's one list, everybody sees the same one, and there's no account. Click a script and it opens in a tab, `Run` runs it on your board.

![the JumperNet Registry list](assets/verify/jumperide-registry.png)

![a registry script open in the editor](assets/verify/jumperide-registry-script.png)

To share yours, have it open in a tab and click `Upload script to registry` at the top of the list. It asks for a name, your name, and a line about what it does (the code box is already filled in from the tab), and `Upload` puts it in the list for everyone right away, there's no review step. Work in progress scripts are fine, that's what it's for.

![the upload dialog](assets/verify/jumperide-registry-upload.png)

The three little icons on a row are open, edit and history. Anyone can edit anyone's script (it's wiki style, so fix the typos you find, and replace the prefilled `Your name` with yours or the edit gets filed under the last author), every version stays in the history, and `Load` opens an old one in its own tab without touching the current one.

To delete a script, edit it and set the name to `delete`. Nothing checks who you are, so only delete your own. OLED images work the same way for uploading and deleting (`Upload OLED image` shows up when a `.bin` is the open tab, `Browse Images` is the gallery), except only the original author can edit one, and an image's history is the last 50 versions. Every script uploaded gets synced into the [JumperIDE repo](https://github.com/Architeuthis-Flux/JumperIDE/tree/main/cloudflare/script-registry-worker/scripts) on its own, so the scripts are all on GitHub too, images live only in the registry.

![a script's history](assets/verify/jumperide-registry-history.png)

**OLED bitmaps.** `Tools` > `New OLED bitmap` gives you a 128x32 canvas, click or drag to draw, and with `Live to device` checked it should show up on the board's OLED as you draw ([more about the OLED here](04-oled.md)). `Download .bin` saves it and `Upload to registry` shares it. `Browse Images` in the registry shows everyone else's.

![drawing in the OLED bitmap editor](assets/verify/jumperide-oled-editor.png)

![the same drawing on the board's OLED](assets/verify/jumperide-oled-live.png)

![the shared OLED images in the registry](assets/verify/jumperide-registry-images.png)

### If you write something cool, publish it to the JumperNet registry from JumperIDE for VS Code (**Jumperless: Publish Script to Registry**) and I'll add the good ones to the default examples.


This is using [MicroPython's built-in Raw REPL](https://docs.micropython.org/en/latest/reference/repl.html#raw-mode-and-raw-paste-mode), so anything that can interact with that will work here. I've tested it with JumperIDE, both the web one and the VS Code extension, but anything that speaks the raw REPL (like `mpremote`) works on that 3rd port.


There's also `jumperless.py` and `jumperless.pyi` module with stubs for all the built-in functions so syntax highlighting and autocomplete  will work in your favorite code editor (sorry, autocomplete for jumperless functions doesn't work in the web IDE. JumperIDE for VS Code installs these stubs for you.) You can grab them here:

### [jumperless_module.py](https://github.com/Architeuthis-Flux/JumperlOS/blob/main/scripts/jumperless_module.py)
### [jumperless.pyi](https://github.com/Architeuthis-Flux/JumperlOS/blob/main/scripts/jumperless.pyi)

They're also already on the board as `/python_scripts/lib/jumperless.py` and `/python_scripts/lib/jumperless.pyi`, so you can just copy them off it.

### Stubs by hand

JumperIDE for VS Code sets this up for you (globally, or per folder with `Set Up This Folder for Jumperless Python`), but if you just want autocomplete in a project of your own it's 3 files, in VS Code or Cursor or whatever else runs Pylance. The `.pyi` is the one the editor reads. The `.py` is the same API as plain Python, for running on the Jumperless, no types in it.

Make a `typings` folder in your project and put the `jumperless.pyi` from above in it. Pylance looks there for stubs by default, `from jumperless import *` should resolve, every function gets its real signature and docs.

Next to it, make `typings/__builtins__.pyi` with one line in it:

```python
from jumperless import *
```

That tells Pylance the whole API is a builtin, which is what it is on the Jumperless, `from jumperless import *` happens before your script runs so you don't need to write it, in the editor or on the Jumperless.

Then a `pyrightconfig.json` in the project root:

```json
{
    "stubPath": "typings",
    "reportMissingModuleSource": false
}
```

The first line makes this folder override the global stubs my VS Code extension sets up (those are a few functions behind the one on main), the second one turns off the yellow squiggle you'd get on `from jumperless import *` because there's a stub but no `.py` behind it, it's harmless, just annoying.

(note: `pip install jumperless` / `uv add jumperless` gets you the Jumperless Wokwi Bridge. Its module is `jumperless_pkg` and it has nothing to do with the board's API, the stubs aren't on PyPI.)

### Jupyter notebooks

[micropython-magic](https://github.com/Josverl/micropython-magic) runs a notebook cell on the Jumperless while the rest of the notebook runs on your computer, and it works with the stubs above.

`pip install micropython-magic` (or `uv add`), then in the first cell:

```python
%load_ext micropython_magic
%mpy --select /dev/cu.usbmodemJLV5port5
```

The port is the 3rd Jumperless one, the same one JumperIDE uses, so disconnect JumperIDE first, mpremote can't open a port something else is holding. On a Mac it's `/dev/cu.usbmodemJLV5port5`, on Linux it's `/dev/ttyACM` and a number, on Windows it's a COM number and they're not always in order, it's the one where Enter gets you a `>>>`. `--verify` didn't work for me, plain `--select` does.

Then start a cell with `# %%micropython` and it runs on the Jumperless:

```python
# %%micropython
for i in range(5):
    print(adc_get(0))
print(gpio_get(1))
oled_print("hi from a notebook")
```

![a notebook cell running on the Jumperless](assets/verify/vscode-notebook-run.png)

The `#` is on purpose, Pylance skips any cell that starts with a `%%` magic it doesn't know, so with a bare `%%micropython` you get no squiggles or hover or anything. micropython-magic turns `# %%micropython` back into the real magic when the cell runs, the editor sees plain Python, hovering `adc_get` gives you the signature and docs from the stub.

![the stub's signature in a cell that runs on the Jumperless](assets/verify/vscode-notebook-hover.png)

Run the `%load_ext` cell first. Until it's loaded, `# %%micropython` is just a comment and the cell runs on your computer, where `adc_get` isn't a thing (you get `NameError: name 'adc_get' is not defined`, that's what that means, the Jumperless never saw it).

---

## JumperIDE for VS Code

![](assets/DirtyDeedsJumperlesssm.png)

[![VSCode Marketplace](https://img.shields.io/badge/VSCode%20Marketplace-JumperIDE-blue?logo=visual-studio-code)](https://marketplace.visualstudio.com/items?itemName=ArchiteuthisFlux.jumperide)
[![Open VSX](https://img.shields.io/open-vsx/v/ArchiteuthisFlux/jumperide?label=Open%20VSX)](https://open-vsx.org/extension/ArchiteuthisFlux/jumperide)
[![GitHub Release](https://img.shields.io/github/v/release/Architeuthis-Flux/JumperIDE-VSCode?label=Release)](https://github.com/Architeuthis-Flux/JumperIDE-VSCode/releases/latest)
[![License: Unlicense](https://img.shields.io/badge/license-Unlicense-blue)](https://github.com/Architeuthis-Flux/JumperIDE-VSCode/blob/main/LICENSE)

Same idea as the web IDE, but it runs in VS Code (or Cursor, or VSCodium), so you get Pylance, git, and everything else you already have. Everything is in one sidebar, and the board shows up as a terminal.

![](assets/JumperIDEwalkthrough.gif)

#### Install

Search `JumperIDE` in the Extensions view, it's on the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=ArchiteuthisFlux.jumperide) and [Open VSX](https://open-vsx.org/extension/ArchiteuthisFlux/jumperide) (that's the one Cursor and VSCodium search). Or download the `.vsix` from the [latest release](https://github.com/Architeuthis-Flux/JumperIDE-VSCode/releases/latest) and run `code --install-extension jumperide-1.0.2.vsix` from the folder it's in.

The first time it loads it sets up autocomplete for the whole Jumperless API in every workspace (the stubs go in `~/.jumperless/typings`) and asks you to reload the window. The hover docs need Python and Pylance, `Set Up This Folder for Jumperless` offers to install them if you don't have them.

#### Connect

Click the Jumperless icon in the activity bar. The `Actions` view is the whole thing: a connection button that shows the state, `Run` and `Stop`, saving, the OLED editor, the terminal, the API reference, and publishing.

![the Actions view](assets/verify/vscode-actions.png)

Click `Disconnected` (or `Jumperless` in the status bar, or `Jumperless: Connect` from the command palette) and pick the starred port, the one whose description ends in `MicroPython REPL — recommended`. On a Mac it reads `port5`. It's the 3rd Jumperless port and it isn't pre-selected, so don't just hit `Enter`, that gets you `port1`, the menu, which can't run scripts, and you end up in terminal-only mode with an empty file tree.

![the port picker](assets/verify/vscode-connect-picker.png)

The status bar should turn pink with the board's name, `Device Files` fills in, and a `Jumperless REPL` terminal opens at the `>>>` prompt.

![connected](assets/verify/vscode-connected.png)

#### Files, run, stop

Click a file in `Device Files` and it opens as a local working copy, so Pylance sees it and hovering a Jumperless function gives you its docs. `Cmd+S` / `Ctrl+S` pushes it back to the board. The icons in the view's header are disconnect, refresh, new file, new folder and a new OLED bitmap, `Delete from Device` is on the right-click menu.

![a device file open, with the hover docs](assets/verify/vscode-hover.png)

`F5` (or `Run Current File`) runs whatever is in the editor, as is, it doesn't save first. The output goes to the `Jumperless REPL` terminal. `Shift+F5` (or `Stop`) sends a `Ctrl+C`.

![running gpio_basics.py](assets/verify/vscode-run.png)

![Connect Run Stop](assets/ConnectRunStop.gif)

`Save to Jumperless` pushes the current file to the board (a file that didn't come from the board asks for a device path, prefilled with last time's answer), `Save Locally` saves a copy on your computer.

#### Serial terminal

`Serial Terminal` (or `Jumperless: Open Serial Terminal`) opens a second terminal on any port. `port1` is the starred one, that's the board's menu, and it types `m` for you so the menu comes up.

`Ctrl+Q` goes through to the board in these terminals instead of to VS Code. Or pick `Use Jumperless App` and it runs the [standalone app](https://github.com/Architeuthis-Flux/Jumperless-App) in a terminal instead.

![the serial terminal port picker](assets/verify/vscode-serial-picker.png)

![the board's menu in the serial terminal](assets/verify/vscode-serial-terminal.png)

#### OLED bitmaps

`New OLED Bitmap` asks whether the file lives on the Jumperless or on this computer (if you're not connected it just saves locally), then opens a 128x32 pixel editor, and any `.bin` in `Device Files` opens in it too. `Black`, `White` and `Toggle` are the pens, and with `Live to device` checked every stroke should show up on the board's OLED as you draw (it `Ctrl+C`s a running script to do it). `Save` writes the `.bin`, `Push to Device` sends the whole thing once.

![the OLED bitmap editor](assets/verify/vscode-oled-editor.png)

![the same drawing on the board's OLED](assets/verify/vscode-oled-live.png)

#### API reference

`API Reference` opens the [MicroPython API docs](09.5-micropythonAPIreference.md) beside your code. The hover docs and autocomplete come from the same place.

![the API reference beside the code](assets/verify/vscode-api-ref.png)

#### JumperNet

`JumperNet Registry` and `OLED Images` in the sidebar are the shared registry, the same one the web IDE shows. Click a script and it opens, right-click for `Save to Device`.

`Publish to JumperNet` (or `Jumperless: Publish Script to Registry`) uploads the file in the editor, it asks for a name, a description and your name. Editing and history are in the web IDE only.

![a registry script opened from the sidebar](assets/verify/vscode-registry.png)

![publishing a script](assets/verify/vscode-publish.png)

#### Settings

`jumperless.serial.preferredPortIndex` is which Jumperless port gets the star (2, the 3rd one), `jumperless.serial.baud` is 115200, `jumperless.registry.baseUrl` is the registry, and `jumperless.connectOnStartup` opens the port picker when VS Code starts.

#### Zero-import autocomplete

On the board, scripts run with the full API preloaded (`from jumperless import *` happens before your code). The editor matches that automatically: on first activation the extension installs typed stubs and points your Python analyzer at them, so files opened from the device resolve the whole API with no imports and no setup. (Controlled by `jumperless.setup.autoSetUpGlobally`, on by default.)

- `typings/jumperless.pyi` — typed stub, synced from [JumperlOS](https://github.com/Architeuthis-Flux/JumperlOS) (by hand, so it can be a few functions behind `scripts/jumperless.pyi` on main)
- `typings/builtins.pyi` — standard-library builtins with Jumperless globals layered on top (typo detection still works)
- `typings/time.pyi` — MicroPython `time` extras (`ticks_ms`, `sleep_ms`, …)

To get the same thing in one of your own project folders (checked into that repo instead of user settings), run **Jumperless: Set Up This Folder for Jumperless Python** — it writes the `typings/` folder and a `pyrightconfig.json` into the workspace.

---

## Quick Start (Built-in REPL)

<!-- ### Starting MicroPython REPL -->
From the main Jumperless menu, press `p` to enter the MicroPython REPL:

![Screenshot 2025-07-04 at 7 03 24 PM](https://github.com/user-attachments/assets/e7ce0688-5ddf-48da-8560-4a8f6b747c4f)


## REPL Navigation

Up / Down arrow keys on a blank prompt will scroll through history, any other key will break out of history mode and enter multiline editing. So you can use arrow keys to navigate and edit the script. 

In history mode, the `>>>` prompts will be pink, when you're editing, they'll be blue.


## Hardware Control Functions

All Jumperless hardware functions are automatically imported into the global namespace - no prefix is actually necessary. JumperIDE for VS Code already resolves them with its stubs, but in an editor without those it's probably good to use `import jumperless as j` so it doesn't complain about undefined names.

---



## Basic Script Structure
```jython
"""
My Jumperless Script
Description of what this script does
"""

print("Starting my script...")

# Connect some nodes
connect(1, 5)
connect(2, 6)

# Set up GPIO
gpio_set_dir(1, True)  # Output
gpio_set_dir(2, False) # Input

# Main loop
for i in range(10):
    gpio_set(1, True)
    time.sleep(0.5)
    gpio_set(1, False)
    time.sleep(0.5)
    
    # Read input (gpio_get returns truthy for HIGH, falsy for LOW)
    if gpio_get(2):
        print("Button pressed!")

# Cleanup
nodes_clear()
print("Script complete!")

```
   



## Loading and Running Scripts


### Method 1 (Recommended): [JumperIDE](https://ide.jumperless.org/)
See [above](#now-you-can-live-code-with-jumperide) for instructions. It's at the top of the page for a reason, it's awesome.

### Method 2: File Manager
From the REPL (enter `p` in the main menu), then type `files` to open the file manager:

```jython
>>> files
```

Navigate to your script and press Enter to load it for editing, then press `Ctrl+P` to load it into the REPL for execution.

**Note:** The standard Python `exec(open(...).read())` also works - the filesystem is mounted as a MicroPython VFS, so the built-in `open()` and the `os` module operate on the same files as `jfs`.

### Method 3: REPL Commands
From the MicroPython REPL, you can use the following commands to manage scripts:

```jython
# Put a saved script on the REPL input line to edit or run
# (bare `load` lists the saved scripts with numbers)
load my_script.py

# Save the last executed script (auto-named if you leave the name off)
save my_new_script.py
```

### Method 4: Direct Execution
From the main Jumperless menu, you can execute single commands. The `>` is the command, type it with the line, and use `print()` to see a value:

```jython
> gpio_set(1, True)
> print(adc_get(0))
> connect(1, 5)
```

## REPL (Interactive Mode)

### Starting REPL
From main menu: Press `p`

### REPL Commands
```jython
CTRL + q           - Exit REPL
quit               - Exit REPL
history            - Show command history and saved scripts
save [name]        - Save last executed script
load <name>        - Load script by name or number
files              - Open file manager
new                - Create new script with eKilo editor
edit               - Open the last input in the eKilo editor
multiline on|off|auto - Force multiline mode on or off, or go back to automatic (not in `helpl`, but it works)
context            - Toggle connection context
helpl              - Show REPL help
help()             - Show hardware commands
```

### Navigation
```
↑/↓ arrows         - Browse command history
←/→ arrows         - Move cursor, edit text
TAB                - Add 4-space indentation
Enter              - Execute (empty line in multiline to finish)
Ctrl+Q             - Force quit REPL or interrupt running script
```

### Multiline Auto-Indent Mode
The REPL automatically detects when you need multiple lines after a `:`, and it indents for you, so type the lines without the leading spaces.

```jython
>>> def blink_led():
...     for i in range(5):
...         gpio_set(1, True)
...         time.sleep(0.5)
...         gpio_set(1, False)
...         time.sleep(0.5)
... 
>>> blink_led()
```

If you want *real* multiline mode, type `multiline on` and then `run` to execute what you typed. `new` and `edit` open the eKilo editor straight from the REPL. 

### Command History
- Use ↑/↓ arrows to browse previous commands
- Commands are automatically saved
- Type `history` to see all saved scripts


## Connection Context Switching

The MicroPython REPL now supports **connection contexts** that determine how connections persist:

- **`global` context**: Changes persist to global state - connections remain after exiting Python
- **`python` context**: Connections you make in the session are undone when you exit, and the state from when you entered comes back

**To toggle contexts:** Type `context` in the REPL

**How it works:**
- In `global` mode: Any connections you make become permanent, just like using the normal command interface
- In `python` mode: The connection state when you entered the REPL is saved, and restored when you exit
- The current context is shown in the REPL banner when you enter, and by `helpl`. Typing `context` prints it too. The default is `global`


## Built-in Examples
The board comes with a pile of example scripts in `/python_scripts/examples/`. Every one of them is described on the [Examples](08.5-examples.md) page.

There are three ways to run one. From the REPL, type `files`, open the script, and press `Ctrl+P`. From the main menu, `/` opens the file manager and `/python_scripts/examples/adc_basics.py` runs a script directly. From the clickwheel, `Files` lists the filesystem and clicking a `.py` file runs it.


## Troubleshooting

**REPL not responding:**
- Press Ctrl+Q to force quit on the main menu port (port 1), or Ctrl+C on the 3rd port that JumperIDE uses
- Unplug / replug your Jumperless (don't worry, almost everything is persistent)

**`@micropython.native` / `@micropython.viper` reboots the board:**
- On 5.7.11.0 defining one is fine, calling it hard faults (the function pointer is missing its Thumb bit, so the first call is a UsageFault). After it comes back, press `m` on the main menu port and it prints a `[crashlog]` line saying so.



## Formatted Output and Custom Types
The Jumperless module returns custom types that print nicely but also work in conditionals:

```jython
# GPIO functions return custom types that print as readable strings
state = gpio_get(1)           # Prints "HIGH", "LOW", or "FLOATING"
direction = gpio_get_dir(1)   # Prints "INPUT" or "OUTPUT"
pull = gpio_get_pull(1)       # Prints "PULLUP", "PULLDOWN", or "NONE"

# These types are also truthy/falsy for use in conditionals:
if gpio_get(1):               # True if HIGH, False if LOW or FLOATING
    print("Pin is HIGH")
if gpio_get_dir(1):           # True if OUTPUT, False if INPUT
    print("Pin is output")

# Connection status works the same way
connected = is_connected(1, 5) # Prints "CONNECTED" or "DISCONNECTED"
if connected:                  # True if connected, False if not
    print("Nodes are connected")

# Voltage and current readings are floats
voltage = adc_get(0)          # Returns float (e.g., 3.300)
current = ina_get_current(0)  # Returns float in A (e.g., 0.0123)
power = ina_get_power(0)      # Returns float in W (e.g., 0.4567)

# All functions work with both numbers and string aliases
gpio_set_dir("GPIO_1", True)  # Same as gpio_set_dir(1, True)
connect("TOP_RAIL", "GPIO_1") # Same as connect(101, 131)
```


