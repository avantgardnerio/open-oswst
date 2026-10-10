"""Minimal KiCad PCB file generator."""


class Board:
    def __init__(self, paper="USLetter", thickness=1.6):
        self.paper = paper
        self.thickness = thickness
        self.layers = [
            (0, "F.Cu", "signal", None),
            (31, "B.Cu", "signal", None),
            (36, "B.SilkS", "user", "B.Silkscreen"),
            (37, "F.SilkS", "user", "F.Silkscreen"),
            (38, "B.Mask", "user", "B.Mask"),
            (39, "F.Mask", "user", "F.Mask"),
            (44, "Edge.Cuts", "user", None),
        ]
        self.graphics = []
        self.footprints = []
        self.zones = []
        self.vias = []
        self._fp_id = 0
        self._nets = {"": 0}  # net name → net number (0 = unconnected)

    def _net(self, name):
        """Get or create a net number for the given net name."""
        if name not in self._nets:
            self._nets[name] = len(self._nets)
        return self._nets[name]

    def _pad_thru(self, name, x, y, size, drill, net=None):
        """Generate a through-hole circle pad string."""
        net_str = ""
        if net:
            n = self._net(net)
            net_str = f"\n      (net {n} \"{net}\")"
        return (
            f'    (pad "{name}" thru_hole circle\n'
            f"      (at {x} {y})\n"
            f"      (size {size} {size})\n"
            f"      (drill {drill})\n"
            f"      (layers *.Cu *.Mask){net_str}\n"
            f"    )"
        )

    def add_text(self, x, y, text, layer="F.SilkS", size=1, thickness=0.15, angle=0):
        """Add a text string at (x, y) on the given layer."""
        at_str = f"(at {x} {y})" if angle == 0 else f"(at {x} {y} {angle})"
        self.graphics.append(
            f'  (gr_text "{text}"\n'
            f"    {at_str}\n"
            f'    (layer "{layer}")\n'
            f"    (effects (font (size {size} {size}) (thickness {thickness})))\n"
            f"  )"
        )

    def add_rect(self, x, y, w, h, layer="Edge.Cuts", width=0.1):
        self.graphics.append(
            f'  (gr_rect (start {x} {y}) (end {x+w} {y+h})'
            f' (stroke (width {width}) (type solid)) (fill none)'
            f' (layer "{layer}"))'
        )

    def add_mounting_hole(self, x, y, drill=3.2):
        """Add a non-plated through-hole at (x, y) with given drill diameter in mm."""
        self._fp_id += 1
        pad_size = drill + 0.5
        self.footprints.append(
            f'  (footprint "MountingHole_{drill}mm"\n'
            f'    (layer "F.Cu")\n'
            f"    (at {x} {y})\n"
            f'    (attr exclude_from_pos_files exclude_from_bom)\n'
            f'    (fp_text reference "H{self._fp_id}" (at 0 0) (layer "F.SilkS") hide\n'
            f"      (effects (font (size 1 1) (thickness 0.15)))\n"
            f"    )\n"
            f'    (pad "" np_thru_hole circle\n'
            f"      (at 0 0)\n"
            f"      (size {pad_size} {pad_size})\n"
            f"      (drill {drill})\n"
            f"      (layers *.Cu *.Mask)\n"
            f"    )\n"
            f"  )"
        )

    def _add_connector(self, footprint_name, x, y, pins, pitch, pad_size, drill,
                        label="", angle=0, pad_labels=None, pad_nets=None):
        """Generic through-hole connector footprint."""
        self._fp_id += 1
        half_span = pitch * (pins - 1) / 2

        pad_lines = []
        for i in range(pins):
            px = -half_span + i * pitch
            pad_name = pad_labels[i] if pad_labels and i < len(pad_labels) else str(i + 1)
            net = pad_nets[i] if pad_nets and i < len(pad_nets) else None
            pad_lines.append(self._pad_thru(pad_name, px, 0, pad_size, drill, net))

        pads = "\n".join(pad_lines)
        ref_text = label or f"J{self._fp_id}"
        at_str = f"(at {x} {y})" if angle == 0 else f"(at {x} {y} {angle})"
        self.footprints.append(
            f'  (footprint "{footprint_name}"\n'
            f'    (layer "F.Cu")\n'
            f"    {at_str}\n"
            f'    (fp_text reference "{ref_text}" (at 0 -2.5) (layer "F.SilkS")\n'
            f"      (effects (font (size 1 1) (thickness 0.15)))\n"
            f"    )\n"
            f"{pads}\n"
            f"  )"
        )

    def add_jst_ph(self, x, y, pins=2, label="", angle=0, pad_labels=None, pad_nets=None):
        """Add a JST-PH through-hole header (2.0mm pitch) at (x, y)."""
        self._add_connector(f"JST_PH_B{pins}B", x, y, pins, 2.0, 1.5, 0.8,
                            label, angle, pad_labels, pad_nets)


    def add_header(self, x, y, pins, label="", angle=0, pad_labels=None, pad_nets=None, pitch=2.54):
        """Add a pin header (default 2.54mm pitch) at (x, y)."""
        self._add_connector(f"PinHeader_1x{pins}_P{pitch}mm", x, y, pins, pitch, 1.7, 1.0,
                            label, angle, pad_labels, pad_nets)

    def add_via(self, x, y, net, size=0.6, drill=0.3):
        """Add a through via (F.Cu to B.Cu) at (x, y) on `net`: a stitching via
        ties the two layers' copper fill together there."""
        n = self._net(net)
        self.vias.append(
            f'  (via (at {x} {y}) (size {size}) (drill {drill})'
            f' (layers "F.Cu" "B.Cu") (net {n}))'
        )

    def add_zone(self, net, points, layers=("F.Cu", "B.Cu"), clearance=0.5,
                 min_thickness=0.25, thermal_gap=0.5, thermal_bridge_width=0.5,
                 island_removal_mode=2, min_island_area=10):
        """Add a copper zone (e.g. ground plane). Points are (x,y) tuples defining the outline.
        island_removal_mode: 0=always remove, 1=never remove, 2=remove below min_island_area (mm²)."""
        n = self._net(net)
        layer_str = " ".join(f'"{l}"' for l in layers)
        pts = " ".join(f"(xy {x} {y})" for x, y in points)
        self.zones.append(
            f'  (zone\n'
            f'    (net {n})\n'
            f'    (net_name "{net}")\n'
            f'    (layers {layer_str})\n'
            f'    (hatch edge 0.5)\n'
            f'    (connect_pads\n'
            f'      (clearance {clearance})\n'
            f'    )\n'
            f'    (min_thickness {min_thickness})\n'
            f'    (filled_areas_thickness no)\n'
            f'    (fill yes\n'
            f'      (thermal_gap {thermal_gap})\n'
            f'      (thermal_bridge_width {thermal_bridge_width})\n'
            f'      (island_removal_mode {island_removal_mode})\n'
            f'      (island_area_min {min_island_area})\n'
            f'    )\n'
            f'    (polygon\n'
            f'      (pts\n'
            f'        {pts}\n'
            f'      )\n'
            f'    )\n'
            f'  )'
        )

    def _render_layers(self):
        lines = []
        for num, name, kind, alias in self.layers:
            if alias:
                lines.append(f'    ({num} "{name}" {kind} "{alias}")')
            else:
                lines.append(f'    ({num} "{name}" {kind})')
        return "\n".join(lines)

    def _render_nets(self):
        lines = []
        for name, num in sorted(self._nets.items(), key=lambda x: x[1]):
            lines.append(f'  (net {num} "{name}")')
        return "\n".join(lines)

    def render(self):
        parts = []
        if self.graphics:
            parts.append("\n".join(self.graphics))
        if self.footprints:
            parts.append("\n".join(self.footprints))
        if self.vias:
            parts.append("\n".join(self.vias))
        if self.zones:
            parts.append("\n".join(self.zones))
        gfx = "\n\n" + "\n\n".join(parts) if parts else ""
        return (
            f"(kicad_pcb\n"
            f'  (version 20240108)\n'
            f'  (generator "kicad.py")\n'
            f'  (generator_version "8.0")\n'
            f"  (general\n"
            f"    (thickness {self.thickness})\n"
            f"    (legacy_teardrops no)\n"
            f"  )\n"
            f'  (paper "{self.paper}")\n'
            f"  (layers\n"
            f"{self._render_layers()}\n"
            f"  )\n"
            f"  (setup\n"
            f"    (pad_to_mask_clearance 0)\n"
            f"    (pcbplotparams\n"
            f"      (layerselection 0x00010fc_ffffffff)\n"
            f"    )\n"
            f"  )\n"
            f"{self._render_nets()}\n"
            f'  (net_class "Default" ""\n'
            f"    (clearance 0.2)\n"
            f"    (trace_width 0.25)\n"
            f"    (via_dia 0.6)\n"
            f"    (via_drill 0.3)\n"
            f"  )\n"
            f'  (net_class "Power" "Battery rail — 800mA peak during TX"\n'
            f"    (clearance 0.2)\n"
            f"    (trace_width 0.5)\n"
            f"    (via_dia 0.8)\n"
            f"    (via_drill 0.4)\n"
            f'    (add_net "VBAT")\n'
            f'    (add_net "VSW")\n'
            f"  )"
            f"{gfx}\n"
            f")\n"
        )

    def save(self, path):
        with open(path, "w") as f:
            f.write(self.render())
        print(f"Wrote {path}")
