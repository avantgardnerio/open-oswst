# Capture OLED framebuffer, invert pixels, display, then restore original.

import time

fb = oled_get_framebuffer()
inverted = bytearray(fb)
for i in range(len(inverted)):
    inverted[i] = ~inverted[i] & 0xFF

oled_set_framebuffer(inverted)
time.sleep(1)
oled_set_framebuffer(fb)
