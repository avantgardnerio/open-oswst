#!/usr/bin/env python3
"""Split a pcb2gcode job into one file per tool, without the tool changes.

    split_tools.py out/drill.ngc   ->   out/drill_1.1mm.ngc, out/drill_1.7mm.ngc
    split_tools.py out/back.ngc    ->   out/back_1.ngc (one tool: same job)

pcb2gcode stops for a tool change (T, M6, M0) at the start of every tool,
even the first. On the 3018 every bit change also needs a new zero (each bit
sticks out of the collet by a different length), so each tool is its own
job instead: fit the bit, cnc.py zero, cnc.py run. cnc.py refuses M0/M6, so
they come out here. The file's opening lines (units, modes) start every part;
the spindle stops at the end of every part.
"""

import re
import sys
from pathlib import Path

DROPPED = re.compile(r"^(T\d+|M0?6|M0?0|M0?1)\b|^\(MSG", re.IGNORECASE)


def main():
    source = Path(sys.argv[1])
    lines = source.read_text().splitlines()
    starts = [index for index, line in enumerate(lines) if re.match(r"^T\d+\s*$", line.strip())]
    if not starts:
        sys.exit(f"{source}: no tool (T) lines")
    opening = lines[:starts[0]]
    for part, start in enumerate(starts):
        stop = starts[part + 1] if part + 1 < len(starts) else len(lines)
        block = lines[start:stop]
        # The size, from pcb2gcode's message: "(MSG, Change tool bit to drill size 1.1mm)"
        size = next((match.group(1) for line in block
                     for match in [re.search(r"size ([\d.]+mm)", line)] if match), f"{part + 1}")
        kept = [line for line in block if not DROPPED.match(line.strip())]
        if part + 1 < len(starts):
            kept += ["M5", "G0 Z15"]  # pcb2gcode only stops the spindle at the very end
        target = source.with_name(f"{source.stem}_{size}.ngc")
        target.write_text("\n".join(opening + kept) + "\n")
        print(f"{target}: {len(kept)} lines")


if __name__ == "__main__":
    main()
