# Capture OLED to file, clear display, then restore from file.

import time

fb = oled_get_framebuffer()
print("Framebuffer size: {} bytes".format(len(fb)))

with open("/screen_capture.bin", "wb") as f:
    f.write(fb)

oled_clear()
time.sleep(2)

with open("/screen_capture.bin", "rb") as f:
    fb_data = f.read()

if oled_set_framebuffer(fb_data):
    print("Screen restored!")
else:
    print("Wrong framebuffer size")
