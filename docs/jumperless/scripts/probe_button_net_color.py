# Probe button: hold Connect/Remove to change net 0 hue. Stop with Ctrl+C.
# One-shot example (count 5 presses) is in comments at bottom.

import time

hue = 0
while True:
    button = check_button()
    if button == CONNECT_BUTTON:
        hue += 1
        set_net_color_hsv(0, hue % 256)
    elif button == REMOVE_BUTTON:
        hue -= 1
        set_net_color_hsv(0, hue % 256)
    time.sleep(0.05)

# One-shot detection (uncomment to use instead):
# presses = 0
# while presses < 5:
#     button = get_button(consume=True)
#     if button == CONNECT_BUTTON:
#         presses += 1
#         print("Press #{}".format(presses))
