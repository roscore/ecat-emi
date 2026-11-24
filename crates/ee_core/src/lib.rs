//! ecat-emi core: lumped, quasi-static EtherCAT EMI coupling model for PCBs (pure Rust).
//! Not a full-wave field solver — for design intuition and comparing options.
pub mod board;
pub mod cable;
pub mod couple;
pub mod grid;
pub mod kicad;
pub mod link;
pub mod num;
pub mod raster;
pub mod roles;
pub mod sample;
pub mod sexpr;
pub mod spectrum;
pub mod gerber;
pub mod report;
pub mod settings;
pub mod zipread;

use sexpr::ParseError;

/// A loaded design: KiCad boards carry nets; Gerber sets get island nets after `gerber::build`.
pub enum Input {
    Kicad(board::Board),
    Gerber(gerber::Files),
}

/// Open user files (any mix of .kicad_pcb, Gerber, Excellon, .zip). Everything stays in memory.
pub fn open(files: Vec<(String, Vec<u8>)>) -> Result<Input, ParseError> {
    let mut flat = Vec::new();
    for (name, bytes) in files {
        if zipread::is_zip(&bytes) {
            flat.extend(zipread::read(&bytes)?);
        } else {
            flat.push((name, bytes));
        }
    }
    if let Some((_, b)) = flat.iter().find(|(n, _)| n.to_ascii_lowercase().ends_with(".kicad_pcb")) {
        return kicad::parse(&String::from_utf8_lossy(b)).map(Input::Kicad);
    }
    let mut set = gerber::Files { layers: vec![], drills: vec![] };
    let mut errors = Vec::new();
    for (name, bytes) in &flat {
        let text = String::from_utf8_lossy(bytes);
        let g = gerber::guess(name, None);
        if g == gerber::LayerGuess::Drill || text.trim_start().starts_with("M48") {
            match gerber::parse_drill(&text) {
                Ok(d) => set.drills.extend(d),
                Err(e) => errors.push(format!("{name}: {e}")),
            }
            continue;
        }
        match gerber::parse(&text) {
            Ok(l) => {
                let g = gerber::guess(name, l.function.as_deref());
                set.layers.push((name.clone(), g, l));
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if set.layers.iter().all(|l| !matches!(l.1, gerber::LayerGuess::Copper(_))) {
        let msg = if errors.is_empty() { "no copper layer found".to_string() } else { errors.join("; ") };
        return Err(ParseError { line: 0, msg });
    }
    Ok(Input::Gerber(set))
}
