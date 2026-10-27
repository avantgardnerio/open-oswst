# LiPo Cell Characterizer for Jumperless V5
# Measures capacity (mAh), ESR, and logs charge/discharge curves
#
# WIRING:
#   Battery (+) -> Row 20
#   Battery (-) -> GND (use row 30 for visual)
#   Load resistor: between row 20 and row 40 (for discharge)
#
# Note: Crossbar has ~60-100 ohm per path. We use parallel
# paths and compensate in ESR calculation.

# this script is not working yet!



CROSSBAR_R = 0.08  # ~80 ohms typical, in kohms for mA calc

def setup_connections():
    """Wire up the measurement circuit"""
    nodes_clear()
    time.sleep(0.1)
    connect(ISENSE_PLUS, 20)   # Battery+ through current sense
    connect(ISENSE_MINUS, GND)
    connect(ISENSE_MINUS, 30)  # Visual connection to GND
    connect(ADC0, 20)          # Voltage measurement
    time.sleep(0.2)

def measure():
    """Return (voltage, current_mA)"""
    v = adc_get(0)
    i = ina_get_current(0) * 1000  # Convert to mA
    return v, i

def measure_esr(dac_ch, v_high, v_low):
    """Measure ESR by pulsing load and measuring dV/dI"""
    oled_print("Measuring ESR...")
    
    # Light load
    dac_set(dac_ch, v_low)
    time.sleep(0.5)
    v1, i1 = measure()
    
    # Heavy load  
    dac_set(dac_ch, v_high)
    time.sleep(0.5)
    v2, i2 = measure()
    
    dac_set(dac_ch, 0)
    
    di = abs(i2 - i1)
    dv = abs(v1 - v2)
    
    if di > 0.1:  # Need meaningful current change
        esr = (dv / di) * 1000  # Convert to mohms
        esr -= CROSSBAR_R * 1000  # Subtract crossbar resistance
        return max(0, esr)
    return -1  # Measurement failed

def get_settings():
    """Prompt user for test parameters"""
    print("\n=== LiPo Cell Characterizer ===\n")
    
    s = {}
    s['name'] = input("Cell ID/name > ") or "unknown"
    s['rated_mah'] = float(input("Rated capacity (mAh) > ") or "100")
    s['cutoff'] = float(input("Cutoff voltage [3.0] > ") or "3.0")
    s['charge_v'] = float(input("Charge voltage [4.2] > ") or "4.2")
    s['charge_c'] = float(input("Charge rate C [0.5] > ") or "0.5")
    s['discharge_c'] = float(input("Discharge rate C [0.2] > ") or "0.2")
    s['cycles'] = int(input("Number of cycles [1] > ") or "1")
    s['interval'] = float(input("Log interval sec [2] > ") or "2")
    
    return s

def log_header(f, s):
    """Write settings and CSV header"""
    f.write("# LiPo Characterization - " + s['name'] + "\n")
    f.write("# Rated: " + str(s['rated_mah']) + "mAh\n")
    f.write("# Cutoff: " + str(s['cutoff']) + "V\n")
    f.write("# Charge: " + str(s['charge_v']) + "V @ " + str(s['charge_c']) + "C\n")
    f.write("# Discharge: " + str(s['discharge_c']) + "C\n")
    f.write("# Crossbar compensation: " + str(CROSSBAR_R*1000) + " mohm\n")
    f.write("#\n")
    f.write("time_s,phase,voltage,current_mA,capacity_mAh\n")

def charge_phase(f, s, start_cap):
    """Charge until current drops or voltage limit reached"""
    oled_print("Charging...")
    print("\n--- CHARGING ---")
    
    target_i = s['rated_mah'] * s['charge_c']  # Target charge current
    
    connect(DAC1, 20)  # Connect DAC to battery
    dac_set(DAC1, s['charge_v'])
    
    capacity = start_cap
    t0 = time.ticks_ms()
    last_t = t0
    
    while True:
        v, i = measure()
        now = time.ticks_ms()
        dt_h = (now - last_t) / 3600000.0  # Hours
        capacity += i * dt_h
        last_t = now
        
        elapsed = (now - t0) / 1000.0
        line = f"{elapsed:.1f},charge,{v:.4f},{i:.2f},{capacity:.3f}\n"
        f.write(line)
        
        print(f"V:{v:.3f} I:{i:.1f}mA Cap:{capacity:.2f}mAh")
        oled_print(f"{v:.2f}V {i:.0f}mA")
        
        # End conditions: current < 10% of target OR voltage at limit
        if (i < target_i * 0.1 and i > 0) or v >= s['charge_v']:
            break
            
        time.sleep(s['interval'])
    
    dac_set(DAC1, 0)
    disconnect(DAC1, 20)
    print("Charge complete!")
    return capacity

def discharge_phase(f, s, start_cap):
    """Discharge through load until cutoff voltage"""
    oled_print("Discharging...")
    print("\n--- DISCHARGING ---")
    
    # Connect load resistor path (row 20 to 40, with 40 to GND)
    connect(40, GND)
    connect(20, 60)  # This enables discharge through load R
    
    capacity = start_cap
    t0 = time.ticks_ms()
    last_t = t0
    
    while True:
        v, i = measure()
        now = time.ticks_ms()
        dt_h = (now - last_t) / 3600000.0
        capacity += abs(i) * dt_h  # Accumulate discharged capacity
        last_t = now
        
        elapsed = (now - t0) / 1000.0
        line = f"{elapsed:.1f},discharge,{v:.4f},{-abs(i):.2f},{capacity:.3f}\n"
        f.write(line)
        
        print(f"V:{v:.3f} I:{-abs(i):.1f}mA Cap:{capacity:.2f}mAh")
        oled_print(f"{v:.2f}V {capacity:.1f}mAh")
        
        if v <= s['cutoff']:
            break
            
        time.sleep(s['interval'])
    
    disconnect(20, 40)
    disconnect(40, GND)
    print("Discharge complete!")
    return capacity

def run_test():
    """Main test sequence"""
    s = get_settings()
    setup_connections()
    
    # Measure initial voltage and ESR
    v0, i0 = measure()
    print(f"\nInitial voltage: {v0:.3f}V")
    oled_print(f"Init: {v0:.2f}V")
    
    # Open log file
    fname = "/lipo_" + s['name'] + ".csv"
    f = jfs.open(fname, 'w+')
    log_header(f, s)
    
    total_charged = 0
    total_discharged = 0
    
    for cycle in range(s['cycles']):
        print(f"\n=== CYCLE {cycle+1}/{s['cycles']} ===")
        oled_print(f"Cycle {cycle+1}")
        
        charged = 4.0
        
        # Discharge
        cap_before = 0
        cap_after = discharge_phase(f, s, 0)
        discharged = cap_after
        total_discharged += discharged
        
        f.write(f"# Cycle {cycle+1}: charged {charged:.2f}mAh, discharged {discharged:.2f}mAh\n")
        print(f"\nCycle {cycle+1} done: +{charged:.1f}/-{discharged:.1f} mAh")
        
        #         # Charge
        # cap_before = 0
        # cap_after = charge_phase(f, s, 0)
        # charged = cap_after
        # total_charged += charged
        
        # time.sleep(2)  # Rest
        
        # # ESR measurement after charge
        # esr = measure_esr(DAC1, 3.5, 4.0)
        # if esr > 0:
        #     f.write(f"# ESR after charge: {esr:.1f} mohm\n")
        #     print(f"ESR: {esr:.1f} mohm")
    
    # Summary
    efficiency = (total_discharged / total_charged * 100) if total_charged > 0 else 0
    summary = f"""
# === SUMMARY ===
# Total charged: {total_charged:.2f} mAh
# Total discharged: {total_discharged:.2f} mAh  
# Efficiency: {efficiency:.1f}%
# Capacity vs rated: {total_discharged/s['cycles']:.1f} / {s['rated_mah']} mAh ({total_discharged/s['cycles']/s['rated_mah']*100:.0f}%)
"""
    f.write(summary)
    print(summary)
    
    f.close()
    print(f"\nData saved to {fname}")
    oled_print("Done!")
    oled_print(f"{total_discharged/s['cycles']:.0f}mAh")

# Run the test
run_test()

