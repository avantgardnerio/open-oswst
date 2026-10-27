"""
Simple FakeGPIO Node-Based API Test

Quick test to verify the new node-based FakeGPIO API works correctly.
Uses TOP_RAIL and BOTTOM_RAIL for differential output.

This script doesn't work, FakeGPIO output is disabled for now.
"""


print("\n" + "="*70)
print("FAKE GPIO NODE-BASED API TEST")
print("="*70)

# Clear connections
print("\n→ Clearing connections...")
nodes_clear()
time.sleep(0.2)

# Test 4: Create INPUT pin
print("\n→ Test 4: Creating INPUT pin with loopback")
print("  Creating loopback: node 10 → node 15 (TX → RX)")
nodes_connect(10, 15)

rx_pin = FakeGpioPin(15, INPUT, 2.0, 0.8)
print("  ✓ RX pin created")

# Test reading with loopback
print("\n→ Test 5: Reading loopback values")
for i in range(5):
    pin_a.on()
    time.sleep_us(1000)
    rx_val = rx_pin.value()
    print(f"  TX=HIGH → RX reads: {rx_val} (expected: 1)")
    
    pin_a.off()
    time.sleep_us(1000)
    rx_val = rx_pin.value()
    print(f"  TX=LOW  → RX reads: {rx_val} (expected: 0)")

print("\n" + "="*70)
print("ALL TESTS COMPLETE")
print("="*70)
print("\nIf all tests passed, the node-based FakeGPIO API is working correctly!")
print("You can now run the RS-485 terminal: fake_gpio.py")
print("="*70 + "\n")

