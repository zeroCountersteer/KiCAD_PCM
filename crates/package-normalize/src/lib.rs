use eda_model::{EdaComponent, Graphic, Package, Pad, Point};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedPad {
    pub number: String,
    pub kind: String,
    pub shape: String,
    pub x_nm: i64,
    pub y_nm: i64,
    pub width_nm: i64,
    pub height_nm: i64,
    pub rotation_mdeg: i32,
    pub drill: Option<Point>,
    pub plated: Option<bool>,
    pub layers: Vec<String>,
    pub mask_nm: Option<i64>,
    pub paste: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedGraphic {
    pub kind: String,
    pub layer: String,
    pub data: Vec<i64>,
    pub text: Option<String>,
    #[serde(default)]
    pub width_nm: i64,
    #[serde(default)]
    pub height_nm: i64,
    #[serde(default)]
    pub fill: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Bounds {
    pub min_x_nm: i64,
    pub min_y_nm: i64,
    pub max_x_nm: i64,
    pub max_y_nm: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fingerprints {
    pub physical_geometry_hash: String,
    pub electrical_geometry_hash: String,
    pub manufacturing_hash: String,
    pub full_geometry_hash: String,
    /// Identity of the bytes-relevant geometry emitted by the KiCad writer.
    #[serde(default)]
    pub kicad_footprint_hash: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourcePackage {
    pub manufacturer: String,
    pub mpn: String,
    pub name: String,
    pub source_sha256: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedPackage {
    pub source: SourcePackage,
    pub pads: Vec<NormalizedPad>,
    pub graphics: Vec<NormalizedGraphic>,
    pub bounds: Bounds,
    pub fingerprints: Fingerprints,
    pub warnings: Vec<String>,
    pub rotation_mdeg: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalPackage {
    pub id: String,
    pub preferred_name: String,
    pub aliases: Vec<String>,
    pub sources: Vec<SourcePackage>,
    pub representative: NormalizedPackage,
    pub fingerprints: Fingerprints,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DedupeResult {
    pub packages: Vec<NormalizedPackage>,
    pub canonical: Vec<CanonicalPackage>,
    pub physical_groups: BTreeMap<String, Vec<String>>,
    pub manufacturing_groups: BTreeMap<String, Vec<String>>,
    pub warnings: Vec<String>,
}

fn rotate(x: i64, y: i64, r: i32) -> (i64, i64) {
    match r {
        0 => (x, y),
        90000 => (-y, x),
        180000 => (-x, -y),
        _ => (y, -x),
    }
}
fn bounds(pads: &[NormalizedPad], graphics: &[NormalizedGraphic]) -> Bounds {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for p in pads {
        let quarter_turn = p.rotation_mdeg.rem_euclid(180000) == 90000;
        let (width, height) = if quarter_turn {
            (p.height_nm, p.width_nm)
        } else {
            (p.width_nm, p.height_nm)
        };
        xs.extend([p.x_nm - width / 2, p.x_nm + width / 2]);
        ys.extend([p.y_nm - height / 2, p.y_nm + height / 2]);
    }
    for g in graphics {
        for xy in g.data.chunks(2) {
            if xy.len() == 2 {
                xs.push(xy[0]);
                ys.push(xy[1]);
            }
        }
    }
    if xs.is_empty() {
        return Bounds::default();
    }
    Bounds {
        min_x_nm: *xs.iter().min().unwrap(),
        min_y_nm: *ys.iter().min().unwrap(),
        max_x_nm: *xs.iter().max().unwrap(),
        max_y_nm: *ys.iter().max().unwrap(),
    }
}
fn graphics(p: &Package) -> Vec<NormalizedGraphic> {
    p.graphics
        .iter()
        .map(|g| match g {
            Graphic::Line {
                start,
                end,
                width_nm,
                layer,
            } => NormalizedGraphic {
                kind: "line".into(),
                layer: layer.clone(),
                data: vec![start.x_nm, start.y_nm, end.x_nm, end.y_nm],
                text: None,
                width_nm: *width_nm,
                height_nm: 0,
                fill: false,
            },
            Graphic::Circle {
                center,
                radius_nm,
                width_nm,
                layer,
            } => NormalizedGraphic {
                kind: "circle".into(),
                layer: layer.clone(),
                data: vec![center.x_nm, center.y_nm, *radius_nm],
                text: None,
                width_nm: *width_nm,
                height_nm: 0,
                fill: false,
            },
            Graphic::Rectangle {
                start, end, layer, fill, width_nm
            } => NormalizedGraphic {
                kind: "rectangle".into(),
                layer: layer.clone(),
                data: vec![start.x_nm, start.y_nm, end.x_nm, end.y_nm],
                text: None,
                width_nm: *width_nm,
                height_nm: 0,
                fill: *fill,
            },
            Graphic::Polygon { points, layer, fill, width_nm, .. } => NormalizedGraphic {
                kind: "polygon".into(),
                layer: layer.clone(),
                data: points.iter().flat_map(|x| [x.x_nm, x.y_nm]).collect(),
                text: None,
                width_nm: *width_nm,
                height_nm: 0,
                fill: *fill,
            },
            Graphic::Arc {
                center,
                start,
                end,
                layer,
                width_nm,
            } => NormalizedGraphic {
                kind: "arc".into(),
                layer: layer.clone(),
                data: vec![
                    center.x_nm,
                    center.y_nm,
                    start.x_nm,
                    start.y_nm,
                    end.x_nm,
                    end.y_nm,
                ],
                text: None,
                width_nm: *width_nm,
                height_nm: 0,
                fill: false,
            },
            Graphic::Text {
                text,
                position,
                layer,
                height_nm,
                width_nm,
                ..
            } => NormalizedGraphic {
                kind: "text".into(),
                layer: layer.clone(),
                data: vec![position.x_nm, position.y_nm],
                text: Some(text.clone()),
                width_nm: *width_nm,
                height_nm: *height_nm,
                fill: false,
            },
        })
        .collect()
}
fn pad(p: &Pad) -> NormalizedPad {
    NormalizedPad {
        number: p.number.clone(),
        kind: if p.drill.is_some() {
            if p.plated == Some(false) {
                "np_thru_hole"
            } else if p.plated == Some(true) {
                "thru_hole"
            } else {
                "unknown_drill_plating"
            }
        } else {
            "smd"
        }
        .into(),
        shape: p.shape.to_ascii_lowercase(),
        x_nm: p.position.x_nm,
        y_nm: p.position.y_nm,
        width_nm: p.size.x_nm,
        height_nm: p.size.y_nm,
        rotation_mdeg: p.rotation_mdeg,
        drill: p.drill.clone(),
        plated: p.plated,
        layers: p.layers.clone(),
        mask_nm: p.solder_mask_expansion_nm,
        paste: p.paste,
    }
}
fn hash<T: Serialize>(v: &T) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(v).unwrap()))
}
fn transformed(
    mut pads: Vec<NormalizedPad>,
    mut gs: Vec<NormalizedGraphic>,
    r: i32,
    include_graphics: bool,
) -> (Vec<NormalizedPad>, Vec<NormalizedGraphic>, Bounds) {
    let b = bounds(&pads, if include_graphics { &gs } else { &[] });
    let ox = (b.min_x_nm + b.max_x_nm) / 2;
    let oy = (b.min_y_nm + b.max_y_nm) / 2;
    for p in &mut pads {
        let (x, y) = rotate(p.x_nm - ox, p.y_nm - oy, r);
        p.x_nm = x;
        p.y_nm = y;
        p.rotation_mdeg = (p.rotation_mdeg + r).rem_euclid(360000);
    }
    for g in &mut gs {
        for xy in g.data.chunks_mut(2).filter(|x| x.len() == 2) {
            let (x, y) = rotate(xy[0] - ox, xy[1] - oy, r);
            xy[0] = x;
            xy[1] = y;
        }
    }
    let b = bounds(&pads, if include_graphics { &gs } else { &[] });
    (pads, gs, b)
}
fn key(pads: &[NormalizedPad], gs: &[NormalizedGraphic], level: u8) -> String {
    let mut ps = pads.to_vec();
    let mut gg = gs.to_vec();
    for p in &mut ps {
        if p.width_nm == p.height_nm {
            p.rotation_mdeg = 0;
        }
        if level == 1 {
            p.number.clear();
            p.layers.clear();
            p.mask_nm = None;
            p.paste = None;
        } else if level == 2 {
            p.layers.clear();
            p.mask_nm = None;
            p.paste = None;
        } else if level == 3 {
        }
    }
    ps.sort_by_key(|p| serde_json::to_string(p).unwrap());
    if level < 4 {
        gg.clear();
    } else {
        gg.sort_by_key(|g| {
            (
                g.kind.clone(),
                g.layer.clone(),
                g.data.clone(),
                g.text.clone(),
            )
        });
    }
    serde_json::to_string(&(ps, gg)).unwrap()
}
fn kicad_layer(layer: &str) -> String {
    match layer.to_ascii_uppercase().as_str() {
        "TOP_SILKSCREEN" => "F.SilkS",
        "TOP_ASSEMBLY" => "F.Fab",
        "TOP_COPPER" | "TOP" => "F.Cu",
        "TOP_SOLDER_MASK" => "F.Mask",
        "TOP_PASTE" => "F.Paste",
        "BOTTOM_SILKSCREEN" => "B.SilkS",
        "BOTTOM_ASSEMBLY" => "B.Fab",
        "BOTTOM_COPPER" | "BOTTOM" => "B.Cu",
        _ => layer,
    }
    .into()
}
fn kicad_projection(pads: &[NormalizedPad], graphics: &[NormalizedGraphic]) -> serde_json::Value {
    let pad_projection = pads
        .iter()
        .filter(|p| p.kind != "unknown_drill_plating")
        .map(|p| {
            let shape = match p.shape.as_str() {
                "circle" => "circle",
                "oval" => "oval",
                "roundrect" => "roundrect",
                "rectangle" | "rect" => "rect",
                other => other,
            };
            let layers = if p.layers.is_empty() {
                if p.kind == "smd" {
                    vec!["F.Cu".into(), "F.Paste".into(), "F.Mask".into()]
                } else {
                    vec!["*.Cu".into(), "*.Mask".into()]
                }
            } else {
                p.layers.iter().map(|x| kicad_layer(x)).collect()
            };
            serde_json::json!({
                "number": p.number, "kind": p.kind, "shape": shape,
                "x": p.x_nm, "y": p.y_nm, "rotation": p.rotation_mdeg,
                "width": p.width_nm, "height": p.height_nm, "layers": layers,
                "drill": p.drill.as_ref().map(|d| d.x_nm),
            })
        })
        .collect::<Vec<_>>();
    let graphic_projection = graphics
        .iter()
        .map(|g| serde_json::json!({"kind": g.kind, "data": g.data, "width": g.width_nm, "height": g.height_nm, "fill": g.fill, "text": g.text, "layer": kicad_layer(&g.layer)}))
        .collect::<Vec<_>>();
    let physical_graphics = graphics.iter().filter(|g| {
        matches!(g.layer.to_ascii_uppercase().as_str(), "TOP_ASSEMBLY" | "BOTTOM_ASSEMBLY") && g.kind != "text"
    }).cloned().collect::<Vec<_>>();
    let b = bounds(pads, &physical_graphics);
    let clearance = 250_000i64;
    let floor_grid = |n: i64| n.div_euclid(10_000) * 10_000;
    let ceil_grid = |n: i64| (n + 9_999).div_euclid(10_000) * 10_000;
    let courtyard = [
        floor_grid(b.min_x_nm - clearance),
        floor_grid(b.min_y_nm - clearance),
        ceil_grid(b.max_x_nm + clearance),
        ceil_grid(b.max_y_nm + clearance),
    ];
    serde_json::json!({"pads": pad_projection, "graphics": graphic_projection, "courtyard": courtyard})
}
fn kicad_groups(packages: &[NormalizedPackage]) -> BTreeMap<String, Vec<String>> {
    let mut groups = BTreeMap::new();
    for p in packages {
        if let Some(hash) = &p.fingerprints.kicad_footprint_hash {
            groups
                .entry(hash.clone())
                .or_insert_with(Vec::new)
                .push(format!("{}:{}", p.source.mpn, p.source.name));
        }
    }
    groups
}
pub fn normalize_package(c: &EdaComponent, p: &Package) -> NormalizedPackage {
    let raw = p.pads.iter().map(pad).collect::<Vec<_>>();
    let rawg = graphics(p);
    let mut best = None;
    for r in [0, 90000, 180000, 270000] {
        let (a, b, bx) = transformed(raw.clone(), rawg.clone(), r, true);
        let k = key(&a, &b, 4);
        if best.as_ref().is_none_or(|x: &(String, _, _, _, _)| k < x.0) {
            best = Some((k, a, b, bx, r));
        }
    }
    let (_, pads, graphics, bounds, rotation_mdeg) = best.unwrap();
    let fingerprint = |level: u8| {
        let mut best = None;
        for r in [0, 90000, 180000, 270000] {
            let (p, g, _) = transformed(raw.clone(), rawg.clone(), r, level == 4);
            let k = key(&p, &g, level);
            if best.as_ref().is_none_or(|x: &(String, _, _)| k < x.0) {
                best = Some((k, p, g));
            }
        }
        best.unwrap().0
    };
    let fingerprints = Fingerprints {
        physical_geometry_hash: hash(&fingerprint(1)),
        electrical_geometry_hash: hash(&fingerprint(2)),
        manufacturing_hash: hash(&fingerprint(3)),
        full_geometry_hash: hash(&fingerprint(4)),
        kicad_footprint_hash: match eda_model::footprint_eligibility(p) {
            eda_model::FootprintEligibility::Eligible => {
                Some(hash(&kicad_projection(&pads, &graphics)))
            }
            _ => None,
        },
    };
    let mut warnings = Vec::new();
    let mut seen = BTreeSet::new();
    for x in &pads {
        if x.width_nm <= 0 || x.height_nm <= 0 {
            warnings.push(format!("{}: zero-size pad {}", p.name, x.number));
        }
        if !seen.insert(x.number.clone()) {
            warnings.push(format!("{}: duplicate pad {}", p.name, x.number));
        }
        if x.width_nm > 100_000_000 || x.height_nm > 100_000_000 {
            warnings.push(format!("{}: extreme pad {}", p.name, x.number));
        }
        if let Some(d) = &x.drill {
            if d.x_nm > x.width_nm || d.y_nm > x.height_nm {
                warnings.push(format!("{}: drill larger than pad {}", p.name, x.number));
            }
        }
    }
    NormalizedPackage {
        source: SourcePackage {
            manufacturer: c.manufacturer.clone(),
            mpn: c.mpn.clone(),
            name: p.name.clone(),
            source_sha256: c.source_sha256.clone(),
        },
        pads,
        graphics,
        bounds,
        fingerprints,
        warnings,
        rotation_mdeg,
    }
}
pub fn canonical_name(p: &NormalizedPackage, used: &mut BTreeSet<String>) -> String {
    let base = format!("{}_{}", safe(&p.source.manufacturer), safe(&p.source.name));
    let mut n = base.clone();
    let mut i = 2;
    while !used.insert(n.clone()) {
        n = format!("{base}_{i}");
        i += 1;
    }
    n
}
fn safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
pub fn dedupe(cs: &[EdaComponent]) -> DedupeResult {
    let packages = cs
        .iter()
        .flat_map(|c| c.packages.iter().map(|p| normalize_package(c, p)))
        .collect::<Vec<_>>();
    let mut physical_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut manufacturing_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in &packages {
        physical_groups
            .entry(p.fingerprints.physical_geometry_hash.clone())
            .or_default()
            .push(format!("{}:{}", p.source.mpn, p.source.name));
        manufacturing_groups
            .entry(p.fingerprints.manufacturing_hash.clone())
            .or_default()
            .push(format!("{}:{}", p.source.mpn, p.source.name));
    }
    let mut used = BTreeSet::new();
    let mut canonical = Vec::new();
    for (h, names) in &kicad_groups(&packages) {
        let ix = packages
            .iter()
            .position(|p| p.fingerprints.kicad_footprint_hash.as_deref() == Some(h))
            .unwrap();
        let mut aliases = names.clone();
        aliases.sort();
        canonical.push(CanonicalPackage {
            id: h.clone(),
            preferred_name: canonical_name(&packages[ix], &mut used),
            aliases,
            sources: packages
                .iter()
                .filter(|p| p.fingerprints.kicad_footprint_hash.as_deref() == Some(h))
                .map(|p| p.source.clone())
                .collect(),
            representative: packages[ix].clone(),
            fingerprints: packages[ix].fingerprints.clone(),
        });
    }
    DedupeResult {
        packages,
        canonical,
        physical_groups,
        manufacturing_groups,
        warnings: Vec::new(),
    }
}
pub fn package_diff(a: &NormalizedPackage, b: &NormalizedPackage) -> String {
    format!("{} vs {}\nphysical: {}\nelectrical: {}\nmanufacturing: {}\nfull: {}\npads: {} vs {}\nbounds: {}x{} vs {}x{} nm\nrotation used: {} vs {} degrees",a.source.name,b.source.name,a.fingerprints.physical_geometry_hash==b.fingerprints.physical_geometry_hash,a.fingerprints.electrical_geometry_hash==b.fingerprints.electrical_geometry_hash,a.fingerprints.manufacturing_hash==b.fingerprints.manufacturing_hash,a.fingerprints.full_geometry_hash==b.fingerprints.full_geometry_hash,a.pads.len(),b.pads.len(),a.bounds.max_x_nm-a.bounds.min_x_nm,a.bounds.max_y_nm-a.bounds.min_y_nm,b.bounds.max_x_nm-b.bounds.min_x_nm,b.bounds.max_y_nm-b.bounds.min_y_nm,a.rotation_mdeg/1000,b.rotation_mdeg/1000)
}
pub fn overlay_svg(a: &NormalizedPackage, b: &NormalizedPackage) -> String {
    let minx = a.bounds.min_x_nm.min(b.bounds.min_x_nm) - 500000;
    let miny = a.bounds.min_y_nm.min(b.bounds.min_y_nm) - 500000;
    let maxx = a.bounds.max_x_nm.max(b.bounds.max_x_nm) + 500000;
    let maxy = a.bounds.max_y_nm.max(b.bounds.max_y_nm) + 500000;
    let mut s=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\"><g transform=\"scale(0.000001,-0.000001) translate(0,{})\">",minx,-maxy,maxx-minx,maxy-miny,maxy);
    for (p, col) in [(&a.pads, "red"), (&b.pads, "blue")] {
        for x in p {
            s += &format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" opacity=\".55\"/>",
                x.x_nm - x.width_nm / 2,
                -x.y_nm - x.height_nm / 2,
                x.width_nm,
                x.height_nm,
                col
            );
        }
    }
    s += "</g></svg>";
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(n: &str, x: i64) -> Pad {
        Pad {
            number: n.into(),
            shape: "rect".into(),
            position: Point { x_nm: x, y_nm: 0 },
            size: Point {
                x_nm: 1000,
                y_nm: 1000,
            },
            ..Default::default()
        }
    }
    fn c(ps: Vec<Pad>) -> EdaComponent {
        EdaComponent {
            manufacturer: "TI".into(),
            mpn: "X".into(),
            packages: vec![Package {
                name: "P".into(),
                pads: ps,
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn translation_rotation_order() {
        let a = normalize_package(
            &c(vec![p("1", 0), p("2", 3000)]),
            &c(vec![p("1", 0), p("2", 3000)]).packages[0],
        );
        let mut q = p("2", 0);
        q.position.y_nm = 3000;
        let b = normalize_package(
            &c(vec![p("1", 0), q.clone()]),
            &c(vec![p("1", 0), q]).packages[0],
        );
        assert_eq!(
            a.fingerprints.physical_geometry_hash,
            b.fingerprints.physical_geometry_hash
        );
        let d = normalize_package(
            &c(vec![p("1", 0), p("3", 3000)]),
            &c(vec![p("1", 0), p("3", 3000)]).packages[0],
        );
        assert_ne!(
            a.fingerprints.electrical_geometry_hash,
            d.fingerprints.electrical_geometry_hash
        );
    }
    #[test]
    fn manufacturing_and_graphics_levels_are_distinct() {
        let mut x = p("1", 0);
        x.paste = Some(true);
        let a = normalize_package(&c(vec![x.clone()]), &c(vec![x]).packages[0]);
        let b = normalize_package(&c(vec![p("1", 0)]), &c(vec![p("1", 0)]).packages[0]);
        assert_eq!(
            a.fingerprints.physical_geometry_hash,
            b.fingerprints.physical_geometry_hash
        );
        assert_ne!(
            a.fingerprints.manufacturing_hash,
            b.fingerprints.manufacturing_hash
        );
        let mut pkg = c(vec![p("1", 0)]).packages[0].clone();
        pkg.graphics.push(Graphic::Circle {
            center: Point::default(),
            radius_nm: 100,
            width_nm: 10,
            layer: "TOP_SILKSCREEN".into(),
        });
        let g = normalize_package(&c(vec![p("1", 0)]), &pkg);
        assert_ne!(
            g.fingerprints.full_geometry_hash,
            b.fingerprints.full_geometry_hash
        );
    }

    #[test]
    fn rectangular_pad_rotation_does_not_swap_local_dimensions_or_drill() {
        let p = NormalizedPad {
            number: "1".into(),
            kind: "thru_hole".into(),
            shape: "rect".into(),
            x_nm: 0,
            y_nm: 0,
            width_nm: 1000,
            height_nm: 2000,
            rotation_mdeg: 0,
            drill: Some(Point {
                x_nm: 300,
                y_nm: 500,
            }),
            plated: Some(true),
            layers: Vec::new(),
            mask_nm: None,
            paste: None,
        };
        let (pads, _, _) = transformed(vec![p], Vec::new(), 90000, false);
        assert_eq!((pads[0].width_nm, pads[0].height_nm), (1000, 2000));
        assert_eq!(
            pads[0].drill,
            Some(Point {
                x_nm: 300,
                y_nm: 500
            })
        );
    }

    #[test]
    fn rect_and_rectangle_share_kicad_identity() {
        let mut a = p("1", 0);
        let mut b = a.clone();
        a.shape = "rect".into();
        b.shape = "rectangle".into();
        let ca = normalize_package(&c(vec![a.clone()]), &c(vec![a]).packages[0]);
        let cb = normalize_package(&c(vec![b.clone()]), &c(vec![b]).packages[0]);
        assert_eq!(
            ca.fingerprints.kicad_footprint_hash,
            cb.fingerprints.kicad_footprint_hash
        );
    }

    #[test]
    fn dedupe_uses_kicad_groups_separately_from_manufacturing_groups() {
        let mut a = p("1", 0);
        let mut b = a.clone();
        a.paste = Some(true);
        b.paste = Some(false);
        let mut ca = c(vec![a]);
        let mut cb = c(vec![b]);
        ca.mpn = "A".into();
        cb.mpn = "B".into();
        let result = dedupe(&[ca, cb]);
        assert_eq!(result.canonical.len(), 1);
        assert_eq!(result.manufacturing_groups.len(), 2);
    }

    #[test]
    fn manufacturing_match_does_not_collapse_different_emitted_graphics() {
        let mut a = c(vec![p("1", 0)]);
        let mut b = a.clone();
        a.mpn = "A".into();
        b.mpn = "B".into();
        a.packages[0].graphics.push(Graphic::Line {
            start: Point { x_nm: 0, y_nm: 0 },
            end: Point {
                x_nm: 1000,
                y_nm: 0,
            },
            width_nm: 10,
            layer: "TOP_SILKSCREEN".into(),
        });
        b.packages[0].graphics.push(Graphic::Line {
            start: Point { x_nm: 0, y_nm: 0 },
            end: Point {
                x_nm: 2000,
                y_nm: 0,
            },
            width_nm: 10,
            layer: "TOP_SILKSCREEN".into(),
        });
        let result = dedupe(&[a, b]);
        assert_eq!(result.manufacturing_groups.len(), 1);
        assert_eq!(result.canonical.len(), 2);
    }

    #[test]
    fn unknown_drill_plating_is_not_classified_as_plated() {
        let mut drilled = p("1", 0);
        drilled.drill = Some(Point {
            x_nm: 300,
            y_nm: 300,
        });
        let component = c(vec![drilled]);
        let normalized = normalize_package(&component, &component.packages[0]);
        assert_eq!(normalized.pads[0].kind, "unknown_drill_plating");
    }
}
