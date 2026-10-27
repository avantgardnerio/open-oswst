# Move a graphic overlay by touching breadboard pads with the probe.

import time

colors_2d = [
    [0x550000, 0x104000, 0x005500, 0x001040, 0x000055],  # Row 0
    [0x550000, 0x104000, 0x005500, 0x001040, 0x000055],  # Row 1
    [0x550000, 0x104000, 0x005500, 0x001040, 0x000055],  # Row 2
    [0x550000, 0x104000, 0x005500, 0x001040, 0x000055],  # Row 3
    [0x550000, 0x104000, 0x005500, 0x001040, 0x000055],  # Row 4
]

overlay_set("box_2d", 12, 5, 5, 5, colors_2d)

while True:
    node = probe_read_blocking()
    overlay_place("box_2d", int(node), 5)
    time.sleep(0.1)
    if clickwheel_get_button() == CLICKWHEEL_PRESSED:
        break
    

overlay_clear_all()