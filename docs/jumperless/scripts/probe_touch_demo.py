# Wait for probe touch, then wait for probe button. Uses global API (no j. prefix).

print("Touch a pad...")
pad = probe_read()
print("You touched: " + str(pad))

if pad == D13_PAD:
    print("That's the Arduino LED pin!")

print("Press a probe button...")
button = get_button()
if button == CONNECT_BUTTON:
    print("Connect button pressed.")
    oled_print("Connect button pressed.")