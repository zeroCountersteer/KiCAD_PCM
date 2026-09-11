use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BBox {
    pub xmin_nm: i64,
    pub xmax_nm: i64,
    pub ymin_nm: i64,
    pub ymax_nm: i64,
    pub zmin_nm: i64,
    pub zmax_nm: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceModel {
    pub sha256: String,
    pub original_filename: String,
    pub manufacturer: String,
    pub source_url: Option<String>,
    pub associated_mpns: Vec<String>,
    pub source_package_names: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalModel {
    pub id: String,
    pub source: SourceModel,
    pub units: String,
    pub bbox: Option<BBox>,
    pub geometry_fingerprint: String,
    pub confidence: String,
    pub rotation_mdeg: i32,
    pub offset_nm: [i64; 3],
}

fn unit_scale(s: &str) -> Option<f64> {
    let x = s.to_ascii_lowercase();
    if x.contains("millimetre") || x.contains("millimeter") || x.contains("mm") {
        Some(1_000_000.)
    } else if x.contains("micrometre") || x.contains("micrometer") || x.contains("um") {
        Some(1_000.)
    } else if x.contains("inch") {
        Some(25_400_000.)
    } else {
        None
    }
}
fn num(s: &str) -> Option<f64> {
    s.trim_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+')
        .parse()
        .ok()
}
pub fn parse(bytes: &[u8], source: SourceModel) -> Result<CanonicalModel, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "STEP is not UTF-8 text".to_string())?;
    if !text.contains("ISO-10303") && !text.contains("HEADER;") {
        return Err("missing STEP header".into());
    }
    let units = text
        .lines()
        .find(|l| l.contains("NAMED_UNIT") || l.contains("SI_UNIT"))
        .and_then(|l| l.split('"').nth(1))
        .unwrap_or("unknown")
        .to_string();
    let scale = unit_scale(&units).unwrap_or(1_000_000.);
    let mut points = Vec::new();
    for l in text.lines() {
        if l.contains("CARTESIAN_POINT") {
            let vals = l
                .split('(')
                .next_back()
                .unwrap_or("")
                .split(')')
                .next()
                .unwrap_or("")
                .split(',')
                .filter_map(num)
                .collect::<Vec<_>>();
            if vals.len() >= 3 {
                points.push([vals[0] * scale, vals[1] * scale, vals[2] * scale]);
            }
        }
    }
    let bbox = if points.is_empty() {
        None
    } else {
        Some(BBox {
            xmin_nm: points.iter().map(|p| p[0] as i64).min().unwrap(),
            xmax_nm: points.iter().map(|p| p[0] as i64).max().unwrap(),
            ymin_nm: points.iter().map(|p| p[1] as i64).min().unwrap(),
            ymax_nm: points.iter().map(|p| p[1] as i64).max().unwrap(),
            zmin_nm: points.iter().map(|p| p[2] as i64).min().unwrap(),
            zmax_nm: points.iter().map(|p| p[2] as i64).max().unwrap(),
        })
    };
    let fp = format!("{:x}", Sha256::digest(serde_json_like(&points).as_bytes()));
    let id = fp.clone();
    Ok(CanonicalModel {
        id,
        source,
        units,
        bbox,
        geometry_fingerprint: fp,
        confidence: "Plausible".into(),
        rotation_mdeg: 0,
        offset_nm: [0, 0, 0],
    })
}
fn serde_json_like(p: &[[f64; 3]]) -> String {
    p.iter()
        .map(|x| format!("{:.6},{:.6},{:.6};", x[0], x[1], x[2]))
        .collect()
}
pub fn model_filename(name: &str) -> String {
    let s = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{s}.step")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn src() -> SourceModel {
        SourceModel {
            sha256: "x".into(),
            original_filename: "x.step".into(),
            manufacturer: "TI".into(),
            source_url: None,
            associated_mpns: vec![],
            source_package_names: vec![],
        }
    }
    #[test]
    fn parses_mm_bbox() {
        let b=b"ISO-10303-21;\n#1=CARTESIAN_POINT('',(0.,0.,0.));\n#2=CARTESIAN_POINT('',(1.,2.,3.));\n";
        let m = parse(b, src()).unwrap();
        assert_eq!(m.bbox.unwrap().xmax_nm, 1_000_000)
    }
    #[test]
    fn names_stable() {
        assert_eq!(model_filename("QFN/32"), "QFN_32.step")
    }
}
