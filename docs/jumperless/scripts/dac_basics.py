"""
Basic DAC (Digital-to-Analog Converter) operations.
This example shows how to set DAC voltages.

Hardware Setup:
1. Connect voltmeter or LED to DAC output pins
2. DAC channels: 0=DAC_A, 1=DAC_B, 2=TOP_RAIL, 3=BOTTOM_RAIL
"""


print("DAC Basics Demo")

# Test all DAC channels
channels = [0, 1, 2, 3]
channel_names = ["DAC_A", "DAC_B", "TOP_RAIL", "BOTTOM_RAIL"]

for i, channel in enumerate(channels):
    print("\nTesting " + channel_names[i] + " (channel " + str(channel) + "):")
    
    # Set different voltages
    voltages = [0.0, 1.65, 3.3]
    for voltage in voltages:
        dac_set(channel, voltage)
        actual = dac_get(channel)
        print("  Set: " + str(voltage) + "V, Read: " + str(round(actual, 3)) + "V")
        time.sleep(1)
    
    # Reset to 0V
    dac_set(channel, 0.0)

print("\nDAC Basics complete!")

