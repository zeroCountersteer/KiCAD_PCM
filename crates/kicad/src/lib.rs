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
            if x.drill.is_some() && x.plated.is_none() {
                v.errors.push(format!(
                    "{} pad {}: drill plating is unspecified",
                    p.name, x.number
                ));
            }
            if x.number.trim().is_empty() {
                v.errors.push(format!("{}: blank pad", p.name))
            }
            if !seen.insert(&x.number) {
                v.errors
                    .push(format!("{}: duplicate pad {}", p.name, x.number))
            }
            if !matches!(
                x.shape.to_ascii_lowercase().as_str(),
                "circle" | "oval" | "roundrect" | "rectangle" | "rect"
            ) {
                v.errors.push(format!(
                    "{} pad {}: unsupported source shape {}",
                    p.name, x.number, x.shape
                ));
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
fn property(k: &str, v: &str, x: f64, y: f64, hidden: bool) -> String {
    let hide = if hidden { " hide" } else { "" };
    format!("    (property \"{}\" \"{}\" (at {:.6} {:.6} 0) (effects (font (size 1.27 1.27)){}))\n", esc(k), esc(v), x, y, hide)
}
fn graphic_bounds(gs: &[Graphic]) -> Option<(i64, i64, i64, i64)> {
    let mut b: Option<(i64, i64, i64, i64)> = None;
    let mut add = |x: i64, y: i64| b = Some(match b { Some((a,c,d,e)) => (a.min(x), c.min(y), d.max(x), e.max(y)), None => (x,y,x,y) });
    for g in gs {
        match g {
            Graphic::Line { start, end, .. } | Graphic::Arc { start, end, .. } => { add(start.x_nm,start.y_nm); add(end.x_nm,end.y_nm); }
            Graphic::Circle { center, radius_nm, .. } => { add(center.x_nm-radius_nm,center.y_nm-radius_nm); add(center.x_nm+radius_nm,center.y_nm+radius_nm); }
            Graphic::Rectangle { start, end, .. } => { add(start.x_nm,start.y_nm); add(end.x_nm,end.y_nm); }
            Graphic::Polygon { points, .. } => for p in points { add(p.x_nm,p.y_nm); }
            Graphic::Text { position, .. } => add(position.x_nm,position.y_nm),
        }
    }
    b
}
pub fn symbol_bounds(s: &Symbol) -> (i64, i64, i64, i64) {
    let mut b = graphic_bounds(&s.units.iter().flat_map(|u| u.graphics.clone()).collect::<Vec<_>>());
    let mut add = |x: i64, y: i64| b = Some(match b { Some((a,c,d,e)) => (a.min(x), c.min(y), d.max(x), e.max(y)), None => (x,y,x,y) });
    for u in &s.units { for p in &u.pins {
        let r = (p.orientation_mdeg as f64).to_radians();
        add(p.position.x_nm, p.position.y_nm);
        add(p.position.x_nm + (r.cos() * p.length_nm as f64) as i64, p.position.y_nm + (r.sin() * p.length_nm as f64) as i64);
    }}
    b.unwrap_or((0,0,0,0))
}
fn arc_mid(center: &eda_model::Point, start: &eda_model::Point, end: &eda_model::Point) -> (i64, i64) {
    let a = ((start.y_nm-center.y_nm) as f64).atan2((start.x_nm-center.x_nm) as f64);
    let mut d = ((end.y_nm-center.y_nm) as f64).atan2((end.x_nm-center.x_nm) as f64) - a;
    while d > std::f64::consts::PI { d -= 2.0*std::f64::consts::PI; }
    while d < -std::f64::consts::PI { d += 2.0*std::f64::consts::PI; }
    let r = (((start.x_nm-center.x_nm).pow(2) + (start.y_nm-center.y_nm).pow(2)) as f64).sqrt();
    let m = a + d/2.0;
    (center.x_nm + (r*m.cos()) as i64, center.y_nm + (r*m.sin()) as i64)
}
fn stroke(width_nm: i64) -> String { format!("(stroke (width {}) (type default))", mm(width_nm)) }
fn fill(fill: bool, background: bool) -> String { if fill { if background { "(fill (type background))" } else { "(fill (type solid))" } } else { "(fill (type none))" }.into() }
fn symbol_graphic(g: &Graphic) -> Option<String> {
    Some(match g {
        Graphic::Line { start, end, width_nm, .. } => format!("      (polyline (pts (xy {} {}) (xy {} {})) {} {})\n",mm(start.x_nm),mm(start.y_nm),mm(end.x_nm),mm(end.y_nm),stroke(*width_nm),fill(false,true)),
        Graphic::Arc { center, start, end, width_nm, .. } => { let (x,y)=arc_mid(center,start,end); format!("      (arc (start {} {}) (mid {} {}) (end {} {}) {} {})\n",mm(start.x_nm),mm(start.y_nm),mm(x),mm(y),mm(end.x_nm),mm(end.y_nm),stroke(*width_nm),fill(false,true)) },
        Graphic::Circle { center, radius_nm, width_nm, .. } => format!("      (circle (center {} {}) (radius {}) {} {})\n",mm(center.x_nm),mm(center.y_nm),mm(*radius_nm),stroke(*width_nm),fill(false,true)),
        Graphic::Rectangle { start, end, width_nm, fill: f, .. } => format!("      (rectangle (start {} {}) (end {} {}) {} {})\n",mm(start.x_nm),mm(start.y_nm),mm(end.x_nm),mm(end.y_nm),stroke(*width_nm),fill(*f,true)),
        Graphic::Polygon { points, width_nm, fill: f, .. } => { if points.len()<2 { return None; } let pts=points.iter().map(|p|format!("(xy {} {})",mm(p.x_nm),mm(p.y_nm))).collect::<Vec<_>>().join(" "); format!("      (polyline (pts {}) {} {})\n",pts,stroke(*width_nm),fill(*f,true)) },
        Graphic::Text { text, position, rotation_mdeg, height_nm, width_nm, .. } => format!("      (text \"{}\" (at {} {} {}) (effects (font (size {} {}))))\n",esc(text),mm(position.x_nm),mm(position.y_nm),angle(*rotation_mdeg),mm(*width_nm),mm(*height_nm)),
    })
}
fn symbol(
    c: &EdaComponent,
    s: &Symbol,
    nickname: &str,
    footprints: &std::collections::BTreeMap<String, String>,
) -> String {
    let sn = name(&s.name);
    let mut o = format!("  (symbol \"{sn}\"\n    (pin_names (offset 1.016))\n    (exclude_from_sim no)\n    (in_bom yes)\n    (on_board yes)\n");
    let mapped_name = c
        .packages
        .iter()
        .find_map(|p| footprints.get(&format!("{}|{}|{}", c.manufacturer, c.mpn, p.name)))
        .cloned();
    let footprint_value = mapped_name
        .map(|name| format!("{nickname}:{name}"))
        .unwrap_or_default();
    let (_,min_y,_,max_y) = symbol_bounds(s);
    o.push_str(&property("Reference", "U", 0.0, max_y as f64/1_000_000.0 + 2.54, false));
    o.push_str(&property("Value", &c.mpn, 0.0, min_y as f64/1_000_000.0 - 2.54, false));
    o.push_str(&property("Footprint", &footprint_value, 0.0, 0.0, true));
    o.push_str(&property("Datasheet", "", 0.0, 0.0, true));
    o.push_str(&property("Manufacturer", &c.manufacturer, 0.0, 0.0, true));
    o.push_str(&property("MPN", &c.mpn, 0.0, 0.0, true));
    o.push_str(&property("SourceSHA256", c.source_sha256.as_deref().unwrap_or(""), 0.0, 0.0, true));
    for (i, u) in s.units.iter().enumerate() {
        o.push_str(&format!("    (symbol \"{}_{}_1\"\n", sn, i + 1));
        for g in &u.graphics { if let Some(out) = symbol_graphic(g) { o.push_str(&out); } }
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
            let hidden = if p.visible { "" } else { " hide" };
            let _=writeln!(o,"      (pin {et} line{hidden} (at {} {} {}) (length {}) (name \"{}\" (effects (font (size 1.27 1.27)))) (number \"{}\" (effects (font (size 1.27 1.27)))))",mm(p.position.x_nm),mm(p.position.y_nm),angle(p.orientation_mdeg),mm(p.length_nm),esc(&p.name),esc(&p.number));
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
    let has_through_hole = p
        .pads
        .iter()
        .any(|pad| pad.drill.is_some() && pad.plated.is_some());
    let mut o = format!(
        "(footprint \"{}\" (version {}) (generator pcbnew)\n  (layer \"F.Cu\")\n",
        name(&p.name),
        KICAD_VERSION
    );
    if !has_through_hole {
        o.push_str("  (attr smd)\n");
    }
    let mut min_x = i64::MAX; let mut min_y = i64::MAX; let mut max_x = i64::MIN; let mut max_y = i64::MIN;
    let mut bound = |x: i64, y: i64| { min_x=min_x.min(x); min_y=min_y.min(y); max_x=max_x.max(x); max_y=max_y.max(y); };
    for pad in &p.pads { bound(pad.position.x_nm-pad.size.x_nm/2,pad.position.y_nm-pad.size.y_nm/2); bound(pad.position.x_nm+pad.size.x_nm/2,pad.position.y_nm+pad.size.y_nm/2); }
    if let Some((a,b,c,d)) = graphic_bounds(&p.graphics) { bound(a,b); bound(c,d); }
    if min_x == i64::MAX { min_x = -500_000; min_y = -500_000; max_x = 500_000; max_y = 500_000; }
    o.push_str(&format!("  (fp_text reference \"REF**\" (at {} {} 0) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))\n  (fp_text value \"{}\" (at {} {} 0) (layer \"F.Fab\") (effects (font (size 1 1) (thickness 0.15))))\n",mm((min_x+max_x)/2),mm(max_y+2_000_000),esc(&p.name),mm((min_x+max_x)/2),mm(min_y-2_000_000)));
    for g in &p.graphics {
        if let Some(out) = footprint_graphic(g) { o.push_str(&out); }
    }
    for x in &p.pads {
        if x.drill.is_some() && x.plated.is_none() {
            o.push_str(&format!(
                "  (fp_text user \"UNSUPPORTED: unknown drill plating for pad {}\" (at 0 0 0) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))\n",
                esc(&x.number)
            ));
            continue;
        }
        let kind = if x.drill.is_some() {
            if x.plated == Some(false) {
                "np_thru_hole"
            } else {
                "thru_hole"
            }
        } else {
            "smd"
        };
        let default_layers = if kind == "smd" {
            "\"F.Cu\" \"F.Paste\" \"F.Mask\""
        } else {
            "\"*.Cu\" \"*.Mask\""
        };
        let layers = if x.layers.is_empty() {
            default_layers.to_string()
        } else {
            x.layers
                .iter()
                .map(|source| layer(source).unwrap_or(source.as_str()))
                .map(|name| format!("\"{}\"", esc(name)))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let shape = match x.shape.to_ascii_lowercase().as_str() {
            "circle" => "circle",
            "oval" => "oval",
            "roundrect" => "roundrect",
            "rectangle" | "rect" => "rect",
            _ => "custom",
        };
        let drill = x
            .drill
            .as_ref()
            .map(|d| format!(" (drill {})", mm(d.x_nm)))
            .unwrap_or_default();
        let _ = writeln!(
            o,
            "  (pad \"{}\" {} {} (at {} {} {}) (size {} {}) (layers {}){})",
            esc(&x.number),
            kind,
            shape,
            mm(x.position.x_nm),
            mm(x.position.y_nm),
            angle(x.rotation_mdeg),
            mm(x.size.x_nm),
            mm(x.size.y_nm),
            layers,
            drill
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
fn footprint_graphic(g: &Graphic) -> Option<String> {
    let src = match g { Graphic::Line{layer,..}|Graphic::Arc{layer,..}|Graphic::Circle{layer,..}|Graphic::Rectangle{layer,..}|Graphic::Polygon{layer,..}|Graphic::Text{layer,..} => layer };
    let l = layer(src)?;
    Some(match g {
        Graphic::Line { start,end,width_nm,.. } => format!("  (fp_line (start {} {}) (end {} {}) {} (layer \"{}\"))\n",mm(start.x_nm),mm(start.y_nm),mm(end.x_nm),mm(end.y_nm),stroke(*width_nm),l),
        Graphic::Arc { center,start,end,width_nm,.. } => { let (x,y)=arc_mid(center,start,end); format!("  (fp_arc (start {} {}) (mid {} {}) (end {} {}) {} (layer \"{}\"))\n",mm(start.x_nm),mm(start.y_nm),mm(x),mm(y),mm(end.x_nm),mm(end.y_nm),stroke(*width_nm),l) },
        Graphic::Circle { center,radius_nm,width_nm,.. } => format!("  (fp_circle (center {} {}) (end {} {}) {} (fill none) (layer \"{}\"))\n",mm(center.x_nm),mm(center.y_nm),mm(center.x_nm+radius_nm),mm(center.y_nm),stroke(*width_nm),l),
        Graphic::Rectangle { start,end,width_nm,fill,.. } => format!("  (fp_rect (start {} {}) (end {} {}) {} (fill {}) (layer \"{}\"))\n",mm(start.x_nm),mm(start.y_nm),mm(end.x_nm),mm(end.y_nm),stroke(*width_nm),if *fill {"solid"} else {"none"},l),
        Graphic::Polygon { points,width_nm,fill,.. } => { if points.len()<2 { return None; } let pts=points.iter().map(|p|format!("(xy {} {})",mm(p.x_nm),mm(p.y_nm))).collect::<Vec<_>>().join(" "); format!("  (fp_poly (pts {}) {} (fill {}) (layer \"{}\"))\n",pts,stroke(*width_nm),if *fill {"solid"} else {"none"},l) },
        Graphic::Text { text,position,rotation_mdeg,height_nm,width_nm,.. } => format!("  (fp_text user \"{}\" (at {} {} {}) (layer \"{}\") (effects (font (size {} {}))))\n",esc(text),mm(position.x_nm),mm(position.y_nm),angle(*rotation_mdeg),l,mm(*width_nm),mm(*height_nm)),
    })
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
    use eda_model::Point;
    #[test]
    fn scaling() {
        assert_eq!(mm(1_000_000), "1");
    }
    #[test]
    fn escaping() {
        assert_eq!(esc("a\"b"), "a\\\"b");
    }
    #[test]
    fn through_hole_output_has_drill_and_no_smd_attribute() {
        let package = Package {
            name: "HDR".into(),
            pads: vec![eda_model::Pad {
                number: "1".into(),
                shape: "circle".into(),
                size: eda_model::Point {
                    x_nm: 1_600_000,
                    y_nm: 1_600_000,
                },
                drill: Some(eda_model::Point {
                    x_nm: 800_000,
                    y_nm: 800_000,
                }),
                plated: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = footprint(&EdaComponent::default(), &package);
        assert!(!out.contains("(attr smd)"));
        assert!(out.contains("np_thru_hole") || out.contains("thru_hole"));
        assert!(out.contains("(drill 0.8)"));
    }

    #[test]
    fn unknown_drill_plating_is_marked_and_not_emitted_as_through_hole() {
        let package = Package {
            name: "UNKNOWN".into(),
            pads: vec![eda_model::Pad {
                number: "1".into(),
                shape: "rect".into(),
                size: eda_model::Point {
                    x_nm: 1_000_000,
                    y_nm: 1_000_000,
                },
                drill: Some(eda_model::Point {
                    x_nm: 500_000,
                    y_nm: 500_000,
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = footprint(&EdaComponent::default(), &package);
        assert!(out.contains("UNSUPPORTED: unknown drill plating"));
        assert!(!out.contains("thru_hole"));
    }

    #[test]
    fn symbol_has_no_fallback_for_unsafe_package_and_uses_later_safe_package() {
        let unsafe_package = Package {
            name: "UNSAFE".into(),
            pads: vec![eda_model::Pad {
                number: "1".into(),
                shape: "circle".into(),
                size: eda_model::Point {
                    x_nm: 1_000_000,
                    y_nm: 1_000_000,
                },
                drill: Some(eda_model::Point {
                    x_nm: 500_000,
                    y_nm: 500_000,
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let safe_package = Package {
            name: "SAFE".into(),
            ..Default::default()
        };
        let component = EdaComponent {
            manufacturer: "TI".into(),
            mpn: "X".into(),
            symbols: vec![eda_model::Symbol {
                name: "X".into(),
                ..Default::default()
            }],
            packages: vec![unsafe_package, safe_package],
            ..Default::default()
        };
        let empty = symbol_lib_many_with_footprints(
            &[EdaComponent {
                packages: vec![component.packages[0].clone()],
                ..component.clone()
            }],
            "N",
            &Default::default(),
        );
        assert!(empty.contains("(property \"Footprint\" \"\""));
        let mut map = std::collections::BTreeMap::new();
        map.insert("TI|X|SAFE".into(), "SafeFootprint".into());
        let output = symbol_lib_many_with_footprints(&[component], "N", &map);
        assert!(output.contains("(property \"Footprint\" \"N:SafeFootprint\""));
        assert!(!output.contains("N:UNSAFE"));
    }

    #[test]
    fn symbol_emits_graphics_hidden_pin_and_separated_properties() {
        let s = Symbol { name: "G".into(), units: vec![eda_model::SymbolUnit {
            graphics: vec![Graphic::Rectangle { start: Point{x_nm:-2_000_000,y_nm:-1_000_000}, end: Point{x_nm:2_000_000,y_nm:1_000_000}, width_nm:100_000, fill:false, layer:"x".into() }],
            pins: vec![eda_model::Pin { number:"1".into(), name:"H".into(), position:Point{x_nm:3_000_000,y_nm:0}, visible:false, ..Default::default() }], ..Default::default()
        }], ..Default::default() };
        let out = symbol_lib_many(&[EdaComponent { mpn:"G".into(), symbols:vec![s], ..Default::default() }], "N");
        assert!(out.contains("(rectangle "));
        assert!(out.contains("(pin unspecified line hide "));
        assert!(out.contains("(property \"Reference\" \"U\" (at 0.000000 3.540000 0)"));
        assert!(out.contains("(property \"Value\" \"G\" (at 0.000000 -3.540000 0)"));
        assert!(out.contains("(property \"Footprint\" \"\" (at 0.000000 0.000000 0) (effects (font (size 1.27 1.27)) hide))"));
    }

    #[test]
    fn footprint_emits_all_graphics_and_places_text_outside_bounds() {
        let p = Package { name:"G".into(), pads:vec![eda_model::Pad { number:"1".into(), position:Point{x_nm:0,y_nm:0}, size:Point{x_nm:1_000_000,y_nm:1_000_000}, shape:"circle".into(), ..Default::default() }], graphics:vec![
            Graphic::Line { start:Point{x_nm:-1_000_000,y_nm:-1_000_000}, end:Point{x_nm:1_000_000,y_nm:-1_000_000}, width_nm:100_000, layer:"TOP_SILKSCREEN".into() },
            Graphic::Arc { center:Point{x_nm:0,y_nm:0}, start:Point{x_nm:1_000_000,y_nm:0}, end:Point{x_nm:0,y_nm:1_000_000}, width_nm:100_000, layer:"TOP_SILKSCREEN".into() },
            Graphic::Circle { center:Point{x_nm:0,y_nm:0}, radius_nm:1_000_000, width_nm:100_000, layer:"TOP_SILKSCREEN".into() },
            Graphic::Rectangle { start:Point{x_nm:-1_000_000,y_nm:-1_000_000}, end:Point{x_nm:1_000_000,y_nm:1_000_000}, width_nm:100_000, fill:false, layer:"TOP_SILKSCREEN".into() },
            Graphic::Polygon { points:vec![Point{x_nm:0,y_nm:0},Point{x_nm:1_000_000,y_nm:0}], width_nm:100_000, fill:false, layer:"TOP_SILKSCREEN".into() },
            Graphic::Text { text:"mark".into(), position:Point{x_nm:0,y_nm:0}, rotation_mdeg:0, height_nm:1_000_000, width_nm:1_000_000, layer:"TOP_SILKSCREEN".into() },
        ], ..Default::default() };
        let out = footprint(&EdaComponent::default(), &p);
        for token in ["(fp_line ","(fp_arc ","(fp_circle ","(fp_rect ","(fp_poly ","(fp_text user "] { assert!(out.contains(token), "missing {token}"); }
        assert!(out.contains("(fp_text reference \"REF**\" (at 0 3"));
        assert!(out.contains("(fp_text value \"G\" (at 0 -3"));
    }
}
