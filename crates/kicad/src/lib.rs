use eda_model::{EdaComponent, ElectricalType, Graphic, Package, Symbol};
use std::{collections::BTreeSet, fmt::Write};
pub const KICAD_VERSION: &str = "20231120";
#[derive(Debug, Default, Clone)]
pub struct Validation {
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}
pub fn name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}
pub fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
pub fn mm(n: i64) -> String {
    format!("{:.6}", n as f64 / 1_000_000.0)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}
pub fn angle(m: i32) -> String {
    format!("{:.3}", m as f64 / 1000.0)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}
pub fn layer(src: &str) -> Option<&'static str> {
    match src.to_ascii_uppercase().as_str() {
        "TOP_SILKSCREEN" => Some("F.SilkS"),
        "TOP_ASSEMBLY" => Some("F.Fab"),
        "TOP_COPPER" | "TOP" => Some("F.Cu"),
        "TOP_SOLDER_MASK" => Some("F.Mask"),
        "TOP_PASTE" => Some("F.Paste"),
        "BOTTOM_SILKSCREEN" => Some("B.SilkS"),
        "BOTTOM_ASSEMBLY" => Some("B.Fab"),
        "BOTTOM_COPPER" | "BOTTOM" => Some("B.Cu"),
        _ => None,
    }
}
pub fn validate(c: &EdaComponent) -> Validation {
    let mut v = Validation::default();
    for s in &c.symbols {
        for u in &s.units {
            let mut seen = BTreeSet::new();
            for p in &u.pins {
                if p.number.trim().is_empty() {
                    v.errors.push(format!("{}: blank symbol pin", s.name))
                }
                if !seen.insert(&p.number) {
                    v.errors
                        .push(format!("{}: duplicate pin {}", s.name, p.number))
                }
                if p.electrical_type == ElectricalType::Unknown {
                    v.warnings.push(format!(
                        "{} pin {}: unspecified electrical type",
                        s.name, p.number
                    ))
                }
            }
        }
    }
    for p in &c.packages {
        let mut seen = BTreeSet::new();
        for x in &p.pads {
            if x.number.trim().is_empty() {
                v.errors.push(format!("{}: blank pad", p.name))
            }
            if !seen.insert(&x.number) {
                v.errors
                    .push(format!("{}: duplicate pad {}", p.name, x.number))
            }
            for l in &x.layers {
                if layer(l).is_none() {
                    v.warnings
                        .push(format!("{} pad {}: unknown layer {}", p.name, x.number, l))
                }
            }
        }
    }
    v
}
pub fn symbol_lib(c: &EdaComponent, nickname: &str) -> String {
    let mut o =
        format!("(kicad_symbol_lib (version {KICAD_VERSION}) (generator \"kicad_symbol_editor\") (generator_version \"10.0\")\n");
    for s in &c.symbols {
        o.push_str(&symbol(c, s, nickname, &std::collections::BTreeMap::new()));
    }
    o.push_str(")\n");
    o
}
pub fn symbol_lib_many(cs: &[EdaComponent], nickname: &str) -> String {
    symbol_lib_many_with_footprints(cs, nickname, &std::collections::BTreeMap::new())
}
pub fn symbol_lib_many_with_footprints(
    cs: &[EdaComponent],
    nickname: &str,
    footprints: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut o =
        format!("(kicad_symbol_lib (version {KICAD_VERSION}) (generator \"kicad_symbol_editor\") (generator_version \"10.0\")\n");
    let mut used = BTreeSet::new();
    for c in cs {
        for s in &c.symbols {
            let mut copy = s.clone();
            if !used.insert(name(&copy.name)) {
                copy.name = format!("{}_{}", copy.name, name(&c.mpn));
            }
            o.push_str(&symbol(c, &copy, nickname, footprints));
        }
    }
    o.push_str(")\n");
    o
}
pub fn sexpr_balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut quoted = false;
    let mut escaped = false;
    for c in s.chars() {
        if quoted {
            if escaped {
                escaped = false
            } else if c == '\\' {
                escaped = true
            } else if c == '"' {
                quoted = false
            }
        } else if c == '"' {
            quoted = true
        } else if c == '(' {
            depth += 1
        } else if c == ')' {
            depth -= 1;
            if depth < 0 {
                return false;
            }
        }
    }
    depth == 0 && !quoted
}
fn symbol(
    c: &EdaComponent,
    s: &Symbol,
    nickname: &str,
    footprints: &std::collections::BTreeMap<String, String>,
) -> String {
    let sn = name(&s.name);
    let mut o = format!("  (symbol \"{sn}\"\n    (pin_names (offset 1.016))\n    (exclude_from_sim no)\n    (in_bom yes)\n    (on_board yes)\n");
    let fp = c
        .packages
        .first()
        .map(|p| {
            footprints
                .get(&format!("{}|{}|{}", c.manufacturer, c.mpn, p.name))
                .cloned()
                .unwrap_or_else(|| name(&p.name))
        })
        .unwrap_or_default();
    let props = [
        ("Reference", "U"),
        ("Value", &c.mpn),
        ("Footprint", &format!("{nickname}:{fp}")),
        ("Datasheet", ""),
        ("Manufacturer", &c.manufacturer),
        ("MPN", &c.mpn),
        ("SourceSHA256", c.source_sha256.as_deref().unwrap_or("")),
    ];
    for (k, v) in props {
        o.push_str(&format!(
            "    (property \"{}\" \"{}\" (at 0 0 0) (effects (font (size 1.27 1.27))))\n",
            esc(k),
            esc(v)
        ));
    }
    for (i, u) in s.units.iter().enumerate() {
        o.push_str(&format!("    (symbol \"{}_{}_1\"\n", sn, i + 1));
        for p in &u.pins {
            let et = match p.electrical_type {
                ElectricalType::Input => "input",
                ElectricalType::Output => "output",
                ElectricalType::Bidirectional => "bidirectional",
                ElectricalType::TriState => "tri_state",
                ElectricalType::Passive => "passive",
                ElectricalType::PowerInput => "power_in",
                ElectricalType::PowerOutput => "power_out",
                ElectricalType::OpenCollector => "open_collector",
                ElectricalType::OpenEmitter => "open_emitter",
                ElectricalType::NoConnect => "no_connect",
                ElectricalType::Unknown => "unspecified",
            };
            let _=writeln!(o,"      (pin {et} line (at {} {} {}) (length {}) (name \"{}\" (effects (font (size 1.27 1.27)))) (number \"{}\" (effects (font (size 1.27 1.27)))))",mm(p.position.x_nm),mm(p.position.y_nm),angle(p.orientation_mdeg),mm(p.length_nm),esc(&p.name),esc(&p.number));
        }
        o.push_str("    )\n");
    }
    o.push_str("  )\n");
    o
}
pub fn footprint(_c: &EdaComponent, p: &Package) -> String {
    footprint_with_model(_c, p, None)
}
pub fn footprint_with_model(_c: &EdaComponent, p: &Package, model: Option<&str>) -> String {
    let mut o = format!(
        "(footprint \"{}\" (version {}) (generator pcbnew)\n  (layer \"F.Cu\")\n  (attr smd)\n",
        name(&p.name),
        KICAD_VERSION
    );
    o.push_str(&format!("  (fp_text reference \"REF**\" (at 0 0 0) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))\n  (fp_text value \"{}\" (at 0 0 0) (layer \"F.Fab\") (effects (font (size 1 1) (thickness 0.15))))\n",esc(&p.name)));
    for g in &p.graphics {
        if let Graphic::Line {
            start,
            end,
            width_nm,
            layer: sl,
        } = g
        {
            if let Some(l) = layer(sl) {
                let _=writeln!(o,"  (fp_line (start {} {}) (end {} {}) (stroke (width {}) (type default)) (layer \"{}\"))",mm(start.x_nm),mm(start.y_nm),mm(end.x_nm),mm(end.y_nm),mm(*width_nm),l);
            }
        }
    }
    for x in &p.pads {
        let kind = if x.drill.is_some() {
            if x.plated == Some(false) {
                "np_thru_hole"
            } else {
                "thru_hole"
            }
        } else {
            "smd"
        };
        let layers = if kind == "smd" {
            "\"F.Cu\" \"F.Paste\" \"F.Mask\""
        } else {
            "\"*.Cu\" \"*.Mask\""
        };
        let shape = match x.shape.to_ascii_lowercase().as_str() {
            "circle" => "circle",
            "oval" => "oval",
            "roundrect" => "roundrect",
            "rectangle" => "rect",
            _ => "rect",
        };
        let _ = writeln!(
            o,
            "  (pad \"{}\" {} {} (at {} {} {}) (size {} {}) (layers {}))",
            esc(&x.number),
            kind,
            shape,
            mm(x.position.x_nm),
            mm(x.position.y_nm),
            angle(x.rotation_mdeg),
            mm(x.size.x_nm),
            mm(x.size.y_nm),
            layers
        );
    }
    if let Some(path) = model {
        o.push_str(&format!(
            "  (model \"{}\" (offset (xyz 0 0 0)) (scale (xyz 1 1 1)) (rotate (xyz 0 0 0)))\n",
            esc(path)
        ));
    }
    o.push_str(")\n");
    o
}
pub fn check_pin_pad(c: &EdaComponent) -> Vec<String> {
    let mut w = Vec::new();
    for s in &c.symbols {
        let pins = s
            .units
            .iter()
            .flat_map(|u| u.pins.iter().map(|p| p.number.clone()))
            .collect::<BTreeSet<_>>();
        for p in &c.packages {
            let pads = p
                .pads
                .iter()
                .map(|x| x.number.clone())
                .collect::<BTreeSet<_>>();
            for x in pins.difference(&pads) {
                w.push(format!(
                    "{}: symbol pin {} has no pad in {}",
                    s.name, x, p.name
                ))
            }
            for x in pads.difference(&pins) {
                w.push(format!(
                    "{}: pad {} has no symbol pin in {}",
                    p.name, x, s.name
                ))
            }
        }
    }
    w
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaling() {
        assert_eq!(mm(1_000_000), "1");
    }
    #[test]
    fn escaping() {
        assert_eq!(esc("a\"b"), "a\\\"b");
    }
}
