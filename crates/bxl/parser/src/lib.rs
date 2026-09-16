use anyhow::{Context, Result};
use bxl_model::{BxlDocument, BxlRecord};
use eda_model::{
    ComponentPinMap, EdaComponent, ElectricalType, Graphic, Package, Pad, Pin, Point,
    SourcePinSemantics, Symbol, SymbolUnit,
};
use regex::Regex;
use std::collections::BTreeMap;
pub const BXL_CANONICALIZER_VERSION: &str = "ti-bxl-canonical-v10";
#[derive(Clone, Debug)]
struct ParsedPadShape { layer: Option<String>, shape: String, width_nm: i64, height_nm: i64 }
fn stack_shape<'a>(shapes: &'a [ParsedPadShape], wanted: &[&str]) -> Option<&'a ParsedPadShape> {
    shapes.iter().find(|s| s.layer.as_deref().is_some_and(|l| wanted.contains(&l)))
}
fn canonical_pad_shape(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "oblong" => "oval".into(),
        "round" => "circle".into(),
        "square" => "rect".into(),
        other => other.into(),
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section { None, Pattern, Symbol }
fn normalize_layer(raw: &str) -> String {
    match raw.trim().to_ascii_uppercase().as_str() {
        "TOP" | "TOP_COPPER" => "TOP_COPPER",
        "TOP_SILKSCREEN" | "TOP SILKSCREEN" | "SILKSCREEN TOP" => "TOP_SILKSCREEN",
        "TOP_ASSEMBLY" | "TOP ASSEMBLY" => "TOP_ASSEMBLY",
        "TOP_SOLDER_MASK" => "TOP_SOLDER_MASK",
        "TOP_SOLDER_PASTE" | "TOP_PASTE" => "TOP_PASTE",
        "BOTTOM_SILKSCREEN" | "BOTTOM SILKSCREEN" => "BOTTOM_SILKSCREEN",
        "BOTTOM_ASSEMBLY" | "BOTTOM ASSEMBLY" => "BOTTOM_ASSEMBLY",
        "BOTTOM_COPPER" => "BOTTOM_COPPER",
        other => other,
    }.into()
}
fn add_graphic(c: &mut EdaComponent, package: &mut Option<Package>, graphic: Graphic, _symbol: bool) {
    if let Some(p) = package.as_mut() {
        let layer = match &graphic { Graphic::Line { layer, .. } | Graphic::Arc { layer, .. } | Graphic::Circle { layer, .. } | Graphic::Rectangle { layer, .. } | Graphic::Polygon { layer, .. } | Graphic::Text { layer, .. } => layer };
        if layer == "PRO_E" || layer == "DIMENSION" || layer == "PAD_DIMENSIONS" || layer == "INPUTDIMENSIONS" || layer == "PIN_DETAIL" { c.warnings.push(format!("unmapped package graphic layer: {layer}")); }
        p.graphics.push(graphic);
    }
}
/// Decompresses the proprietary BXL container using the compatible GPL-3 bxl-rs
/// implementation. The decoded stream is retained for the next grammar phase.
pub fn parse(bytes: &[u8]) -> Result<BxlDocument> {
    let decoded = bxl::decompress(bytes)
        .map_err(|e| anyhow::anyhow!(e))
        .context("BXL decompression failed")?;
    let text = String::from_utf8(decoded.clone()).ok();
    let records = text.as_deref().map(parse_text_records).unwrap_or_default();
    Ok(BxlDocument {
        version: None,
        records,
        raw_text: text,
    })
}
fn parse_text_records(text: &str) -> Vec<BxlRecord> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut p = line.split_whitespace();
            Some(BxlRecord {
                kind: p.next().unwrap_or("unknown").into(),
                fields: p.map(str::to_owned).collect(),
                offset: i,
            })
        })
        .collect()
}
pub fn canonicalize(
    doc: &BxlDocument,
    manufacturer: &str,
    mpn: &str,
    sha256: Option<String>,
) -> EdaComponent {
    let mut c = EdaComponent {
        manufacturer: manufacturer.into(),
        mpn: mpn.into(),
        source_sha256: sha256,
        ..Default::default()
    };
    let Some(text) = doc.raw_text.as_deref() else {
        c.warnings.push("decoded BXL stream is not UTF-8".into());
        return c;
    };
    let ps = Regex::new(r#"PadStack\s+"([^"]+)""#).unwrap();
    let sh = Regex::new(r#"PadShape\s+"([^"]+)".*?\(Width\s+([-0-9.]+)\).*?\(Height\s+([-0-9.]+)\).*?\(Layer\s+([^\)]+)\)"#).unwrap();
    let sh_unlayered = Regex::new(r#"PadShape\s+"([^"]+)".*?\(Width\s+([-0-9.]+)\).*?\(Height\s+([-0-9.]+)\)\s*$"#).unwrap();
    let mut stacks: BTreeMap<String, Vec<ParsedPadShape>> = BTreeMap::new();
    let drill_re = Regex::new(r#"(?:Drill|DrillSize)\s+\(?\s*([-0-9.]+)"#).unwrap();
    let mut stack_drills: BTreeMap<String, i64> = BTreeMap::new();
    let pin_map_re = Regex::new(
        r#"PinMap.*?(?:PinNum\s+\(?\s*([^\s\)]+)).*?PadNum\s+\(?\s*([^\s\)]+)|PinMap\s+([^\s\)]+)\s+([^\s\)]+)"#,
    ).unwrap();
    let mut current = None;
    for line in text.lines() {
        if let Some(m) = ps.captures(line) {
            current = Some(m[1].to_string())
        }
        if let Some(m) = sh.captures(line).or_else(|| sh_unlayered.captures(line)) {
            if let Some(k) = current.clone() {
                stacks.entry(k).or_default().push(ParsedPadShape {
                    layer: m.get(4).map(|x| normalize_layer(x.as_str())),
                    shape: canonical_pad_shape(&m[1]), width_nm: mil_nm(&m[2]), height_nm: mil_nm(&m[3]),
                });
            }
        }
        if let Some(m) = drill_re.captures(line) {
            if let Some(k) = current.clone() {
                stack_drills.insert(k, mil_nm(&m[1]));
            }
        }
        if line.contains("EndPadStack") {
            current = None
        }
    }
    let pat = Regex::new(r#"^\s*Pattern\s+"([^"]+)""#).unwrap();
    let sym = Regex::new(r#"^\s*Symbol\s+"([^"]+)""#).unwrap();
    let pad=Regex::new(r#"Pad\s+\(Number\s+([^\)]+)\)\s+\(PinName\s+"([^"]*)"\)\s+\(PadStyle\s+"([^"]+)"\).*?\(Origin\s+([-0-9.]+),\s*([-0-9.]+)\)"#).unwrap();
    let pad_rotate_re = Regex::new(r#"\(Rotate\s+([-0-9.]+)\)"#).unwrap();
    let pin=Regex::new(r#"Pin\s+\(PinNum\s+([^\)]+)\)\s+\(Origin\s+([-0-9.]+),\s*([-0-9.]+)\)\s+\(PinLength\s+([-0-9.]+)\)"#).unwrap();
    let rotate_re = Regex::new(r#"\(Rotate\s+([-0-9.]+)\)"#).unwrap();
    let pname = Regex::new(r#"PinName\s+"([^"]*)""#).unwrap();
    let raw_type =
        Regex::new(r#"(?:ElectricalType|PinType|Electrical)\s+"?([^\s\)\"]+)"?"#).unwrap();
    let raw_shape = Regex::new(r#"(?:PinShape|Shape)\s+"?([^\s\)\"]+)"?"#).unwrap();
    let line_re = Regex::new(r#"Line\s+\(Layer\s+([^\)]+)\)\s+\(Origin\s+([-0-9.]+),\s*([-0-9.]+)\)\s+\(EndPoint\s+([-0-9.]+),\s*([-0-9.]+)\)(?:\s+\(Width\s+([-0-9.]+)\))?"#).unwrap();
    let symbol_line_re = Regex::new(r#"Line\s+\(Origin\s+([-0-9.]+),\s*([-0-9.]+)\)\s+\(EndPoint\s+([-0-9.]+),\s*([-0-9.]+)\)(?:\s+\(Width\s+([-0-9.]+)\))?"#).unwrap();
    let arc_re = Regex::new(r#"Arc\s+\(Layer\s+([^\)]+)\)\s+\(Origin\s+([-0-9.]+),\s*([-0-9.]+)\)\s+\(Radius\s+([-0-9.]+)\)(?:\s+\(StartAngle\s+([-0-9.]+)\))?\s+\(SweepAngle\s+([-0-9.]+)\)(?:\s+\(Width\s+([-0-9.]+)\))?"#).unwrap();
    let text_re = Regex::new(r#"Text\s+\(Layer\s+([^\)]+)\)\s+\(Origin\s+([-0-9.]+),\s*([-0-9.]+)\)\s+\(Text\s+"([^"]*)"\).*?\(IsVisible\s+(True|False)\)"#).unwrap();
    let style_re = Regex::new(r#"TextStyle\s+"([^"]+)"\s+\(FontWidth\s+([-0-9.]+)\)\s+\(FontHeight\s+([-0-9.]+)\)"#).unwrap();
    let mut styles = BTreeMap::<String, (i64, i64)>::new();
    for m in style_re.captures_iter(text) { styles.insert(m[1].into(), (mil_nm(&m[2]), mil_nm(&m[3]))); }
    let mut section = Section::None;
    let mut package = None;
    let mut symbol: Option<Symbol> = None;
    let mut unit = SymbolUnit {
        name: "unit-1".into(),
        ..Default::default()
    };
    let mut pending = None;
    let mut explicit_maps: BTreeMap<String, String> = BTreeMap::new();
    let mut patterns = Vec::new();
    for line in text.lines() {
        if line.contains("EndPattern") { section = Section::None; if let Some(p) = package.take() { c.packages.push(p); } continue; }
        if line.contains("EndSymbol") { section = Section::None; if let Some(mut s) = symbol.take() { s.units.push(std::mem::take(&mut unit)); c.symbols.push(s); } continue; }
        if let Some(m) = pin_map_re.captures(line) {
            let pin = m.get(1).or_else(|| m.get(3)).map(|x| x.as_str());
            let pad = m.get(2).or_else(|| m.get(4)).map(|x| x.as_str());
            if let (Some(pin), Some(pad)) = (pin, pad) {
                explicit_maps.insert(pin.trim_matches('"').into(), pad.trim_matches('"').into());
            }
        }
        for key in ["PatternName", "AlternatePattern"] {
            if line.contains(key) {
                if let Some(value) = line.split('"').nth(1) {
                    patterns.push(format!("{key}:{value}"));
                }
            }
        }
        if let Some(m) = pat.captures(line) {
            if let Some(p) = package.take() {
                c.packages.push(p)
            }
            package = Some(Package {
                name: m[1].into(),
                ..Default::default()
            });
            section = Section::Pattern;
        }
        if let Some(m) = sym.captures(line) {
            if let Some(s) = symbol.take() {
                c.symbols.push(s)
            }
            symbol = Some(Symbol {
                name: m[1].into(),
                ..Default::default()
            });
            unit = SymbolUnit { name: "unit-1".into(), ..Default::default() };
            section = Section::Symbol;
        }
        if let Some(m) = pad.captures(line) {
            if let Some(p) = package.as_mut() {
                let selected = stacks.get(&m[3]).and_then(|shapes| {
                    shapes.iter().find(|s| matches!(s.layer.as_deref(), Some("TOP_COPPER") | Some("TOP")))
                        .or_else(|| shapes.iter().find(|s| s.layer.is_none()))
                        .or_else(|| shapes.iter().find(|s| matches!(s.layer.as_deref(), Some("BOTTOM_COPPER") | Some("BOTTOM"))))
                });
                let (shape, w, h) = selected.map(|s| (s.shape.clone(), s.width_nm, s.height_nm))
                    .unwrap_or(("unknown".into(), 0, 0));
                let mask = stacks.get(&m[3]).and_then(|shapes| stack_shape(shapes, &["TOP_SOLDER_MASK", "BOTTOM_SOLDER_MASK"]));
                let paste = stacks.get(&m[3]).and_then(|shapes| stack_shape(shapes, &["TOP_PASTE", "TOP_SOLDER_PASTE", "BOTTOM_PASTE", "BOTTOM_SOLDER_PASTE"]));
                let mask_expansion = mask.and_then(|s| {
                    let dx = (s.width_nm - w) / 2;
                    let dy = (s.height_nm - h) / 2;
                    (dx == dy).then_some(dx)
                });
                p.pads.push(Pad {
                    number: m[1].trim().into(),
                    name: Some(m[2].into()),
                    shape,
                    position: Point {
                        x_nm: mil_nm(&m[4]),
                        y_nm: mil_nm(&m[5]),
                    },
                    size: Point { x_nm: w, y_nm: h },
                    drill: stack_drills
                        .get(&m[3])
                        .map(|d| Point { x_nm: *d, y_nm: *d }),
                    // PadStyle is a geometry/style identifier, not a plating
                    // declaration. Leave plating unknown unless an explicit
                    // source attribute is decoded.
                    plated: None,
                    rotation_mdeg: pad_rotate_re.captures(line).and_then(|x| x[1].parse::<f64>().ok()).unwrap_or(0.0) as i32 * 1000,
                    solder_mask_size: mask.map(|s| Point { x_nm: s.width_nm, y_nm: s.height_nm }),
                    paste_size: paste.map(|s| Point { x_nm: s.width_nm, y_nm: s.height_nm }),
                    solder_mask_expansion_nm: mask_expansion,
                    ..Default::default()
                })
            }
        }
        if section == Section::Pattern {
            if let Some(m) = line_re.captures(line) {
                add_graphic(&mut c, &mut package, Graphic::Line {
                    start: Point {
                        x_nm: mil_nm(&m[2]), y_nm: mil_nm(&m[3]),
                    },
                    end: Point {
                        x_nm: mil_nm(&m[4]), y_nm: mil_nm(&m[5]),
                    },
                    width_nm: m.get(6).map_or(0, |x| mil_nm(x.as_str())), layer: normalize_layer(&m[1]),
                }, false);
            }
            if let Some(m) = arc_re.captures(line) {
                let cx=mil_nm(&m[2]); let cy=mil_nm(&m[3]); let r=mil_nm(&m[4]); let start: f64=m.get(5).map_or(0.0,|x|x.as_str().parse().unwrap_or(0.0)); let sweep=m[6].parse::<f64>().unwrap_or(0.0); let a=start.to_radians(); let z=(start+sweep).to_radians();
                let g = if sweep.abs() >= 359.999 { Graphic::Circle { center:Point{x_nm:cx,y_nm:cy}, radius_nm:r, width_nm:m.get(7).map_or(0,|x|mil_nm(x.as_str())), layer:normalize_layer(&m[1]) } } else { Graphic::Arc { center:Point{x_nm:cx,y_nm:cy}, start:Point{x_nm:cx+(r as f64*a.cos()) as i64,y_nm:cy+(r as f64*a.sin()) as i64}, end:Point{x_nm:cx+(r as f64*z.cos()) as i64,y_nm:cy+(r as f64*z.sin()) as i64}, width_nm:m.get(7).map_or(0,|x|mil_nm(x.as_str())), layer:normalize_layer(&m[1]) } };
                add_graphic(&mut c, &mut package, g, false);
            }
            if let Some(m) = text_re.captures(line) { add_graphic(&mut c, &mut package, Graphic::Text { text:m[4].into(), position:Point{x_nm:mil_nm(&m[2]),y_nm:mil_nm(&m[3])}, rotation_mdeg:0, height_nm:0, width_nm:0, layer:normalize_layer(&m[1]) }, false); }
        } else if section == Section::Symbol {
            if let Some(m) = symbol_line_re.captures(line) { unit.graphics.push(Graphic::Line { start:Point{x_nm:mil_nm(&m[1]),y_nm:mil_nm(&m[2])}, end:Point{x_nm:mil_nm(&m[3]),y_nm:mil_nm(&m[4])}, width_nm:m.get(5).map_or(0,|x|mil_nm(x.as_str())), layer:"symbol".into() }); }
            if let Some(m) = arc_re.captures(line) { let cx=mil_nm(&m[2]); let cy=mil_nm(&m[3]); let r=mil_nm(&m[4]); let start: f64=m.get(5).map_or(0.0,|x|x.as_str().parse().unwrap_or(0.0)); let sweep=m[6].parse::<f64>().unwrap_or(0.0); let a=start.to_radians(); let z=(start+sweep).to_radians(); if sweep.abs()>=359.999 { unit.graphics.push(Graphic::Circle {center:Point{x_nm:cx,y_nm:cy},radius_nm:r,width_nm:m.get(7).map_or(0,|x|mil_nm(x.as_str())),layer:"symbol".into()}); } else { unit.graphics.push(Graphic::Arc {center:Point{x_nm:cx,y_nm:cy},start:Point{x_nm:cx+(r as f64*a.cos()) as i64,y_nm:cy+(r as f64*a.sin()) as i64},end:Point{x_nm:cx+(r as f64*z.cos()) as i64,y_nm:cy+(r as f64*z.sin()) as i64},width_nm:m.get(7).map_or(0,|x|mil_nm(x.as_str())),layer:"symbol".into()}); } }
            if let Some(m) = text_re.captures(line) { unit.graphics.push(Graphic::Text {text:m[4].into(),position:Point{x_nm:mil_nm(&m[2]),y_nm:mil_nm(&m[3])},rotation_mdeg:0,height_nm:0,width_nm:0,layer:"symbol".into()}); }
        }
        if let Some(m) = pin.captures(line) {
            let raw = raw_type.captures(line).map(|x| x[1].to_string());
            let shape = raw_shape.captures(line).map(|x| x[1].to_string());
            let mut flags = Vec::new();
            for f in ["Hidden", "Inverted", "Clock", "NoConnect"] {
                if line.to_ascii_lowercase().contains(&f.to_ascii_lowercase()) {
                    flags.push(f.into());
                }
            }
            if line.contains("(IsVisible False)") && !flags.iter().any(|x| x == "Hidden") {
                flags.push("Hidden".into());
            }
            let electrical_type = raw
                .as_deref()
                .map(map_electrical)
                .unwrap_or(ElectricalType::Unknown);
            pending = Some(Pin {
                number: m[1].trim().into(),
                position: Point {
                    x_nm: mil_nm(&m[2]),
                    y_nm: mil_nm(&m[3]),
                },
                length_nm: mil_nm(&m[4]),
                orientation_mdeg: rotate_re.captures(line).and_then(|x| x[1].parse::<f64>().ok()).unwrap_or(0.0) as i32 * 1000,
                electrical_type,
                visible: !line.contains("(IsVisible False)"),
                source_semantics: SourcePinSemantics {
                    electrical_type_raw: raw,
                    shape_raw: shape,
                    flags,
                    logical_id: Some(m[1].trim().into()),
                },
                ..Default::default()
            })
        }
        if let Some(m) = pname.captures(line) {
            if let Some(mut p) = pending.take() {
                p.name = m[1].into();
                unit.pins.push(p)
            }
        }
    }
    if let Some(p) = package {
        c.packages.push(p)
    }
    if let Some(s) = symbol {
        c.symbols.push(s)
    }
    c.metadata.insert(
        "source_units".into(),
        "mil (inferred from decoded TI corpus)".into(),
    );
    if !patterns.is_empty() {
        patterns.sort();
        patterns.dedup();
        c.metadata
            .insert("pattern_relationships".into(), patterns.join(";"));
    }
    for s in &c.symbols {
        for u in &s.units {
            for p in &u.pins {
                c.pin_map.push(ComponentPinMap {
                    logical_id: p
                        .source_semantics
                        .logical_id
                        .clone()
                        .unwrap_or_else(|| p.number.clone()),
                    pin_number: p.number.clone(),
                    pin_name: p.name.clone(),
                    unit: 1,
                    symbol_pin_ref: Some(p.number.clone()),
                    pad_ref: explicit_maps.get(&p.number).cloned(),
                    electrical_type_raw: p.source_semantics.electrical_type_raw.clone(),
                    flags: p.source_semantics.flags.clone(),
                    explicit: explicit_maps.contains_key(&p.number),
                });
            }
        }
    }
    c
}
fn map_electrical(raw: &str) -> ElectricalType {
    match raw.to_ascii_lowercase().as_str() {
        "input" | "in" => ElectricalType::Input,
        "output" | "out" => ElectricalType::Output,
        "bidirectional" | "bidir" | "io" => ElectricalType::Bidirectional,
        "tristate" | "tri_state" => ElectricalType::TriState,
        "passive" => ElectricalType::Passive,
        "powerinput" | "power_input" => ElectricalType::PowerInput,
        "poweroutput" | "power_output" => ElectricalType::PowerOutput,
        "opencollector" | "open_collector" => ElectricalType::OpenCollector,
        "openemitter" | "open_emitter" => ElectricalType::OpenEmitter,
        "noconnect" | "no_connect" | "nc" => ElectricalType::NoConnect,
        _ => ElectricalType::Unknown,
    }
}
fn mil_nm(v: &str) -> i64 {
    v.parse::<f64>()
        .unwrap_or(0.0)
        .mul_add(25400.0, 0.0)
        .round() as i64
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_text_records() {
        let d = parse_text_records("HEADER 1\nPIN 1 A");
        assert_eq!(d[1].kind, "PIN");
    }

    #[test]
    fn canonicalizes_stack_mapping_and_pattern_relationships() {
        let doc = BxlDocument {
            version: None,
            records: Vec::new(),
            raw_text: Some("PadStack \"TH\"\nPadShape \"Circle\" (Width 40) (Height 40)\n(Drill 20)\nEndPadStack\nPattern \"DIP2\"\nPatternName \"DIP2\"\nAlternatePattern \"DIP2-HAND\"\nPad (Number 1) (PinName \"A\") (PadStyle \"TH\") (Origin 0, 0)\nComponent \"U1\"\nPinMap (PinNum 1) (PadNum 1)\nSymbol \"U1\"\nPin (PinNum 1) (Origin 0, 0) (PinLength 10) (ElectricalType \"Input\")\nPinName \"A\"\nEndSymbol\n".into()),
        };
        let component = canonicalize(&doc, "TI", "TEST", None);
        let pad = &component.packages[0].pads[0];
        assert_eq!(
            pad.drill,
            Some(Point {
                x_nm: 508000,
                y_nm: 508000
            })
        );
        assert_eq!(pad.plated, None);
        let mapping = &component.pin_map[0];
        assert_eq!(mapping.pad_ref.as_deref(), Some("1"));
        assert!(mapping.explicit);
        assert!(component
            .metadata
            .get("pattern_relationships")
            .is_some_and(
                |v| v.contains("PatternName:DIP2") && v.contains("AlternatePattern:DIP2-HAND")
            ));
    }

    #[test]
    fn preserves_graphics_by_explicit_section_and_pin_visibility() {
        let doc = BxlDocument { version: None, records: Vec::new(), raw_text: Some(r#"
Pattern "PKG"
  Line (Layer TOP_SILKSCREEN) (Origin -10, -5) (EndPoint 10, -5) (Width 2)
EndPattern
Symbol "BODY"
  Line (Origin -5, -5) (EndPoint 5, -5) (Width 3)
  Pin (PinNum 1) (Origin 5, 0) (PinLength 2) (Rotate 180) (IsVisible False)
    PinName "H"
EndSymbol
"#.into()) };
        let c = canonicalize(&doc, "TI", "X", None);
        assert_eq!(c.packages[0].graphics.len(), 1);
        assert_eq!(c.packages[0].graphics[0], Graphic::Line { start: Point{x_nm:-254000,y_nm:-127000}, end: Point{x_nm:254000,y_nm:-127000}, width_nm:50800, layer:"TOP_SILKSCREEN".into() });
        assert_eq!(c.symbols[0].units[0].graphics.len(), 1);
        assert_eq!(c.symbols[0].units[0].graphics[0], Graphic::Line { start: Point{x_nm:-127000,y_nm:-127000}, end: Point{x_nm:127000,y_nm:-127000}, width_nm:76200, layer:"symbol".into() });
        assert!(!c.symbols[0].units[0].pins[0].visible);
        assert!(c.symbols[0].units[0].pins[0].source_semantics.flags.iter().any(|x| x == "Hidden"));
        assert_eq!(c.symbols[0].units[0].pins[0].orientation_mdeg, 180000);
    }

    #[test]
    fn preserves_pad_rotation_without_swapping_local_dimensions() {
        let doc = BxlDocument { version: None, records: Vec::new(), raw_text: Some(r#"
PadStack "S"
 PadShape "Rectangle" (Width 10) (Height 20) (Layer TOP)
EndPadStack
Pattern "DCQ"
 Pad (Number 1) (PinName "1") (PadStyle "S") (Origin 0, 0) (Rotate 90)
EndPattern
"#.into()) };
        let c = canonicalize(&doc, "TI", "X", None);
        let p = &c.packages[0].pads[0];
        assert_eq!((p.size.x_nm, p.size.y_nm, p.rotation_mdeg), (254000, 508000, 90000));
    }

    #[test]
    fn selects_copper_shape_from_layered_pad_stack() {
        let doc = BxlDocument { version: None, records: Vec::new(), raw_text: Some(r#"
PadStack "S"
 PadShape "Rectangle" (Width 24) (Height 48) (PadType 0) (Layer TOP)
 PadShape "Rectangle" (Width 28) (Height 52) (PadType 0) (Layer TOP_SOLDER_MASK)
 PadShape "Rectangle" (Width 20) (Height 44) (PadType 0) (Layer TOP_SOLDER_PASTE)
EndPadStack
Pattern "P"
 Pad (Number 1) (PinName "1") (PadStyle "S") (Origin 0, 0)
EndPattern
"#.into()) };
        let c = canonicalize(&doc, "TI", "X", None);
        assert_eq!(c.packages[0].pads[0].size, Point { x_nm: 609600, y_nm: 1219200 });
        assert_eq!(c.packages[0].pads[0].solder_mask_size, Some(Point { x_nm: 711200, y_nm: 1320800 }));
        assert_eq!(c.packages[0].pads[0].paste_size, Some(Point { x_nm: 508000, y_nm: 1117600 }));
    }
}
