"""
PSRAM Test Script for Jumperless v5

This script tests whether external PSRAM is available and working.
The Jumperless v5 can optionally have 8MB PSRAM installed on GPIO 19 (QSPI CS1).

When PSRAM is available:
- MicroPython heap is extended to include the PSRAM region
- Total heap will be approximately 8MB + 128KB instead of just 128KB

Usage:
    Run this script from the MicroPython REPL:
    >>> exec(open('/scripts/ex/psram_test.py').read())
    
    Or simply:
    >>> import gc
    >>> gc.mem_info()

Author: Jumperless Team
"""

import gc
import time

# Constants for memory calculations
KB = 1024
MB = 1024 * 1024
SRAM_HEAP_SIZE = 128 * KB  # Default SRAM heap (from MICROPY_HEAP_SIZE)
PSRAM_SIZE = 8 * MB  # Expected PSRAM size
PSRAM_THRESHOLD = 1 * MB  # If total heap > 1MB, PSRAM is likely present

def format_size(size_bytes):
    """Format size in human-readable format"""
    if size_bytes >= MB:
        return f"{size_bytes / MB:.2f} MB"
    elif size_bytes >= KB:
        return f"{size_bytes / KB:.1f} KB"
    else:
        return f"{size_bytes} bytes"

def test_psram():
    """
    Test PSRAM availability and run memory diagnostics
    
    Returns:
        dict: Memory information including PSRAM status
    """
    print("=" * 50)
    print("PSRAM Test for Jumperless v5")
    print("=" * 50)
    
    # Force garbage collection to get accurate memory stats
    gc.collect()
    
    # Get memory statistics
    mem_free = gc.mem_free()
    mem_alloc = gc.mem_alloc()
    mem_total = mem_free + mem_alloc
    
    print(f"\n--- Memory Statistics ---")
    print(f"Total Heap:     {format_size(mem_total)}")
    print(f"Free Memory:    {format_size(mem_free)}")
    print(f"Used Memory:    {format_size(mem_alloc)}")
    
    # Check if PSRAM is present based on heap size
    psram_detected = mem_total > PSRAM_THRESHOLD
    
    if psram_detected:
        psram_size_estimate = mem_total - SRAM_HEAP_SIZE
        print(f"\n--- PSRAM Detected! ---")
        print(f"PSRAM Size:     ~{format_size(psram_size_estimate)}")
        print(f"SRAM Heap:      {format_size(SRAM_HEAP_SIZE)}")
        print(f"Combined Heap:  {format_size(mem_total)}")
    else:
        print(f"\n--- No PSRAM Detected ---")
        print(f"Running with SRAM only ({format_size(SRAM_HEAP_SIZE)} heap)")
        print(f"\nTo enable PSRAM:")
        print(f"  1. Install 8MB PSRAM chip on GPIO 19 (QSPI CS1)")
        print(f"  2. Set `[hardware] psram_installed = 1;` in config")
        print(f"  3. Reboot the Jumperless")
        print(f"\nIMPORTANT: Only set psram_installed=1 if PSRAM is physically installed!")
        print(f"           Setting it to 1 without hardware may cause boot issues.")
    
    # Memory allocation test
    print(f"\n--- Memory Allocation Test ---")
    try:
        # Test allocating a large buffer (only if we have enough memory)
        test_size = 100 * KB if psram_detected else 10 * KB
        print(f"Attempting to allocate {format_size(test_size)}...")
        
        start_time = time.ticks_ms()
        test_buffer = bytearray(test_size)
        alloc_time = time.ticks_diff(time.ticks_ms(), start_time)
        
        # Write pattern to test memory integrity
        print(f"Testing memory integrity...")
        for i in range(0, len(test_buffer), 4096):
            test_buffer[i] = i & 0xFF
        
        # Verify pattern
        errors = 0
        for i in range(0, len(test_buffer), 4096):
            if test_buffer[i] != (i & 0xFF):
                errors += 1
        
        del test_buffer
        gc.collect()
        
        if errors == 0:
            print(f"  Allocation: PASS ({alloc_time}ms)")
            print(f"  Integrity:  PASS (0 errors)")
        else:
            print(f"  Allocation: PASS")
            print(f"  Integrity:  FAIL ({errors} errors)")
    except MemoryError as e:
        print(f"  Allocation: FAIL (MemoryError)")
    
    # Print detailed gc info (if available)
    print(f"\n--- Detailed GC Info ---")
    if hasattr(gc, 'mem_info'):
        gc.mem_info(1)  # Verbose output
    else:
        print(f"  gc.mem_info() not available in this build")
        print(f"  Total:  {format_size(mem_total)}")
        print(f"  Free:   {format_size(gc.mem_free())}")
        print(f"  Used:   {format_size(gc.mem_alloc())}")
    
    print(f"\n" + "=" * 50)
    if psram_detected:
        print("PSRAM TEST PASSED - Extended memory available!")
    else:
        print("PSRAM NOT DETECTED - Using SRAM only")
    print("=" * 50)
    
    return {
        'psram_detected': psram_detected,
        'total_heap': mem_total,
        'free_memory': mem_free,
        'used_memory': mem_alloc
    }

def memory_stress_test(iterations=5, chunk_size=None):
    """
    Run a memory stress test with repeated allocations
    
    Args:
        iterations: Number of allocation/deallocation cycles
        chunk_size: Size of each allocation (auto-detected if None)
    """
    gc.collect()
    mem_total = gc.mem_free() + gc.mem_alloc()
    
    if chunk_size is None:
        # Use 1/4 of available memory per chunk
        chunk_size = gc.mem_free() // 4
    
    print(f"\n--- Memory Stress Test ---")
    print(f"Iterations: {iterations}")
    print(f"Chunk size: {format_size(chunk_size)}")
    
    errors = 0
    for i in range(iterations):
        try:
            # Allocate
            chunks = []
            while gc.mem_free() > chunk_size + 10000:
                chunks.append(bytearray(chunk_size))
            
            allocated = len(chunks) * chunk_size
            print(f"  Iter {i+1}: Allocated {format_size(allocated)} in {len(chunks)} chunks")
            
            # Deallocate
            del chunks
            gc.collect()
            
        except MemoryError:
            errors += 1
            print(f"  Iter {i+1}: MemoryError")
            gc.collect()
    
    print(f"\nStress test complete: {iterations - errors}/{iterations} passed")

# Run tests when script is executed
if __name__ == "__main__" or True:  # Always run when exec'd
    result = test_psram()
