(V-bit groove test: rows = depths 0.05 0.08 0.11 0.15 mm, columns = feeds 100 200 300 mm/min)
G21 G90 G94
G0 Z2.0
M3 S10000
G4 P3
(depth 0.05 mm, feed 100 mm/min)
G0 X1.500 Y1.500
G1 Z-0.050 F30
G1 X6.500 F100
G0 Z2.0
(depth 0.05 mm, feed 200 mm/min)
G0 X8.500 Y1.500
G1 Z-0.050 F30
G1 X13.500 F200
G0 Z2.0
(depth 0.05 mm, feed 300 mm/min)
G0 X15.500 Y1.500
G1 Z-0.050 F30
G1 X20.500 F300
G0 Z2.0
(depth 0.08 mm, feed 100 mm/min)
G0 X1.500 Y5.000
G1 Z-0.080 F30
G1 X6.500 F100
G0 Z2.0
(depth 0.08 mm, feed 200 mm/min)
G0 X8.500 Y5.000
G1 Z-0.080 F30
G1 X13.500 F200
G0 Z2.0
(depth 0.08 mm, feed 300 mm/min)
G0 X15.500 Y5.000
G1 Z-0.080 F30
G1 X20.500 F300
G0 Z2.0
(depth 0.11 mm, feed 100 mm/min)
G0 X1.500 Y8.500
G1 Z-0.110 F30
G1 X6.500 F100
G0 Z2.0
(depth 0.11 mm, feed 200 mm/min)
G0 X8.500 Y8.500
G1 Z-0.110 F30
G1 X13.500 F200
G0 Z2.0
(depth 0.11 mm, feed 300 mm/min)
G0 X15.500 Y8.500
G1 Z-0.110 F30
G1 X20.500 F300
G0 Z2.0
(depth 0.15 mm, feed 100 mm/min)
G0 X1.500 Y12.000
G1 Z-0.150 F30
G1 X6.500 F100
G0 Z2.0
(depth 0.15 mm, feed 200 mm/min)
G0 X8.500 Y12.000
G1 Z-0.150 F30
G1 X13.500 F200
G0 Z2.0
(depth 0.15 mm, feed 300 mm/min)
G0 X15.500 Y12.000
G1 Z-0.150 F30
G1 X20.500 F300
G0 Z2.0
M5
G0 Z5
G0 X0 Y0
