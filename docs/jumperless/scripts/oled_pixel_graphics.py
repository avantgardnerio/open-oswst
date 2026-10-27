# Draw a diagonal line and a box using oled_set_pixel.

oled_clear()
for i in range(32):
    oled_set_pixel(i, i, 1)
oled_show()

for x in range(20, 108):
    oled_set_pixel(x, 10, 1)
    oled_set_pixel(x, 22, 1)
for y in range(10, 23):
    oled_set_pixel(20, y, 1)
    oled_set_pixel(107, y, 1)
oled_show()
