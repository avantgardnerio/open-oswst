# Arduino Stuff

## UART Passthrough

With an Arduino Nano in the header and the UART lines connected, anything on those lines should be passed through to the second serial port that shows up when you plug in your Jumperless. 

Don't worry about the baud rate, the Jumperless senses what the host computer is set to and changes the speed accordingly. Same for the parity and stop bits, so stuff that isn't 8N1 goes through (UPDI wants 8E2, one of the beta testers found that out flashing an ATtiny412 through it, that whole saga is in [this Crowd Supply update](https://www.crowdsupply.com/architeuthis-flux/jumperless-v5/updates/a-series-of-tubes)).

The Nano goes in the header socket on the board, and every signal pin on that header is a routable node - `D0` through `D13`, `A0` through `A7`, `AREF` and `RESET`. You can connect any of them to a breadboard row or a rail just like anything else. The power pins (`5V`, `3V3`, `VIN`, `GND`) aren't routable. `D0` and `D1` are the only ones that ever get wired up for you.

<img width="360" alt="Screenshot 2025-05-19 at 11 07 55 AM" src="https://github.com/user-attachments/assets/2b255e34-0d0a-4e86-b577-d59c9561fa42" />

## Quick Connection Shortcuts

The shortcuts to connect `D0` and `D1` to the Jumperless's UART `Tx` and `Rx` is `A` to connect, and `a` to disconnect. Add a `?` to ask instead of doing - `A?` tells you whether the UART is routed and whether a Nano is there, `a?` just whether it's routed.

`r` resets the Nano. `rt` and `rb` pick which `RESET` pad to pulse, and `r2` does both.

You don't need `A` before an upload, flashing routes the UART for you. It's only there for talking to the sketch afterwards if you'd disconnected it.

## Automatic Flashing

It will even sense when Arduino IDE is trying to upload code and twiddle the reset lines to allow you to flash code with just a single USB cable going to your Jumperless.

That reset twiddling is set up for a classic AVR Nano. If you've got a Nano ESP32 in the header, set the `[serial_1]` config key `flash_reset_type` to `esp32` or the upload will never get it into the bootloader.

![](assets/arduinoPort.png)

<!-- <img width="1277" alt="Screenshot 2025-05-19 at 11 09 38 AM" src="https://github.com/user-attachments/assets/625ebf79-7308-4abb-a321-f1bf1f713d4f" />  -->

**Tip:** You can also use [Wokwi](https://wokwi.com) with the Jumperless Bridge app for flashing - no need to even have the Arduino IDE open

---

## Commands from Routable UART

You can send commands to the Jumperless from your Arduino (or anything connected to the routable UART) by wrapping them in XML-style tags. The command inside the tags gets executed. The tagged text still shows up on the passthrough port along with everything else your Arduino prints, it doesn't get stripped out - but it's never echoed back down the UART to the Arduino. (there's a demo of netlists going in over the UART in [this Crowd Supply update](https://www.crowdsupply.com/architeuthis-flux/jumperless-v5/updates/jumperless-probelessly).)

### Two Types of Tags

There are two flavors of command tags, depending on what you want to do:

#### `<j>` Tags - Raw Commands
These run exactly like you typed them in the main Jumperless menu. Use these for things like making connections with `f`, loading files, or any single-character menu command. They don't send anything back to the Arduino.

#### `<p>` Tags - Python Commands  
These run MicroPython commands directly. Perfect for `connect()`, `disconnect()`, `adc_get()`, `dac_set()`, and all the other Python hardware functions. The `<p>` tag automatically prepends the `>` that normally tells the Jumperless "this is a Python command." A `<p>` command sends back exactly what it prints and nothing else, so wrap anything you want to read in `print()`.

### Supported Tag Names

Any of these work (use matching opening and closing tags):

| Tag | Example |
|-----|---------|
| `<j>` | `<j>f 1-30</j>` |
| `<jumperless>` | `<jumperless>x</jumperless>` |
| `<jumperlessCommand>` | `<jumperlessCommand>n</jumperlessCommand>` |
| `<p>` | `<p>print(adc_get(0))</p>` |

---

## Python Commands with `<p>` Tags

The `<p>` tag is the most powerful way to control your Jumperless from Arduino code. It gives you direct access to all the MicroPython hardware functions.

### Basic Example

```cpp
#define OPENJCOMMAND Serial.print("<p>");
#define CLOSEJCOMMAND Serial.println("</p>");

void setup() {
  Serial.begin(115200);
  delay(1500);  // Give Jumperless time to boot
}

void loop() {
  // Read voltage on ADC channel 0
  OPENJCOMMAND
  Serial.print("print(adc_get(0))");
  CLOSEJCOMMAND
  delay(100);
  
  // Read the response
  while(Serial.available() > 0) {
    char c = Serial.read();
    // Process the voltage reading...
  }
}
```





### Full Example - ADC Scanning

<video autoplay loop muted playsinline controls width="100%">
  <source src="https://github.com/user-attachments/assets/fcdae6a6-ef4c-4fe7-9d5c-74cee5946c08" type="video/mp4">
  Your browser does not support the video tag.
</video>

This sketch connects ADC0 to different breadboard rows and reads the voltage at each one:

```cpp
#define OPENJCOMMAND Serial.print("<p>");
#define CLOSEJCOMMAND Serial.println("</p>");

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  delay(1500);
}

int lastNode = 8;
int node = 8;
unsigned long delayTime = 60;

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(delayTime);
  
  node++;
  if (node > 60) {
    node = 1;
  }

  // Disconnect from previous node
  OPENJCOMMAND
  Serial.print("disconnect( ADC0," + String(lastNode) + ")");
  CLOSEJCOMMAND
  delay(delayTime);

  // Connect to new node
  OPENJCOMMAND
  Serial.print("connect(ADC0 ," + String(node) + ")");
  CLOSEJCOMMAND
  delay(delayTime);

  // Read the voltage
  OPENJCOMMAND
  Serial.print("print(adc_get(0))");
  CLOSEJCOMMAND
  delay(delayTime);

  // Read response from Jumperless
  char response[30] = {0};
  int idx = 0;
  while(Serial.available() > 0 && idx < 29) {
    response[idx++] = Serial.read();
    delay(5);
  }
  
  Serial.println(response);
  Serial.print("num chars read = ");
  Serial.println(idx);
  Serial.flush();

  lastNode = node;
  digitalWrite(LED_BUILTIN, LOW);
}
```

### Available Python Functions

Here are the most useful functions you can call with `<p>` tags:

```cpp
// Connections
"connect(1, 30)"              // Connect breadboard rows
"connect(D13, TOP_RAIL)"      // Connect Arduino pin to power
"disconnect(ADC0, 15)"        // Remove a connection
"nodes_clear()"               // Clear ALL connections

// Analog I/O
"adc_get(0)"                  // Read voltage (channels 0-7, 0-3 are the routable ones)
"dac_set(0, 3.3)"            // Set DAC output voltage
"dac_set(TOP_RAIL, 5.0)"     // Set rail voltage

// Digital I/O
"gpio_set(1, True)"          // Set GPIO high
"gpio_set(1, False)"         // Set GPIO low
"gpio_get(2)"                // Read GPIO state

// Current sensing
"ina_get_current(0)"         // Read current in amps
"ina_get_voltage(0)"         // Read bus voltage at the current sensor
```

See the [MicroPython API Reference](09.5-micropythonAPIreference.md) for the complete list.

---

## Raw Commands with `<j>` Tags

Use `<j>` tags when you want to send menu commands - the same ones you'd type in the serial terminal.

### Example - Making Connections

```cpp
#define OPENJCOMMAND Serial.print("<j>");
#define CLOSEJCOMMAND Serial.println("</j>");

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  delay(1500);
}

int node1 = 1;
int node2 = 8;
unsigned long delayTime = 60;

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(delayTime);
  
  node1++;
  node2++;
  if (node1 > 60) node1 = 1;
  if (node2 > 60) node2 = 1;

  // Use the 'f' command to make a connection
  // Format: f <node1>-<node2>
  OPENJCOMMAND
  Serial.print("f " + String(node1) + "-" + String(node2) + "\n");
  CLOSEJCOMMAND
  delay(delayTime);

  digitalWrite(LED_BUILTIN, LOW);
}
```

### Useful Raw Commands

| Command | What it does |
|---------|--------------|
| `f 1-30` | Connect nodes 1 and 30 |
| `+ D13-GND` | Add connection (alternate syntax) |
| `- 1-30` | Remove connection |
| `x` | Clear all connections |
| `n` | Show net list |

---

## Tips and Gotchas

### Timing
The Jumperless needs a little time to process each command. A delay of 40-100ms between commands is usually safe. If you're seeing weird behavior, try increasing the delay.

### Response Reading
Only `<p>` commands send anything back, and only what they print - `print(adc_get(0))` gets you a voltage, `adc_get(0)` on its own gets you nothing. `<j>` commands are fire-and-forget from the Arduino's side. Make sure to read the Serial buffer after a command that does return data, or it'll fill up and cause issues.

### Startup Delay
Add a `delay(1500)` in your `setup()` to give the Jumperless time to fully boot before sending commands.

### Flashing Your Arduino
Just use the Arduino IDE normally - select the second serial port that shows up (the one labeled with "port3" or similar), and upload. The Jumperless automatically handles the reset timing.

### Which Tag to Use?
- **Use `<p>`** for anything that's a Python function: `connect()`, `adc_get()`, `dac_set()`, etc.
- **Use `<j>`** for menu commands: `f`, `x`, `n`, etc.

---

## Wokwi Integration

If you're using the [Jumperless Wokwi Bridge](https://github.com/Architeuthis-Flux/Jumperless-Wokwi-Bridge), you can flash your Arduino directly from Wokwi simulations - the bridge handles all the communication for you.
