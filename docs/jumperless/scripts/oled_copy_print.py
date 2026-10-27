# Debug output to OLED: print() appears on both serial and OLED.
# Useful when you don't have serial connected.

import time

oled_copy_print(True)

for i in range(10):
    voltage = adc_get(0)
    print("V{}: {:.2f}V".format(i, voltage))
    time.sleep(0.5)

oled_copy_print(False)
