use anyhow::{Context, Result};
use bxl_model::{BxlDocument, BxlRecord};
use eda_model::{
    ComponentPinMap, EdaComponent, ElectricalType, Graphic, Package, Pad, Pin, Point,
    SourcePinSemantics, Symbol, SymbolUnit,
};
use regex::Regex;
use std::collections::BTreeMap;
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
    let sh =
        Regex::new(r#"PadShape\s+"([^"]+)"\s+\(Width\s+([-0-9.]+)\)\s+\(Height\s+([-0-9.]+)\)"#)
            .unwrap();
    let mut stacks: BTreeMap<String, (String, i64, i64)> = BTreeMap::new();
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
        if let Some(m) = sh.captures(line) {
            if let Some(k) = current.clone() {
                stacks.insert(k, (m[1].into(), mil_nm(&m[2]), mil_nm(&m[3])));
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
    let sym = Regex::new(r#"Symbol\s+"([^"]+)""#).unwrap();
    let pad=Regex::new(r#"Pad\s+\(Number\s+([^\)]+)\)\s+\(PinName\s+"([^"]*)"\)\s+\(PadStyle\s+"([^"]+)"\).*?\(Origin\s+([-0-9.]+),\s*([-0-9.]+)\)"#).unwrap();
    let pin=Regex::new(r#"Pin\s+\(PinNum\s+([^\)]+)\)\s+\(Origin\s+([-0-9.]+),\s*([-0-9.]+)\)\s+\(PinLength\s+([-0-9.]+)\).*?(?:\(Rotate\s+([-0-9.]+)\))?"#).unwrap();
    let pname = Regex::new(r#"PinName\s+"([^"]*)""#).unwrap();
    let raw_type =
        Regex::new(r#"(?:ElectricalType|PinType|Electrical)\s+"?([^\s\)\"]+)"?"#).unwrap();
    let raw_shape = Regex::new(r#"(?:PinShape|Shape)\s+"?([^\s\)\"]+)"?"#).unwrap();
    let line_re = Regex::new(
        r#"Line\s+\(Origin\s+([-0-9.]+),\s*([-0-9.]+)\)\s+\(EndPoint\s+([-0-9.]+),\s*([-0-9.]+)\)"#,
    )
    .unwrap();
    let mut package = None;
    let mut symbol = None;
    let mut unit = SymbolUnit {
        name: "unit-1".into(),
        ..Default::default()
    };
    let mut pending = None;
    let mut explicit_maps: BTreeMap<String, String> = BTreeMap::new();
    let mut patterns = Vec::new();
    for line in text.lines() {
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
            })
        }
        if let Some(m) = sym.captures(line) {
            if let Some(s) = symbol.take() {
                c.symbols.push(s)
            }
            symbol = Some(Symbol {
                name: m[1].into(),
                ..Default::default()
            })
        }
        if let Some(m) = pad.captures(line) {
            if let Some(p) = package.as_mut() {
                let (shape, w, h) = stacks
                    .get(&m[3])
                    .cloned()
                    .unwrap_or(("unknown".into(), 0, 0));
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
                    ..Default::default()
                })
            }
        }
        if let Some(m) = line_re.captures(line) {
            if let Some(p) = package.as_mut() {
                p.graphics.push(Graphic::Line {
                    start: Point {
                        x_nm: mil_nm(&m[1]),
                        y_nm: mil_nm(&m[2]),
                    },
                    end: Point {
                        x_nm: mil_nm(&m[3]),
                        y_nm: mil_nm(&m[4]),
                    },
                    width_nm: 0,
                    layer: "unknown".into(),
                });
            }
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
                orientation_mdeg: m
                    .get(5)
                    .map(|x| x.as_str().parse::<f64>().unwrap_or(0.0) * 1000.0)
                    .unwrap_or(0.0) as i32,
                electrical_type,
                visible: true,
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
        if line.contains("EndSymbol") {
            if let Some(mut s) = symbol.take() {
                s.units.push(std::mem::take(&mut unit));
                c.symbols.push(s)
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
}
