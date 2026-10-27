"""
Basic GPIO (General Purpose Input/Output) operations.
This example shows digital I/O, direction control, and pull resistors.
"""

print("GPIO Basics Demo")

# Test GPIO pin 1
pin = 1
print("Testing GPIO pin " + str(pin))

# Set as output
gpio_set_dir(pin, True)  # True = OUTPUT
print("Set as output")

# Blink test
print("Blinking 5 times...")
for i in range(5):
    gpio_set(pin, True)   # HIGH
    print("  GPIO" + str(pin) + " = HIGH")
    time.sleep(0.5)
    
    gpio_set(pin, False)  # LOW
    print("  GPIO" + str(pin) + " = LOW")
    time.sleep(0.5)

# Set as input
gpio_set_dir(pin, INPUT)  # False = INPUT
print("Set as input")

# Test pull resistors
pulls = [0, 1, -1]  # None, Up, Down
pull_names = ["NONE", "PULLUP", "PULLDOWN"]

for i, pull in enumerate(pulls):
    gpio_set_pull(pin, pull)
    state = gpio_get(pin)
    print("Pull " + pull_names[i] + ": " + str(state))
    time.sleep(1)

print("GPIO Basics complete!")

