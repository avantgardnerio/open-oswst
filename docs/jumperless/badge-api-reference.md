# Temporal Badge MicroPython API Reference

All functions and constants from the `badge` module are automatically imported
into the global namespace. You can call them directly (e.g. `button(BTN_CONFIRM)`)
without needing the `badge.` prefix.

---

[Init](#init):

* `init()` — Initialize the badge hardware bridge (called automatically)

[OLED Display](#oled-display):

* `oled_print(text)` — Print text at cursor
* `oled_println(text)` — Print text + newline + show
* `oled_clear([show])` — Clear display
* `oled_show()` — Refresh display
* `oled_set_cursor(x, y)` — Set text cursor
* `oled_set_text_size(size)` — Set text size (1–4)
* `oled_get_text_size()` — Get text size
* `oled_invert(enable)` — Invert display colors
* `oled_text_width(text)` — Get pixel width of a string
* `oled_text_height()` — Get pixel height of current font
* `oled_set_font(name)` — Set font by name
* `oled_get_fonts()` — Get available font names
* `oled_get_current_font()` — Get current font name
* `oled_set_pixel(x, y, color)` — Set single pixel
* `oled_get_pixel(x, y)` — Read single pixel
* `oled_draw_box(x, y, w, h)` — Draw filled rectangle
* `oled_set_draw_color(color)` — Set draw color (0=black, 1=white, 2=XOR)
* `oled_get_framebuffer()` — Get framebuffer as bytes
* `oled_set_framebuffer(data)` — Set framebuffer from bytes
* `oled_get_framebuffer_size()` — Get (width, height, bytes)
* `oled_screenshot([mode])` — Dump OLED framebuffer to terminal as block characters

[Native UI Chrome](#native-ui-chrome):

* `ui_header(title, [right])` — Draw the standard header and rule
* `ui_action_bar([left_button], [left_label], [right_button], [right_label])` — Draw footer actions with native button glyphs
* `ui_chrome(title, [right], [left_button], [left_label], [right_button], [right_label])` — Clear and draw standard header/footer chrome
* `ui_inline_hint(x, y, hint)` — Draw an inline hint with native button glyphs
* `ui_inline_hint_right(right_x, y, hint)` — Right-align an inline hint
* `ui_measure_hint(hint)` — Return an inline hint's pixel width

[Mouse Overlay](#mouse-overlay):

* `mouse_overlay(enable)` — Enable/disable cursor overlay
* `mouse_set_bitmap(data, w, h)` — Set cursor sprite (row-major bitmap, max 32×32)
* `mouse_x()` — Current cursor X position
* `mouse_y()` — Current cursor Y position
* `mouse_set_pos(x, y)` — Warp cursor to position
* `mouse_clicked()` — Read last click button ID (-1 if none)
* `mouse_set_speed(speed)` — Set cursor speed (1–20, default 3)
* `mouse_set_mode(mode)` — Set positioning mode (`MOUSE_ABSOLUTE` or `MOUSE_RELATIVE`, default `MOUSE_RELATIVE`)

[Buttons & Joystick](#buttons-joystick):

* `button(id)` — Read button state (True if held)
* `button_pressed(id)` — Edge-triggered press (True once per press)
* `button_held_ms(id)` — Milliseconds button has been held
* `joy_x()` — Joystick X axis (0–4095)
* `joy_y()` — Joystick Y axis (0–4095)

[LED Matrix](#led-matrix-88) (8×8):

* `led_brightness(value)` — Set global brightness (0–255)
* `led_clear()` — Turn off all LEDs
* `led_fill([brightness])` — Turn on all LEDs
* `led_set_pixel(x, y, brightness)` — Set single LED
* `led_get_pixel(x, y)` — Read single LED brightness
* `led_show_image(name)` — Show builtin image by name
* `led_set_frame(rows, [brightness])` — Draw 8×8 bitmask pattern
* `led_start_animation(name, [interval_ms])` — Start builtin animation
* `led_stop_animation()` — Stop current animation
* `led_override_begin()` — Pause ambient LED mode for foreground drawing
* `led_override_end()` — Release foreground drawing and restore ambient LEDs
* `led_screenshot([mode], [ansi])` — Dump LED matrix to terminal as block characters
* `screenshot([mode], [ansi])` — Dump OLED and LED matrix together to terminal;
  REPL hotkey: bare `o` + Enter

[Matrix App Host](#matrix-app-host) (Background LED Callbacks):

* `matrix_app_start(callback, [interval_ms], [brightness])` — Register a Python callback for background LED matrix rendering
* `matrix_app_set_speed(interval_ms)` — Change the tick interval
* `matrix_app_set_brightness(brightness)` — Change the LED brightness
* `matrix_app_stop()` — Unregister the callback
* `matrix_app_active()` — Check if a callback is registered
* `matrix_app_info()` — Return diagnostic tuple

[App Manifest](#app-manifest) (Folder-app metadata):

* `__title__`, `__description__`, `__icon__`, `__matrix_title__`, `__order__` — Top-level dunders read from `/apps/<slug>/main.py` to decorate the main menu tile
* `icon.py` — 12×12 packed-XBM `DATA` tuple for the home-screen icon
* `matrix.py` — Persistent LED-matrix script registered via `matrix_app_start`
* `rescan_apps()` — Hot-refresh the registry after editing manifests

[IMU](#imu):

* `imu_ready()` — Check if IMU is initialized
* `imu_tilt_x()` — X-axis tilt in milli-g
* `imu_tilt_y()` — Y-axis tilt in milli-g
* `imu_accel_z()` — Z-axis acceleration in milli-g
* `imu_face_down()` — True if badge is face-down
* `imu_motion()` — Consume motion event (True if motion detected since last call)

[Haptics](#haptics) (Vibration Motor + Coil Tone):

* `haptic_pulse([strength], [duration_ms], [freq_hz])` — Fire vibration pulse
* `haptic_strength([value])` — Get or set motor strength (0–255)
* `haptic_off()` — Stop motor
* `tone(freq_hz, [duration_ms], [duty])` — Play audible tone from motor coil
* `no_tone()` — Stop tone
* `tone_playing()` — Check if tone is active

[IR Send/Receive](#ir-sendreceive) (NEC Protocol):

* `ir_send(addr, cmd)` — Transmit one 1-byte-addr / 1-byte-cmd NEC frame
* `ir_start()` — Start IR receive mode
* `ir_stop()` — Stop IR receive, flush queue
* `ir_available()` — Check if a received frame is waiting
* `ir_read()` — Read received (addr, cmd) as a tuple
* `ir_send_words(words)` — Transmit a multi-word NEC frame (1–64 × 32-bit)
* `ir_read_words()` — Read a received multi-word NEC frame as a tuple
* `ir_flush()` — Drop every pending RX frame
* `ir_tx_power([percent])` — Get/set IR carrier duty (1–50%)

[Badge Identity & Boops](#badge-identity-boops):

* `my_uuid()` — Return this badge's 12-char hex UID
* `boops()` — Return `/boops.json` contents as a string

[Script Control](#script-control):

* `exit()` — Raise `SystemExit` to cleanly stop the running app
* `dev(*args)` — Test harness dispatcher *(dev builds only)*

[Filesystem Access](#filesystem-access):

* Standard `os` module — `listdir`, `mkdir`, `remove`, `rename`, `stat`, etc.
* Standard `open()` / `read()` / `write()` / `close()`

---

## Init

### `init()`

Initialize the badge hardware bridge. This is called automatically when the
badge module loads — you should not normally need to call it. Returns `0` on
success; raises `OSError` on failure.

---

## OLED Display

128×64 monochrome SSD1306 OLED display controlled via U8G2.

### `oled_print(text)`

Print text at the current cursor position. Does not refresh the display
automatically — call `oled_show()` to make it visible.

* `text`: String to display.

### `oled_println(text)`

Print text followed by a newline, then automatically refresh the display.

* `text`: String to display.

**Example:**

```jython
oled_clear()
oled_println("Hello, badge!")
oled_println("Line 2")
```

### `oled_clear([show])`

Clear the display and reset cursor to (0, 0).

* `show` (optional): If `True`, refresh display immediately. Default `False`.

**Example:**

```jython
oled_clear()          # Clear buffer only
oled_clear(True)      # Clear and refresh immediately
```

### `oled_show()`

Refresh the display to show buffered changes. Required after `oled_print()`,
`oled_set_pixel()`, or `oled_set_framebuffer()` to make changes visible.

### `oled_set_cursor(x, y)`

Move the text cursor to pixel coordinates.

* `x`: X position (0–127).
* `y`: Y position (0–63).

### `oled_set_text_size(size)`

Set the text rendering size.

* `size`: Text size multiplier (1–4). Returns `True` on success.

### `oled_get_text_size()`

Returns the current text size (1–4).

### `oled_invert(enable)`

Invert the display colors.

* `enable`: `True` to invert, `False` for normal.

### `oled_text_width(text)`

Get the pixel width of a string in the current font.

* `text`: The string to measure.
* Returns the width in pixels.

**Example:**

```jython
w = oled_text_width("Hello")
x = (128 - w) // 2
oled_set_cursor(x, 30)
oled_print("Hello")
oled_show()
```

### `oled_text_height()`

Get the maximum character height of the current font.

* Returns the height in pixels.

### `oled_set_font(name)`

Set the font by name.

* `name`: Font family name (case-sensitive).
* Returns `True` if font was found, `False` otherwise.

### `oled_get_fonts()`

Returns a comma-separated string of available font names.

**Example:**

```jython
fonts = oled_get_fonts().split(",")
for f in fonts:
    oled_set_font(f)
    oled_clear()
    oled_println(f)
    import time; time.sleep(1)
```

### `oled_get_current_font()`

Returns the name of the currently active font.

### `oled_set_pixel(x, y, color)`

Set a single pixel in the framebuffer.

* `x`: X coordinate (0–127).
* `y`: Y coordinate (0–63).
* `color`: `1` for white/on, `0` for black/off.

Call `oled_show()` after setting pixels to make changes visible.

### `oled_get_pixel(x, y)`

Read the color of a single pixel.

* Returns `1` (white/on) or `0` (black/off).

### `oled_draw_box(x, y, w, h)`

Draw a filled rectangle in the framebuffer using the current draw color.

* `x`: Left edge (0–127).
* `y`: Top edge (0–63).
* `w`: Width in pixels.
* `h`: Height in pixels.

Call `oled_show()` after drawing to make changes visible.

**Example:**

```jython
oled_clear()
oled_draw_box(10, 10, 50, 20)
oled_set_draw_color(0)
oled_set_cursor(12, 12)
oled_print("Hello")
oled_set_draw_color(1)
oled_show()
```

### `oled_set_draw_color(color)`

Set the drawing color for subsequent draw operations.

* `color`: `0` = black/off, `1` = white/on (default), `2` = XOR (inverts
  existing pixels).

### `oled_get_framebuffer()`

Returns the entire display framebuffer as a `bytes` object. Format: 1 bit per
pixel, organized in vertical bytes (Adafruit SSD1306 format).

### `oled_set_framebuffer(data)`

Replace the entire display framebuffer and refresh.

* `data`: `bytes` or `bytearray` of the correct size (typically 1024 bytes for
  128×64).
* Returns `True` on success, `False` on size mismatch.

### `oled_get_framebuffer_size()`

Returns a tuple `(width, height, buffer_size_bytes)`.

**Example:**

```jython
w, h, size = oled_get_framebuffer_size()
print(str(w) + "x" + str(h) + ", " + str(size) + " bytes")
```

### `oled_screenshot([mode])`

Dump the current OLED framebuffer to the serial console as a bordered
block-character image. Useful for inspecting display state without a camera,
and for capturing UI reference while writing docs (see also `screenshot()`
and the REPL `o` hotkey below).

* `mode` (optional): Rendering size on the following ladder (smallest first):
  * `0.5` (float) — **quarter-block**, 2×2 pixels per glyph. OLED 64×32 chars.
    Smallest output.
  * `0` (int) — **half-block**, 1×2 pixels per glyph (`▀`/`▄`/`█`/space).
    OLED 128×32 chars. 1W × 1W square cells on 2:1 monospace fonts.
  * `N` (int, ≥1) — **square** at `2N` chars wide × `N` char-rows tall per pixel
    (2NW × 2NW square cells). `1` is the smallest square (2×1 chars), `2` is
    twice as big (4×2 chars), etc.
  * `N.5` (float, ≥1.5) — **tall** escape hatch: each pixel = `round(2(N−1))`
    chars wide × 1 char-row tall. `1.5` gives 1W × 2W per pixel (literal
    1-char-per-pixel), `2.5` gives 3W × 2W, etc. Use when you want
    char-per-pixel output without aspect correction.

  Default: `1` (smallest square: 2 chars × 1 row per pixel).

**Example:**

```jython
oled_clear()
oled_println("Hello!")
oled_screenshot()      # int 1 — smallest square, 256 chars wide
oled_screenshot(0.5)   # float — quarter-block compact, 64 chars wide
oled_screenshot(0)     # int 0 — half-block compact, 128 chars wide
oled_screenshot(2)     # int 2 — bigger square, 512 chars wide
oled_screenshot(1.5)   # float — literal 1 char/pixel, 128 chars wide tall
```

---

## Native UI Chrome

These helpers draw the same header, footer, and button glyph style used by
the firmware screens. App code should usually import `badge_ui`, which wraps
these native functions with Python conveniences like `ui.chrome(...)`.
`badge_ui` also includes `ui.hint(...)`, `ui.hint_text(...)`, and
`ui.hint_row(...)` helpers for composing multiple glyph-backed action hints
without hand-building strings throughout an app.

### `ui_header(title, [right])`

Draw the standard small header and divider. `right` is optional top-right text.

### `ui_action_bar([left_button], [left_label], [right_button], [right_label])`

Draw footer actions with native button glyphs. Button names include `OK`,
`BACK`, `X`, `Y`, `A`, and `B`; semantic names respect the badge's confirm/back
swap setting.

### `ui_chrome(title, [right], [left_button], [left_label], [right_button], [right_label])`

Clear the OLED buffer, draw the standard header, then draw the footer action
bar. Call `oled_show()` after drawing your screen content.

### `ui_inline_hint(x, y, hint)`

Draw inline text with native button glyph replacement, such as `"OK:start"` or
`"BACK quit"`. Returns the drawn width in pixels.

### `ui_inline_hint_right(right_x, y, hint)`

Right-align an inline hint to `right_x`. Returns the drawn width in pixels.

### `ui_measure_hint(hint)`

Return the pixel width an inline hint will use.

---

## Buttons & Joystick

Four face buttons and a 2-axis analog joystick. The directional constants are
kept for compatibility; new apps should prefer the semantic aliases where they
fit the interaction.

### Button Constants

```jython
BTN_RIGHT = 0
BTN_DOWN  = 1
BTN_LEFT  = 2
BTN_UP    = 3

BTN_CIRCLE   = BTN_RIGHT
BTN_CROSS    = BTN_DOWN
BTN_SQUARE   = BTN_LEFT
BTN_TRIANGLE = BTN_UP

BTN_CONFIRM = 4
BTN_SAVE    = BTN_CONFIRM
BTN_BACK    = 5
BTN_PRESETS = BTN_TRIANGLE
```

`BTN_CONFIRM`/`BTN_SAVE` and `BTN_BACK` follow the firmware
`swap_ok` setting: the default is B/Circle confirm and A/Cross back; setting
`swap_ok = 0` uses A/Cross confirm and B/Circle back. The physical constants and
PlayStation-style shape constants always refer to the actual hardware button.

### `button(id)`

Read the current state of a button.

* `id`: Button constant (`BTN_CONFIRM`, `BTN_BACK`, `BTN_PRESETS`, etc.).
* Returns `True` if the button is currently held down.

**Example:**

```jython
if button(BTN_CONFIRM):
    oled_println("confirm held!")
```

### `button_pressed(id)`

Edge-triggered button press detection. Returns `True` **once** per physical
press, then `False` until the button is released and pressed again. Consumes
the press event on read.

* `id`: Button constant.
* Returns `True` if a new press was detected since the last call.

**Example:**

```jython
import time
count = 0
while True:
    if button_pressed(BTN_CONFIRM):
        count += 1
        oled_clear()
        oled_println("Presses: " + str(count))
    time.sleep(0.02)
```

### `button_held_ms(id)`

Get how long a button has been continuously held.

* `id`: Button constant.
* Returns milliseconds the button has been held, or `0` if not pressed.

**Example:**

```jython
ms = button_held_ms(BTN_DOWN)
if ms > 1000:
    oled_println("Long press!")
```

### `joy_x()`

Read the joystick X axis.

* Returns an integer 0–4095. Center is approximately 2047.

### `joy_y()`

Read the joystick Y axis.

* Returns an integer 0–4095. Center is approximately 2047.

**Example:**

```jython
x = joy_x()
y = joy_y()
oled_clear()
oled_println("X:" + str(x) + " Y:" + str(y))
```

---

## LED Matrix (8×8)

8×8 LED matrix driven by the IS31FL3731 with per-pixel PWM brightness control.

### Image Constants

Builtin image names for use with `led_show_image()`:

```jython
IMG_SMILEY    = "smiley"
IMG_HEART     = "heart"
IMG_ARROW_UP  = "arrow_up"
IMG_ARROW_DOWN = "arrow_down"
IMG_X_MARK    = "x_mark"
IMG_DOT       = "dot"
```

### Animation Constants

Builtin animation names for use with `led_start_animation()`:

```jython
ANIM_SPINNER      = "spinner"
ANIM_BLINK_SMILEY = "blink_smiley"
ANIM_PULSE_HEART  = "pulse_heart"
```

### `led_brightness(value)`

Set the global LED brightness.

* `value`: Brightness level (0–255). `0` is off, `255` is maximum.

### `led_clear()`

Turn off all LEDs on the matrix.

### `led_fill([brightness])`

Turn on all LEDs. If `brightness` is omitted, uses the current global brightness.

* `brightness` (optional): Per-pixel brightness (0–255).

### `led_set_pixel(x, y, brightness)`

Set a single LED brightness.

* `x`: Column (0–7).
* `y`: Row (0–7).
* `brightness`: LED brightness (0–255).

**Example:**

```jython
led_clear()
led_set_pixel(3, 3, 100)
led_set_pixel(4, 4, 100)
```

### `led_get_pixel(x, y)`

Read the brightness of a single LED.

* Returns brightness value (0–255).

### `led_show_image(name)`

Display a builtin image on the matrix.

* `name`: Image name string (see Image Constants above).
* Returns `True` if the image was found.

**Example:**

```jython
led_show_image(IMG_HEART)

import time
time.sleep(2)
led_show_image("smiley")    # string name works too
```

### `led_set_frame(rows, [brightness])`

Draw an arbitrary 8×8 pattern from a list of row bitmasks. Each row is a uint8
where the MSB is the leftmost pixel.

* `rows`: List or tuple of exactly 8 integers (0–255), one per row.
* `brightness` (optional): On-pixel brightness (0–255). Defaults to current
  global brightness.

**Example:**

```jython
# Draw a smiley face
led_set_frame([
    0b00111100,
    0b01000010,
    0b10100101,
    0b10000001,
    0b10100101,
    0b10011001,
    0b01000010,
    0b00111100,
])

# Draw an X with explicit brightness
led_set_frame([
    0b10000001,
    0b01000010,
    0b00100100,
    0b00011000,
    0b00011000,
    0b00100100,
    0b01000010,
    0b10000001,
], 50)  # dim
```

### `led_start_animation(name, [interval_ms])`

Start a builtin animation on the matrix.

* `name`: Animation name string (see Animation Constants above).
* `interval_ms` (optional): Frame interval in milliseconds. Default 120ms.
* Returns `True` on success.

**Example:**

```jython
led_start_animation(ANIM_PULSE_HEART)

import time
time.sleep(5)
led_stop_animation()
```

### `led_stop_animation()`

Stop the currently running animation.

### `led_override_begin()`

Pause the saved ambient LED mode so a foreground app can draw on the matrix.
Call this before direct LED drawing when you want to temporarily override the
LED app.

### `led_override_end()`

Release a foreground LED override and restore the saved ambient LED mode.
Call this from cleanup paths after `led_override_begin()`.

### `led_screenshot([mode], [ansi])`

Dump the current 8×8 LED matrix to the serial console as a bordered
block-character image.

* `mode` (optional): Rendering size on the same ladder as `oled_screenshot()`:
  * `0.5` (float) — **quarter-block**, 4×4 chars (6 wide with border).
  * `0` (int) — **half-block**, 8×4 chars (10 wide with border).
  * `N` (int, ≥1) — **square** `2N` chars × `N` char-rows per LED.
  * `N.5` (float, ≥1.5) — **tall** escape hatch, `round(2(N−1))` chars × 1
    char-row per LED.

  Default: `1` (smallest square: 2 chars × 1 row per LED, 16 chars wide).

* `ansi` (optional): If `True`, lit LEDs print in bright red via ANSI escape
  codes. Requires a terminal that interprets ANSI (not PlatformIO device
  monitor). Default: `True`.

**Example:**

```jython
led_set_pixel(0, 0, 255)
led_set_pixel(7, 7, 255)
led_screenshot()          # int 1 — 2W × 2W square per LED
led_screenshot(0.5)       # float — quarter-block compact (4×4 chars)
led_screenshot(0)         # int 0 — half-block compact (8×4 chars)
led_screenshot(2, False)  # int 2 — bigger square, no ANSI colour
led_screenshot(1.5)       # float — 1 char × 1 char-row per LED (tall)
```

### `screenshot([mode], [ansi])`

Dump both the OLED and the LED matrix to the serial console in one call, with
the LED matrix centred horizontally below the OLED. The quickest way to
snapshot the full badge display state.

* `mode` (optional): Rendering size for the OLED, on the same ladder as
  `oled_screenshot()`. The LED renders **one size larger** (so each LED visual
  cell is a step bigger than an OLED pixel), staying in the same int/float
  family as the OLED:
  * `0.5` (float) — OLED quarter-block (66 wide). LED at float 1.5 (1 char ×
    1 row per LED, tall, 10 wide).
  * `0` (int) — OLED half-block (130 wide). LED at int 1 (square 2W × 2W per
    LED, 18 wide).
  * `N` (int, ≥1) — OLED pixel = 2N × N chars (square). LED cell =
    2(N+1) × (N+1) chars (square, one step larger).
  * `N.5` (float, ≥1.5) — OLED tall. LED also tall, one step larger.

  Default: `0`.

* `ansi` (optional): ANSI bright-red for lit LED pixels. Default: `True`.

**Example:**

```jython
screenshot()           # default — half-block OLED + small square LED
screenshot(0.5)        # quarter-block OLED + tiny tall LED
screenshot(1)          # OLED 2W square, LED 4W square
screenshot(2)          # OLED 4W square, LED 6W square
screenshot(1.5)        # tall OLED (1 char/pixel) + tall LED (3 chars/LED)
screenshot(0, False)   # default, no ANSI colour
```

**REPL hotkey:** typing a bare `o` followed by `<Enter>` at the MicroPython
REPL runs `screenshot()` with default arguments. `o` is a global singleton
whose `repr` triggers the screenshot, so `print(o)` and `repr(o)` work the
same way. Rebinding (`o = 42`) replaces the hotkey within the running
session.

**Docs workflow:** open the screen on the badge, connect over USB serial
(JumperIDE, `mpremote`, or `python3 serial_log.py`), type `o` at the REPL,
and copy or screenshot the terminal output. Default `screenshot()` uses
half-block OLED glyphs (readable on 2:1 monospace fonts) with the LED matrix
centred below. Use `screenshot(0, False)` if your terminal does not interpret
ANSI colour. The [Developer Guide](badge-developer-guide.md#serial-screenshots-oled--led)
has mode-ladder examples and terminal tips.

---

## Matrix App Host

Register a Python callback that the firmware calls periodically to render on
the 8×8 LED matrix in the background. This lets apps run LED animations that
continue while the main Python loop handles OLED and input, or even while
no Python code is running (the callback fires from the service pump).

### `matrix_app_start(callback, [interval_ms], [brightness])`

Register a callable to be invoked every `interval_ms` milliseconds.

* `callback`: A callable that accepts one argument — the current `millis()`
  timestamp. Pass `None` to clear the callback.
* `interval_ms` (optional): Tick interval in milliseconds. Minimum 16 ms.
  Defaults to the firmware's ambient interval.
* `brightness` (optional): LED brightness (0–255). Defaults to the firmware's
  ambient brightness.

**Example:**

```jython
frame = 0

def led_tick(now_ms):
    global frame
    led_clear()
    col = frame % 8
    for row in range(8):
        led_set_pixel(col, row, 80)
    frame += 1

matrix_app_start(led_tick, 100, 40)
```

### `matrix_app_set_speed(interval_ms)`

Change the tick interval for the active callback.

* `interval_ms`: New interval in milliseconds (minimum 16 ms).
* Returns the clamped interval.

### `matrix_app_set_brightness(brightness)`

Change the LED brightness for the active callback.

* `brightness`: New brightness (0–255).
* Returns the clamped brightness.

### `matrix_app_stop()`

Unregister the callback and restore default ambient LED mode.

### `matrix_app_active()`

Check if a background callback is currently registered.

* Returns `True` if a callback is active.

### `matrix_app_info()`

Return a diagnostic tuple with the current matrix app state.

* Returns `(active, saved, interval_ms, brightness, overridden, invocations)`.

---

## App Manifest

The firmware's `AppRegistry` discovers folder apps under `/apps/<slug>/` at
boot (and on demand via `rescan_apps()`) and exposes them on the main grid
menu. Each app's tile is decorated from a small set of optional top-level
dunder assignments inside `main.py`. The scanner reads the first ~2 KB of
the file as text and parses the assignments without executing any code, so
your dunders must sit at the top of the file with simple string literals.

### Dunders

Place these at the top of `/apps/<slug>/main.py`:

| Dunder | Type | Default | Purpose |
|--------|------|---------|---------|
| `__title__` | `str` (≤ 19 chars) | slug, title-cased | Main-menu tile label |
| `__description__` | `str` (≤ 63 chars) | empty | Detail panel text |
| `__icon__` | `str` (path or inline tuple) | tries `icon.py` | 12×12 home-screen icon |
| `__matrix_title__` | `str` (≤ 19 chars) | `__title__` | MATRIX APPS picker label (only when `matrix.py` is present) |
| `__order__` | `int` (signed) | `10000 + discovery index` | Sort key on the main grid; lower = earlier |

```jython
"""My Game — Tamagotchi-style desk pet."""

__title__       = "My Game"
__description__ = "A tiny pet that lives in your pocket."
__icon__        = "icon.py"
__matrix_title__ = "Pet"

# ... rest of main.py ...
```

The slug is the folder name. It must match `[A-Za-z0-9_-]+` and may not
start with a `.`; anything else is silently skipped by the scanner. The
registry holds at most 32 dynamic apps per badge.

### `icon.py` — Home-Screen Icon

Path resolution for `__icon__`:

* Bare filename (`"icon.py"`) → `/apps/<slug>/icon.py`
* Absolute path (`"/apps/foo/icon.py"`) → used as-is
* Inline tuple (`"(0xFF, 0x..., )"`) → parsed directly

The icon file just needs a top-level `DATA = (...)` tuple of 24 bytes
arranged as a 12×12 packed XBM (2 bytes per row × 12 rows). Bit 0 of each
byte is the leftmost pixel — same byte order as U8G2's `drawXBM`. The high
4 bits of every odd byte are unused (the row is only 12 wide). `WIDTH` /
`HEIGHT` are decorative; the firmware always reads 12×12.

```jython
"""My Game icon."""

WIDTH = 12
HEIGHT = 12
# Two bytes per row (low = cols 0..7, high = cols 8..11). Binary
# literals so the dot pattern is visible in the source. XBM is
# LSB-first, so reading the literal left-to-right gives the mirrored
# row — that's accepted, the bit values are still correct.
DATA = (
    0b01110111, 0b00000111,
    0b01110111, 0b00000111,
    0b00000000, 0b00000000,
    0b01100000, 0b00000000,
    0b01100000, 0b00000000,
    0b11111100, 0b00000011,
    0b00000000, 0b00000000,
    0b00111110, 0b00000000,
    0b00100000, 0b00000000,
    0b11100000, 0b00000011,
    0b00000000, 0b00000010,
    0b11000000, 0b00000011,
)
```

If `__icon__` is omitted, the registry still tries
`/apps/<slug>/icon.py` opportunistically. If that file is missing or
unparsable the tile falls back to the generic apps glyph.

### `matrix.py` — Persistent Matrix App

A sibling `matrix.py` next to `main.py` enables the app's slot in the
**MATRIX APPS** picker (main menu → MATRIX APPS). Selecting it persists the
slug to `/led_state.json` and re-sources `matrix.py` once on every boot
that selection survives. The script's only job is to register a callback
via [`matrix_app_start`](#matrix-app-host) and return — *do not* spin a
main loop.

```jython
"""Drifting-dot ambient."""

__matrix_title__ = "Drift"

import badge

_phase = 0

def _tick(now_ms):
    global _phase
    _phase = (_phase + 1) & 7
    frame = [0] * 8
    frame[7] = 0x80 >> _phase
    badge.led_set_frame(frame)

badge.matrix_app_start(_tick, 250, 24)
```

Same constraints as any other `matrix_app_start` callback: it runs from the
firmware service pump, so keep ticks fast and don't block. The firmware
tears down the previous callback for you when the user picks a different
matrix app or any built-in mode (Sparkle, Off, etc.) — there's no need to
call `matrix_app_stop()` from your script when switching modes.

### `rescan_apps()`

Force the registry to re-scan `/apps/` after editing manifests, without
rebooting:

```jython
import badge
badge.rescan_apps()
```

This rebuilds the main-menu grid in place. Existing screens stay open.

### Tile Order (`__order__`)

The main grid is rendered in stable-sort order by a signed `int16` key.
Three layers feed the key, each overriding the previous one:

1. **Defaults.** Curated tiles use `10 × array index` (with `SETTINGS`
   pinned to `30000` so it stays at the back). Dynamic apps default to
   `10000 + discovery index`.
2. **App manifest.** `__order__ = 50` at the top of `main.py` claims a
   specific slot. Any signed integer literal works.
3. **User override.** The manual reorder UI (Settings → Menu → Reorder)
   writes per-label overrides into the `menu_order` NVS namespace;
   those override both of the above.

Ties resolve by insertion order — duplicate keys keep the order
established by the previous layer. Negative keys land before all
curated tiles; large keys land near `SETTINGS`.

```jython
__order__ = -10   # before BOOP / CONTACTS / …
__order__ = 25    # between MAP (30) and SCHEDULE (40)
__order__ = 9999  # nearly last
```

### Manual Reorder Screen

Players can rearrange the grid themselves at runtime:

> Settings → **Menu → Reorder**

| Button | Action |
|--------|--------|
| Joystick Y | Move cursor up/down |
| `X` | Pick up / drop a row |
| `A` (confirm) | Save and rebuild |
| `B` (back) | Cancel without saving |

While picked up, dragging Y swaps the row through the list in real
time. Saving writes the new positions into NVS (`menu_order`
namespace, FNV-1a-hashed labels as keys). **Settings → Menu → Reset
Order** wipes the namespace and returns every tile to its default
order.

---

## IMU (Accelerometer)

LIS2DH12 3-axis accelerometer for tilt sensing and motion detection.

### `imu_ready()`

Check if the IMU is initialized and taking readings.

* Returns `True` if the IMU is ready.

### `imu_tilt_x()`

Read the smoothed X-axis tilt.

* Returns a float in milli-g (mG). Typical range ±1000 mG.

### `imu_tilt_y()`

Read the smoothed Y-axis tilt.

* Returns a float in milli-g (mG). Typical range ±1000 mG.

### `imu_accel_z()`

Read the Z-axis acceleration.

* Returns a float in milli-g (mG). ~1000 mG when stationary (1g gravity).

### `imu_face_down()`

Check if the badge is face-down.

* Returns `True` if the badge is face-down (Z-axis below threshold).

### `imu_motion()`

Check for a motion event. **Consumes** the event on read — calling again
returns `False` until new motion is detected.

* Returns `True` if motion was detected since the last call.

**Example:**

```jython
import time

while True:
    if imu_motion():
        oled_clear()
        oled_println("Motion!")
        haptic_pulse()

    x = imu_tilt_x()
    y = imu_tilt_y()

    # Map tilt to LED matrix pixel
    px = int((x + 1000) / 250)
    py = int((y + 1000) / 250)
    px = max(0, min(7, px))
    py = max(0, min(7, py))

    led_clear()
    led_set_pixel(px, py, 100)
    time.sleep(0.05)
```

---

## Haptics

Vibration motor with PWM control and audible coil tone support. The motor can
produce haptic feedback pulses or, at very low duty cycles, audible tones from
coil vibration.

### `haptic_pulse([strength], [duration_ms], [freq_hz])`

Fire a haptic vibration pulse. All parameters are optional — omitted values use
the configured defaults (strength ~155, duration ~35ms, frequency ~80Hz).

* `strength` (optional): Motor intensity (0–255).
* `duration_ms` (optional): Pulse duration in milliseconds.
* `freq_hz` (optional): PWM carrier frequency in Hz.

**Example:**

```jython
haptic_pulse()              # Default pulse
haptic_pulse(200)           # Stronger
haptic_pulse(100, 50)       # Medium strength, 50ms
haptic_pulse(255, 100, 150) # Full strength, 100ms, 150Hz carrier
```

### `haptic_strength([value])`

Get or set the default motor strength used by `haptic_pulse()`.

* `value` (optional): New strength (0–255). If omitted, just returns current.
* Returns the current strength (0–255).

**Example:**

```jython
print(haptic_strength())    # Read current
haptic_strength(200)        # Set to 200
```

### `haptic_off()`

Immediately stop the motor and cancel any active pulse.

### `tone(freq_hz, [duration_ms], [duty])`

Play an audible tone from the motor coil. At very low duty cycles (~30/255),
the coil doesn't spin but vibrates audibly at the PWM frequency.

* `freq_hz`: Tone frequency in Hz.
* `duration_ms` (optional): Duration in milliseconds. `0` or omitted = play
  until `no_tone()`.
* `duty` (optional): Duty cycle (0–255). Default 30.

**Example:**

```jython
import time

# Play a scale
for freq in [262, 294, 330, 349, 392, 440, 494, 523]:
    tone(freq, 200)
    time.sleep(0.25)

# Continuous tone until stopped
tone(440)
time.sleep(2)
no_tone()
```

### `no_tone()`

Stop the currently playing tone.

### `tone_playing()`

Check if a tone is currently playing.

* Returns `True` if a tone is active.

---

## IR Send/Receive

Infrared communication over a modified NEC protocol using the onboard IR LED
(TX) and TSOP receiver (RX). The RMT-driven encoder automatically prepends an
NEC leader, packs each payload word, appends a CRC32, and emits a trailing
pulse — MicroPython just picks the payload.

Two payload shapes are available:

* **Classic 1-byte/1-byte frames** via `ir_send(addr, cmd)` / `ir_read()`.
  A single 32-bit NEC word `(~addr, addr, ~cmd, cmd)`, ~110 ms on wire.
* **Multi-word frames** via `ir_send_words(words)` / `ir_read_words()`.
  1–64 raw 32-bit words (up to 256 bytes of payload) with an appended CRC.
  Each data word adds ~54 ms, so a 3-word frame ≈ 230 ms on wire.

The Boop screen and MicroPython share the same RMT hardware; `ir_start()`
brings the radio up for Python use and waits for it to be ready before
returning, so the first `ir_send*()` after `ir_start()` will not race.

### `ir_send(addr, cmd)`

Transmit a single classic NEC frame.

* `addr`: NEC address byte (0–255).
* `cmd`: NEC command byte (0–255).

Blocks briefly while the RMT hardware streams the frame. Returns `0` on
success; raises `OSError` if the IR hardware is down (e.g. called before
`ir_start()`).

### `ir_start()`

Bring the IR hardware up and start receiving in Python mode. Incoming frames
are queued in an 8-slot ring buffer for retrieval with `ir_read()` or
`ir_read_words()`. `ir_start()` blocks up to ~500 ms while Core 0 powers the
RMT channel, and drains any stale frames left over from the Boop screen.

### `ir_stop()`

Stop Python IR RX and drop any queued frames. Does **not** power down the
hardware — the Boop screen may still use it.

### `ir_available()`

Check if at least one classic NEC frame is waiting in the RX queue.

* Returns `True` if `ir_read()` would return a frame.

### `ir_read()`

Pop one classic NEC frame from the queue.

* Returns `(addr, cmd)` if a frame is available, or `None` if the queue
  is empty.

**Example:**

```jython
import time

ir_start()
oled_println("Listening for IR...")

while True:
    if ir_available():
        frame = ir_read()
        if frame:
            addr, cmd = frame
            oled_clear()
            oled_println("IR: " + hex(addr) + " " + hex(cmd))
            haptic_pulse()
    time.sleep(0.05)
```

### `ir_send_words(words)`

Transmit a multi-word NEC frame. The encoder automatically emits a leader,
then each word LSB-first, then a CRC32 over the payload.

* `words`: A list, tuple or other sequence of 1–64 integers. Each element is
  converted to an unsigned 32-bit value.
* Raises `ValueError` if the sequence is empty or longer than 64 words.
* Raises `OSError(1)` if the IR hardware is not up (e.g. before `ir_start()`).

**Example:**

```jython
ir_start()
ir_send_words([0xB0, 0xDEADBEEF, 0x12345678])
```

### `ir_read_words()`

Read one received multi-word NEC frame.

* Returns a `tuple` of up to 64 integers (the payload words, CRC-validated
  and stripped by the decoder) if a frame is available.
* Returns `None` if the RX queue is empty.

### `ir_flush()`

Drop every pending RX frame (both classic and multi-word queues). Safe to
call at any time; a no-op if the IR hardware is down.

### `ir_tx_power([percent])`

Get or set the IR carrier duty cycle. A higher duty drives the IR LED harder
and extends range at the cost of current and LED stress.

* `percent` (optional): New duty cycle in the range **1–50** (percent of the
  38 kHz carrier period). Call with no argument to just read the current
  value. Power-on default is **50**.
* Returns the current duty as an integer percent after the (optional) set.
* Raises `ValueError` if `percent` is outside 1–50.

**Example:**

```jython
ir_start()
print("Default duty:", ir_tx_power())   # 50
ir_tx_power(10)                          # throttle down for self-loopback
```

---

## Badge Identity & Boops

### `my_uuid()`

Return this badge's globally unique identifier, derived from the ESP32-S3
eFuse `OPTIONAL_UNIQUE_ID` (first 6 bytes, hex-encoded).

* Returns a 12-character lowercase hex string, e.g. `"a1b2c3d4e5f6"`.

### `boops()`

Return the on-flash `/boops.json` contents as a string. This is the same
document that the Boop screen maintains — each completed boop is appended
with the peer UID, peer name / ticket (if the server backfilled them), and
a `status` field (`"ok"`, `"local"`, etc.).

* Returns a JSON string. When no boops have been recorded yet, returns
  `'{"pairings":[]}'` so callers can always `json.loads()` the result.

**Example:**

```jython
import json

data = json.loads(boops())
for p in data.get("pairings", []):
    oled_println(p.get("peer_badge_uid", "?"))
oled_show()
```

---

## Script Control

### `exit()`

Cleanly stop the currently running MicroPython app by raising `SystemExit`.
Prefer this over `sys.exit()` so the host runtime sees the same exception
type the rest of the firmware expects.

Holding all four face buttons for about 1 second also force-exits the running
app from outside.

### `dev(*args)` *(dev builds only)*

Variadic string-argument dispatcher for the firmware test harness. Only
available in builds with `BADGE_ENABLE_MP_DEV` (e.g. the `echo-dev`
PlatformIO environment). Each argument is coerced to a string and forwarded
to the C++ runtime. Returns a string result.

---

## Filesystem Access

The badge's VFS is mounted and accessible through the standard `os` module.
Apps can read and write files for saving state, high scores, or configuration.

```jython
import os

os.listdir("/apps")
os.listdir("/")

with open("/apps/my_app/save.json", "w") as f:
    f.write('{"score": 42}')

with open("/apps/my_app/save.json", "r") as f:
    data = f.read()
```

Available `os` operations: `listdir`, `mkdir`, `remove`, `rename`, `stat`,
`getcwd`, `chdir`, `ilistdir`, `statvfs`. Standard `open()` / `read()` /
`write()` / `close()` work as expected.

---

## Mouse Overlay

Hardware-composited cursor overlay for building GUIs and games. When enabled,
a cursor sprite is automatically drawn on top of the OLED framebuffer at every
display refresh. The joystick controls cursor position and face buttons
generate click events — all handled asynchronously in the service pump so
Python code only needs to query position and clicks.

### `mouse_overlay(enable)`

Enable or disable the cursor overlay.

* `enable`: `True` to enable, `False` to disable.

When enabled, the joystick moves the cursor and button presses are captured
as click events (instead of being consumed by `button_pressed()`).

**Example:**

```jython
mouse_overlay(True)
mouse_set_pos(64, 32)

while True:
    oled_clear()
    oled_set_cursor(0, 0)
    oled_print("x:" + str(mouse_x()) + " y:" + str(mouse_y()))
    oled_show()

    btn = mouse_clicked()
    if btn == BTN_RIGHT:
        oled_println("Clicked!")
    if btn == BTN_LEFT:
        break
    time.sleep_ms(30)

mouse_overlay(False)
```

### `mouse_set_bitmap(data, w, h)`

Set a custom cursor sprite. Format is a packed 1-bit-per-pixel bitmap,
row-major, **MSB-first within each byte** (the leftmost pixel of a row is
bit 7 of its first byte). Each row is padded to a whole number of bytes, so
`ceil(w / 8) * h` bytes are read from `data`.

* `data`: `bytes` or `bytearray` containing the bitmap.
* `w`: Width in pixels (1–32). Values larger than 32 are clamped to 32.
* `h`: Height in pixels (1–32). Values larger than 32 are clamped to 32.

The internal cursor buffer is 128 bytes, so any `(w, h)` with
`ceil(w / 8) * h ≤ 128` is accepted. A 32×32 sprite uses the entire buffer.
The hot-spot is automatically set to the sprite's center. The default cursor
is an 8×8 arrow pointer.

**Example:**

```jython
# 8x8 crosshair cursor
crosshair = bytes([
    0b00010000,
    0b00010000,
    0b00010000,
    0b11101110,
    0b00010000,
    0b00010000,
    0b00010000,
    0b00000000,
])
mouse_set_bitmap(crosshair, 8, 8)
```

### `mouse_x()`

Returns the current cursor X position (0–127).

### `mouse_y()`

Returns the current cursor Y position (0–63).

### `mouse_set_pos(x, y)`

Warp the cursor to an absolute position.

* `x`: X position (clamped to 0–127).
* `y`: Y position (clamped to 0–63).

### `mouse_clicked()`

Read the last click event. Returns the button ID (`BTN_RIGHT`, `BTN_DOWN`,
`BTN_LEFT`, `BTN_UP`) or `-1` if no click is pending. **Consumes** the event
on read — calling again returns `-1` until a new button is pressed.

### `mouse_set_speed(speed)`

Set the cursor movement speed (only affects relative mode).

* `speed`: Pixels per joystick poll at full deflection (1–20). Default is 3.

**Example:**

```jython
mouse_set_speed(5)   # faster cursor
mouse_set_speed(1)   # very precise
```

### `mouse_set_mode(mode)`

Switch between absolute and relative positioning.

* `mode`: `MOUSE_ABSOLUTE` (joystick position = cursor position) or
  `MOUSE_RELATIVE` (joystick deflection = cursor velocity). Default is
  `MOUSE_RELATIVE`.

In **absolute** mode the cursor tracks the joystick 1:1 — stick center is
screen center. In **relative** mode the joystick acts like a mouse — deflect
to move, release to stop. Use `mouse_set_speed()` and `mouse_set_pos()` to
tune relative mode.

**Example:**

```jython
mouse_set_mode(MOUSE_RELATIVE)
mouse_set_pos(64, 32)
mouse_set_speed(4)
```

---

## Constants Reference

### Buttons

| Constant | Value | Description |
|----------|-------|-------------|
| `BTN_RIGHT` | 0 | Right button |
| `BTN_DOWN` | 1 | Down button |
| `BTN_LEFT` | 2 | Left button |
| `BTN_UP` | 3 | Up button |
| `BTN_CIRCLE` | 0 | PlayStation-style alias for right |
| `BTN_CROSS` | 1 | PlayStation-style alias for down |
| `BTN_SQUARE` | 2 | PlayStation-style alias for left |
| `BTN_TRIANGLE` | 3 | PlayStation-style alias for up |
| `BTN_CONFIRM` | 4 | Semantic confirm/select, follows `swap_ok` |
| `BTN_SAVE` | 4 | Semantic save/apply, follows `swap_ok` |
| `BTN_BACK` | 5 | Semantic back/cancel, follows `swap_ok` |
| `BTN_PRESETS` | 3 | Semantic alias for preset/actions |

### LED Matrix Images

| Constant | Value | Description |
|----------|-------|-------------|
| `IMG_SMILEY` | `"smiley"` | Smiley face |
| `IMG_HEART` | `"heart"` | Heart shape |
| `IMG_ARROW_UP` | `"arrow_up"` | Upward arrow |
| `IMG_ARROW_DOWN` | `"arrow_down"` | Downward arrow |
| `IMG_X_MARK` | `"x_mark"` | X mark |
| `IMG_DOT` | `"dot"` | Center dot |

### LED Matrix Animations

| Constant | Value | Description |
|----------|-------|-------------|
| `ANIM_SPINNER` | `"spinner"` | Rotating spinner |
| `ANIM_BLINK_SMILEY` | `"blink_smiley"` | Blinking smiley face |
| `ANIM_PULSE_HEART` | `"pulse_heart"` | Pulsing heart |

### Mouse Overlay Modes

| Constant | Value | Description |
|----------|-------|-------------|
| `MOUSE_ABSOLUTE` | 1 | Joystick position = cursor position |
| `MOUSE_RELATIVE` | 0 | Joystick deflection = cursor velocity (default) |
