//! Bundled synthetic sample board (generic, written from scratch — not a real product).
//! 4 layers, 80 × 60 mm: 48 V half-bridge on the left, EtherCAT PHY + magnetics + RJ45 on the
//! right. The RX pair is routed past the switch node at 1.5 mm, over a slot in the GND plane.
use std::fmt::Write;

pub const NETS: [&str; 16] = [
    "GND", "VBUS", "SW", "GATE_H", "GATE_L", "BOOT", "+3V3", "ECAT_TX_P", "ECAT_TX_N", "ECAT_RX_P", "ECAT_RX_N", "MDI_TX_P",
    "MDI_TX_N", "MDI_RX_P", "MDI_RX_N", "VDRV",
];

/// The GND-plane slot (x0, y0, x1, y1) on In1.Cu, under the RX pair.
pub const SLOT: [f64; 4] = [28.0, 25.6, 46.0, 28.4];

struct W {
    s: String,
}

fn net(name: &str) -> usize {
    NETS.iter().position(|n| *n == name).map(|k| k + 1).unwrap_or(0)
}

impl W {
    fn track(&mut self, pts: &[[f64; 2]], w: f64, layer: &str, n: &str) {
        for p in pts.windows(2) {
            let _ = writeln!(
                self.s,
                "  (segment (start {} {}) (end {} {}) (width {w}) (layer \"{layer}\") (net {}))",
                p[0][0],
                p[0][1],
                p[1][0],
                p[1][1],
                net(n)
            );
        }
    }
    fn via(&mut self, at: [f64; 2], n: &str) {
        let _ = writeln!(self.s, "  (via (at {} {}) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net {}))", at[0], at[1], net(n));
    }
    /// Footprint with SMD (or THT when `tht`) pads given in absolute coordinates.
    fn footprint(&mut self, lib: &str, reference: &str, at: [f64; 2], pads: &[(&str, [f64; 2], [f64; 2], &str)], tht: bool) {
        let _ = writeln!(self.s, "  (footprint \"{lib}\" (layer \"F.Cu\") (at {} {})", at[0], at[1]);
        let _ = writeln!(self.s, "    (property \"Reference\" \"{reference}\" (at 0 -2) (layer \"F.SilkS\"))");
        for (num, p, size, n) in pads {
            let (kind, shape, layers) =
                if tht { ("thru_hole", "circle", "\"*.Cu\" \"*.Mask\"") } else { ("smd", "rect", "\"F.Cu\" \"F.Paste\" \"F.Mask\"") };
            let drill = if tht { " (drill 1.0)" } else { "" };
            let _ = writeln!(
                self.s,
                "    (pad \"{num}\" {kind} {shape} (at {} {}) (size {} {}){drill} (layers {layers}) (net {} \"{n}\"))",
                ((p[0] - at[0]) * 1e4).round() / 1e4,
                ((p[1] - at[1]) * 1e4).round() / 1e4,
                size[0],
                size[1],
                net(n)
            );
        }
        self.s.push_str("  )\n");
    }
    fn zone(&mut self, n: &str, layer: &str, pts: &[[f64; 2]]) {
        let _ = writeln!(self.s, "  (zone (net {}) (net_name \"{n}\") (layer \"{layer}\")", net(n));
        let _ = write!(self.s, "    (filled_polygon (layer \"{layer}\") (pts");
        for p in pts {
            let _ = write!(self.s, " (xy {} {})", p[0], p[1]);
        }
        self.s.push_str("))\n  )\n");
    }
}

/// The sample as `.kicad_pcb` text.
pub fn kicad_text() -> String {
    let mut w = W { s: String::new() };
    w.s.push_str("(kicad_pcb (version 20221018) (generator ecat_emi_sample)\n");
    w.s.push_str("  (general (thickness 1.6))\n");
    w.s.push_str("  (layers (0 \"F.Cu\" signal) (1 \"In1.Cu\" power) (2 \"In2.Cu\" power) (31 \"B.Cu\" signal) (44 \"Edge.Cuts\" user))\n");
    w.s.push_str("  (setup (stackup\n");
    for (name, kind, t, e) in [
        ("F.Cu", "copper", 0.035, 0.0),
        ("dielectric 1", "prepreg", 0.2, 4.3),
        ("In1.Cu", "copper", 0.035, 0.0),
        ("dielectric 2", "core", 1.065, 4.5),
        ("In2.Cu", "copper", 0.035, 0.0),
        ("dielectric 3", "prepreg", 0.2, 4.3),
        ("B.Cu", "copper", 0.035, 0.0),
    ] {
        let eps = if e > 0.0 { format!(" (epsilon_r {e})") } else { String::new() };
        let _ = writeln!(w.s, "    (layer \"{name}\" (type \"{kind}\") (thickness {t}){eps})");
    }
    w.s.push_str("  ))\n");
    w.s.push_str("  (net 0 \"\")\n");
    for (k, n) in NETS.iter().enumerate() {
        let _ = writeln!(w.s, "  (net {} \"{n}\")", k + 1);
    }

    // Power stage.
    w.footprint("Conn:Power_2pin", "J3", [4.0, 10.0], &[("1", [4.0, 8.0], [2.0, 2.0], "VBUS"), ("2", [4.0, 12.0], [2.0, 2.0], "GND")], true);
    for (r, x) in [("C1", 6.0), ("C2", 9.0)] {
        w.footprint("Cap:C_1206", r, [x, 24.0], &[("1", [x, 22.5], [1.6, 1.0], "VBUS"), ("2", [x, 25.5], [1.6, 1.0], "GND")], false);
    }
    w.footprint(
        "FET:PowerPAK_5x6",
        "Q1",
        [14.0, 18.0],
        &[("D", [14.0, 16.0], [4.0, 2.0], "VBUS"), ("S", [14.0, 20.0], [4.0, 1.5], "SW"), ("G", [11.5, 19.0], [0.8, 0.8], "GATE_H")],
        false,
    );
    w.footprint(
        "FET:PowerPAK_5x6",
        "Q2",
        [14.0, 30.0],
        &[("D", [14.0, 28.0], [4.0, 1.5], "SW"), ("S", [14.0, 32.0], [4.0, 2.0], "GND"), ("G", [11.5, 31.0], [0.8, 0.8], "GATE_L")],
        false,
    );
    w.footprint(
        "IC:Driver_SO8",
        "U2",
        [4.0, 42.0],
        &[
            ("1", [5.5, 40.0], [1.0, 0.5], "GATE_H"),
            ("2", [5.5, 41.0], [1.0, 0.5], "GATE_L"),
            ("3", [5.5, 42.0], [1.0, 0.5], "BOOT"),
            ("4", [5.5, 43.0], [1.0, 0.5], "VDRV"),
            ("5", [2.5, 42.0], [1.0, 0.5], "GND"),
        ],
        false,
    );
    w.footprint("Cap:C_0603", "C3", [18.0, 12.0], &[("1", [17.2, 12.0], [0.8, 0.8], "BOOT"), ("2", [18.8, 12.0], [0.8, 0.8], "SW")], false);
    w.footprint("Conn:Phase_2pin", "J1", [50.0, 6.0], &[("1", [48.0, 6.0], [2.2, 2.2], "SW"), ("2", [52.0, 6.0], [2.2, 2.2], "SW")], true);

    // EtherCAT side.
    w.footprint(
        "IC:PHY_QFN32",
        "U1",
        [22.0, 38.0],
        &[
            ("1", [25.5, 36.0], [0.8, 0.3], "ECAT_RX_P"),
            ("2", [25.5, 36.5], [0.8, 0.3], "ECAT_RX_N"),
            ("3", [25.5, 38.0], [0.8, 0.3], "ECAT_TX_P"),
            ("4", [25.5, 38.5], [0.8, 0.3], "ECAT_TX_N"),
            ("5", [18.5, 37.0], [0.8, 0.3], "+3V3"),
            ("6", [18.5, 39.0], [0.8, 0.3], "GND"),
        ],
        false,
    );
    w.footprint(
        "Magnetics:LAN_Transformer",
        "T1",
        [62.0, 37.5],
        &[
            ("1", [60.0, 35.0], [1.0, 0.6], "ECAT_RX_P"),
            ("2", [60.0, 36.0], [1.0, 0.6], "ECAT_RX_N"),
            ("3", [60.0, 39.0], [1.0, 0.6], "ECAT_TX_P"),
            ("4", [60.0, 40.0], [1.0, 0.6], "ECAT_TX_N"),
            ("5", [64.0, 35.0], [1.0, 0.6], "MDI_RX_P"),
            ("6", [64.0, 36.0], [1.0, 0.6], "MDI_RX_N"),
            ("7", [64.0, 39.0], [1.0, 0.6], "MDI_TX_P"),
            ("8", [64.0, 40.0], [1.0, 0.6], "MDI_TX_N"),
        ],
        false,
    );
    w.footprint(
        "Conn:RJ45_8P8C",
        "J2",
        [75.0, 37.5],
        &[
            ("1", [72.0, 39.0], [1.2, 1.2], "MDI_TX_P"),
            ("2", [72.0, 40.0], [1.2, 1.2], "MDI_TX_N"),
            ("3", [72.0, 35.0], [1.2, 1.2], "MDI_RX_P"),
            ("6", [72.0, 36.0], [1.2, 1.2], "MDI_RX_N"),
            ("SH", [77.0, 32.0], [2.0, 2.0], "GND"),
        ],
        true,
    );

    // Power tracks.
    w.track(&[[4.0, 8.0], [6.0, 8.0], [6.0, 22.5], [9.0, 22.5], [9.0, 16.0], [14.0, 16.0]], 1.5, "F.Cu", "VBUS");
    w.track(&[[6.0, 25.5], [9.0, 25.5], [9.0, 32.0], [14.0, 32.0]], 1.5, "F.Cu", "GND");
    w.track(&[[14.0, 20.0], [14.0, 24.0], [14.0, 28.0]], 2.0, "F.Cu", "SW");
    w.track(&[[14.0, 24.0], [50.0, 24.0], [50.0, 6.0]], 2.0, "F.Cu", "SW");
    w.track(&[[18.8, 12.0], [18.8, 20.0]], 0.4, "F.Cu", "SW");
    // Gate drive on the bottom layer.
    w.track(&[[6.5, 40.0], [10.5, 40.0], [10.5, 19.0]], 0.3, "B.Cu", "GATE_H");
    w.track(&[[6.5, 41.0], [11.5, 41.0], [11.5, 31.0]], 0.3, "B.Cu", "GATE_L");
    w.track(&[[6.5, 42.0], [17.2, 42.0], [17.2, 12.0]], 0.3, "B.Cu", "BOOT");
    for (a, n) in [([6.5, 40.0], "GATE_H"), ([10.5, 19.0], "GATE_H"), ([6.5, 41.0], "GATE_L"), ([11.5, 31.0], "GATE_L"), ([6.5, 42.0], "BOOT"), ([17.2, 12.0], "BOOT")] {
        w.via(a, n);
    }

    // TX pair: straight to the magnetics, far from the bridge.
    w.track(&[[25.5, 38.0], [57.0, 38.0], [58.0, 39.0], [60.0, 39.0]], 0.2, "F.Cu", "ECAT_TX_P");
    w.track(&[[25.5, 38.5], [56.8, 38.5], [58.3, 40.0], [60.0, 40.0]], 0.2, "F.Cu", "ECAT_TX_N");
    // RX pair: detours up and runs 1.5 mm (edge to edge) from the 2 mm switch-node trace.
    w.track(&[[25.5, 36.0], [27.0, 36.0], [27.0, 26.6], [54.0, 26.6], [54.0, 35.0], [60.0, 35.0]], 0.2, "F.Cu", "ECAT_RX_P");
    w.track(&[[25.5, 36.5], [27.5, 36.5], [27.5, 27.1], [53.5, 27.1], [53.5, 36.0], [60.0, 36.0]], 0.2, "F.Cu", "ECAT_RX_N");
    // Line side.
    w.track(&[[64.0, 39.0], [72.0, 39.0]], 0.2, "F.Cu", "MDI_TX_P");
    w.track(&[[64.0, 40.0], [72.0, 40.0]], 0.2, "F.Cu", "MDI_TX_N");
    w.track(&[[64.0, 35.0], [72.0, 35.0]], 0.2, "F.Cu", "MDI_RX_P");
    w.track(&[[64.0, 36.0], [72.0, 36.0]], 0.2, "F.Cu", "MDI_RX_N");

    // GND stitching vias.
    for p in [[7.5, 27.0], [12.0, 34.0], [20.0, 42.0], [2.5, 44.0], [30.0, 44.0], [58.0, 30.0], [70.0, 44.0], [77.0, 30.0]] {
        w.via(p, "GND");
    }

    // Planes: GND on In1 with the slot (hole bridged into the outline), +3V3 on In2.
    let [sx0, sy0, sx1, sy1] = SLOT;
    w.zone(
        "GND",
        "In1.Cu",
        &[
            [sx0, 0.5],
            [79.5, 0.5],
            [79.5, 59.5],
            [0.5, 59.5],
            [0.5, 0.5],
            [sx0, 0.5],
            [sx0, sy0],
            [sx0, sy1],
            [sx1, sy1],
            [sx1, sy0],
            [sx0, sy0],
        ],
    );
    w.zone("+3V3", "In2.Cu", &[[0.5, 0.5], [79.5, 0.5], [79.5, 59.5], [0.5, 59.5]]);
    w.s.push_str("  (gr_rect (start 0 0) (end 80 60) (layer \"Edge.Cuts\") (width 0.1))\n)\n");
    w.s
}
