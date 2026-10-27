"""
Async Read
Demo of using select.poll() to check for input on stdin without blocking.
"""
import sys
import select
import time

# Create a poll object
poll_obj = select.poll()

# Register sys.stdin for polling on input events
poll_obj.register(sys.stdin, select.POLLIN)

print("Polling stdin, Ctrl+C to exit:")

while True:
    # Poll with 0 timeout is nonblocking
    events = poll_obj.poll(0) 

    if events:
        # Data is available. The 'events' list contains tuples of (object, event_mask)
        for obj, event in events:
            
            print("ack") # it occasionally drops the first chars if it doesn't poll at the right time, so maybe send some sort of request to send and wait for an ack?
            
            while obj is sys.stdin and event & select.POLLIN:

                char = sys.stdin.read(1) 
                
                if char:
                    print(f"{char}", end='')
                else:
                 
                    print("End of input stream (EOF detected). Exiting.")
                    break
                if char == '\n':
                    break

    sleep_start = time.ticks_ms()
    
    while time.ticks_ms() - sleep_start < 1000:
        events = poll_obj.poll(0) 
        if events:
            break
            
        # print("loop")
