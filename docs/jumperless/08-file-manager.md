
# File Manager

The Jumperless has a built in File Manager which you can access in the menu with `/`, or enter `U` in the menu and Jumperless will mount as a USB Mass Storage drive called `JUMPERLESS` where you can edit files on the filesystem.

`/` on its own opens the File Manager. Put a filename after it, like `/adc_basics.py`, and that script runs straight away - it looks in `/python_scripts`, its `examples`, `lib` and `modules` folders, then the root.

## File System Structure

```
├── config.txt
│
├── slots/
│   ├── slot0.yaml
│   └── ... (a file shows up for each slot you've saved to, up to slot7.yaml)
│
├── projects/
│   ├── 555/
│   │   ├── README.md
│   │   ├── main.py
│   │   └── wiring.yaml
│   ├── i2cscrn/
│   ├── nand00/
│   └── eeprom/
│
├── images/                (bitmaps for the OLED)
├── screens/               (OLED GUI layouts, once something has saved one)
├── undo_history.txt             (the undo history, saved with your slots)
│
└── python_scripts/
    ├── history.txt
    ├── cool_micropython_script.py
    ├── ... (your python scripts go here)
    │
    ├── lib/
    │   ├── jumperless.py
    │   ├── jumperless.pyi
    │   └── oledgui.py
    │
    ├── modules/
    │
    └── examples/
        ├── adc_basics.py
        ├── dac_basics.py
        ├── file_io_basics.py
        ├── gpio_basics.py
        ├── interaction_demo.py
        ├── pin_irq_basics.py
        ├── led_brightness_control.py
        ├── node_connections.py
        ├── oscilloscope.py
        ├── stylophone.py
        ├── uart_basics.py
        ├── uart_loopback.py
        ├── voltage_monitor.py
        └── ... (and more)
```

Each slot's configuration is stored as a YAML file in the `/slots/` directory, and the global hardware configuration is in `/config.txt`.

---

## Navigation

| Control | Action |
|---------|--------|
| **↑/↓ Arrow Keys** or **Rotary Encoder** | Move selection up/down |
| **Enter** or **Click Encoder** | Open directory or edit file. Slot files get *loaded* instead (use `e` to edit them), `.bin` and `.bmp` open in the bitmap editor, and clicking a project's `wiring.yaml` in `/projects/` starts that project. `.py` files run if you got here from the clickwheel `Files` menu; from the terminal `/` they open in the editor. |
| **Hold Encoder** | Go up one directory (at the root it quits) |
| **Long hold Encoder** | Quit File Manager |
| **/** | Go to root directory |
| **.** | Go up one directory |
| **Esc** | Go up one directory, or quit if you're already at the root |
| **h** | Show help |
| **v** | Quick view of file contents |
| **e** | Edit file |
| **i** | File info |
| **n** | New file (prompts for filename) |
| **d** | New directory |
| **x** | Delete file or directory (confirm with `y`/`N`) |
| **r** | Refresh directory listing |
| **u** | Memory status |
| **m** | Force initialize MicroPython examples |
| **q** or **CTRL + q** | Quit File Manager (Ctrl+Q also quits the Text Editor) |

---

### File Type Icons and Colors
| Icon | File Type | Extensions | Color |
|------|-----------|------------|-------|
| **⌘** | Directories | - | Cyan |
| **𓆚** | Python files | .py, .pyw, .pyi | Green |
| **⍺** | Text files | .txt, .md, .readme | Yellow |
| **⚙** | Config files | .cfg, .conf, any name starting with `config` | Magenta |
| **⟐** | JSON files | .json, .yaml, .yml | Blue |
| **☊** | Legacy slot files | nodeFileSlot*.txt | Orange |
| **⎃** | Net color files | netColorsSlot*.txt | Pink |

Images, audio, video, documents, and archives get their own icons too. Slot files are `.yaml`, so they show up blue like JSON. Anything unrecognized shows as a grey **⍺**.


---


## Jumperless eKilo Text Editor

The File Manager also has text editor based off [**eKilo**](https://github.com/antonio-foti/ekilo)

<img width="928" alt="Screenshot 2025-07-06 at 11 25 50 AM" src="https://github.com/user-attachments/assets/1b4c74bc-19fa-4e74-8799-b778e8a56825" />


### Editor Controls
- **Ctrl+S**: Save file
- **Ctrl+Q**: Quit editor. If you have unsaved changes it asks you to press it once more (Esc doesn't discard your edits, it just tells you how to leave)
- **Ctrl+P**: Save and load the file into the MicroPython REPL - `.py` files only, anything else just saves
- **Ctrl+U**: Memory status
- **Tab**: Indent
- **Arrow keys**: Navigate cursor
- **Rotary encoder**: Move cursor horizontally. Turn past the end of the file (or back before the start) and you get a Save/Cancel menu to pick from with the wheel and a click
- **Click encoder**: Enter character selection mode (you can scroll through the letters on the OLED and click again to insert it)
- **Long hold encoder**: Save (if you've changed anything) and quit

### Character Selection With the Click Wheel and OLED
When using the rotary encoder in the editor:

- **Click encoder**: Enter character selection mode
- **Rotate encoder**: Cycle through available characters
- **Click encoder**: Confirm character selection
- **Wait 5 seconds**: Exit character selection mode

Yes, you could write code with just the click wheel and the OLED if you really wanted to.

![1760676653009](https://github.com/user-attachments/assets/31541e79-bde0-4219-9542-ee060933ed8a)

--- 

## OLED Display Support

If you have an OLED connected, the File Manager shows:
- **Current path** and **selected file**
- **File navigation** with scrolling support
- **Real-time updates** as you navigate



---

### MicroPython Examples
The example scripts live in `/python_scripts/examples/`. Open one in the File Manager and press `Ctrl+P` to run it (or just type its path at the main menu), and see the [Examples](08.5-examples.md) page for the full list and what each one does.

If you delete one it comes back on its own the next time the File Manager opens. `m` forces a refresh: it also updates examples that still match an older firmware's version and always refreshes the `lib/` modules, and if you've edited an example it leaves yours alone and writes the current default next to it as `<name>_original.py`.


## Editing Slot Files

Slot files (located in `/slots/`) use **YAML format** and can be edited directly! They're human-readable files containing:

- **bridges** - Your circuit connections, one `{n1: A, n2: B}` pair per line (`dup:` and `color:` show up only where they're set; net colors and names live under `nets`)
- **power** - Rail and DAC voltages (topRail, bottomRail, dac0, dac1)
- **config** - Routing preferences and GPIO settings
- **nets**, **parts** and **overlays** - written by the board itself

**Example slot file:**
```yaml
version: 2
sourceOfTruth: bridges

bridges:
  - {n1: 1, n2: 10, dup: 2, color: red}
  - {n1: NANO_D5, n2: GP_1, dup: 2}
  - {n1: TOP_RAIL, n2: 5, dup: 2}

power:
  topRail: 3.30
  bottomRail: 2.50
  dac0: 3.33
  dac1: 0.00
```

**Named nodes** you can use: `NANO_D0-D13`, `NANO_A0-A7`, `GP_1-8` (or `RP_GPIO_1-8`), `TOP_RAIL`, `BOTTOM_RAIL`, `GND`, `DAC0`, `DAC1`, and more (see [glossary](99-glossary.md)). Note these differ from the MicroPython constants - `GPIO_1` won't parse in slot files.

If you edit the active slot's file, the Jumperless automatically reloads it - when you quit the onboard eKilo editor, or, if you have the Jumperless mounted as a USB Mass Storage drive and are editing the files on your computer, when you eject/unmount the drive.

Opening any `/slots/slotN.yaml` in the onboard editor previews that slot on the breadboard while you're editing it. When you close the editor the board goes back to whichever slot was active and re-reads its file, which is how your saved edits to the active slot land.

---

## USB Mass Storage

Enter `U` in the menu and Jumperless will mount as a USB Mass Storage drive called `JUMPERLESS` where you can edit files on the filesystem.

Keep in mind that file operations are pretty slow, so make sure to give it time to fully save files when you drop them onto the filesystem.

Ejecting the drive from your computer applies your edits - the board syncs, saves anything pending, and reloads the active slot - but it's still a drive you can remount. Enter `u` to actually turn Mass Storage off.

You can also enter `Z` for a little debug menu: `1` toggles USB debug output, `2` forces a refresh of the filesystem from what your computer wrote, and `3` validates every slot file. While the drive is on, `u` disables it, `G` reloads config.txt, `y` refreshes connections, and `S` shows status.

<img width="757" height="380" alt="Screenshot 2025-07-15 at 8 55 44 AM" src="https://github.com/user-attachments/assets/124d2f5a-a320-453f-8598-7604f37a57d7" />
  
   
<img width="179" height="189" alt="Screenshot 2025-07-15 at 8 55 54 AM" src="https://github.com/user-attachments/assets/4531cae9-56d9-42da-9279-952f7b23d405" />
 
  

<img width="1014" height="463" alt="Screenshot 2025-07-15 at 8 56 13 AM" src="https://github.com/user-attachments/assets/a9e79a69-a7da-4365-a457-44b2c5d2fc24" />
