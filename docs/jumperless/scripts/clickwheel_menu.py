# Menu navigation with clickwheel: turn to select, press to choose, double-click to exit.

import time

menu_items = ["Option 1", "Option 2", "Option 3", "Option 4"]
selected = 0

clickwheel_reset_position()

while True:
    direction = clickwheel_get_direction()
    if direction == CLICKWHEEL_UP:
        selected = (selected + 1) % len(menu_items)
        oled_print("> {}".format(menu_items[selected]))
        print("> {}".format(menu_items[selected]))
    elif direction == CLICKWHEEL_DOWN:
        selected = (selected - 1) % len(menu_items)
        oled_print("> {}".format(menu_items[selected]))
        print("> {}".format(menu_items[selected]))


    button = clickwheel_get_button()
    if button == CLICKWHEEL_PRESSED:
        oled_print("Selected: {}".format(menu_items[selected]))
        print("Selected: {}".format(menu_items[selected]))
    elif button == CLICKWHEEL_HELD:
        oled_print("Exit menu")
        print("Exit menu")
        break

    time.sleep(0.05)

value = 50

