use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[async_trait::async_trait]
pub trait VendorSource: Send + Sync {
    async fn discover_parts(&self) -> anyhow::Result<Vec<PartRecord>>;
    async fn discover_assets(&self, part: &PartRecord) -> anyhow::Result<Vec<AssetRecord>>;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartRecord {
    pub manufacturer: String,
    pub manufacturer_part_number: String,
    pub manufacturer_part_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetRecord {
    pub manufacturer: String,
    pub mpn: String,
    pub part_url: String,
    pub asset_url: String,
    pub discovery_url: Option<String>,
    pub filename: String,
    pub format: String,
    pub content_type: Option<String>,
    #[serde(default)]
    pub source_package: Option<String>,
    #[serde(default)]
    pub request_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestRecord {
    pub manufacturer: String,
    pub mpn: String,
    pub part_url: String,
    pub asset_url: String,
    pub discovery_url: Option<String>,
    pub filename: String,
    pub format: String,
    pub sha256: String,
    pub size: u64,
    pub retrieved_at: DateTime<Utc>,
    pub http_etag: Option<String>,
    pub http_last_modified: Option<String>,
    pub content_type: Option<String>,
    #[serde(default)]
    pub source_package: Option<String>,
    #[serde(default)]
    pub request_url: Option<String>,
    #[serde(default)]
    pub final_download_url: Option<String>,
    #[serde(default)]
    pub content_disposition_filename: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FailureRecord {
    pub manufacturer: String,
    pub mpn: String,
    pub url: String,
    pub stage: String,
    pub http_status: Option<u16>,
    pub error: String,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct EdaComponent {
    pub manufacturer: String,
    pub mpn: String,
    pub aliases: Vec<String>,
    pub symbols: Vec<Symbol>,
    pub packages: Vec<Package>,
    pub models: Vec<ModelRef>,
    pub metadata: std::collections::BTreeMap<String, String>,
    pub source_sha256: Option<String>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub pin_map: Vec<ComponentPinMap>,
    #[serde(default)]
    pub canonicalizer_version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ComponentPinMap {
    pub logical_id: String,
    pub pin_number: String,
    pub pin_name: String,
    pub unit: usize,
    pub symbol_pin_ref: Option<String>,
    pub pad_ref: Option<String>,
    pub electrical_type_raw: Option<String>,
    pub flags: Vec<String>,
    pub explicit: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Point {
    pub x_nm: i64,
    pub y_nm: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Graphic {
    Line {
        start: Point,
        end: Point,
        width_nm: i64,
        layer: String,
    },
    Arc {
        center: Point,
        start: Point,
        end: Point,
        width_nm: i64,
        layer: String,
    },
    Circle {
        center: Point,
        radius_nm: i64,
        width_nm: i64,
        layer: String,
    },
    Rectangle {
        start: Point,
        end: Point,
        width_nm: i64,
        fill: bool,
        layer: String,
    },
    Polygon {
        points: Vec<Point>,
        width_nm: i64,
        fill: bool,
        layer: String,
    },
    Text {
        text: String,
        position: Point,
        rotation_mdeg: i32,
        height_nm: i64,
        width_nm: i64,
        layer: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub units: Vec<SymbolUnit>,
    pub metadata: std::collections::BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SymbolUnit {
    pub name: String,
    pub graphics: Vec<Graphic>,
    pub pins: Vec<Pin>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Pin {
    pub number: String,
    pub name: String,
    pub position: Point,
    pub orientation_mdeg: i32,
    pub length_nm: i64,
    pub electrical_type: ElectricalType,
    pub visible: bool,
    #[serde(default)]
    pub source_semantics: SourcePinSemantics,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SourcePinSemantics {
    pub electrical_type_raw: Option<String>,
    pub shape_raw: Option<String>,
    pub flags: Vec<String>,
    pub logical_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ElectricalType {
    #[default]
    Unknown,
    Input,
    Output,
    Bidirectional,
    PowerInput,
    PowerOutput,
    Passive,
    OpenCollector,
    OpenEmitter,
    NoConnect,
    TriState,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub pads: Vec<Pad>,
    pub graphics: Vec<Graphic>,
    pub metadata: std::collections::BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Pad {
    pub number: String,
    pub name: Option<String>,
    pub shape: String,
    pub position: Point,
    pub size: Point,
    pub rotation_mdeg: i32,
    pub drill: Option<Point>,
    pub plated: Option<bool>,
    pub layers: Vec<String>,
    pub solder_mask_expansion_nm: Option<i64>,
    pub paste: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ModelRef {
    pub filename: String,
    pub sha256: Option<String>,
    pub original_filename: Option<String>,
}

pub fn format_from_filename(name: &str) -> String {
    let lower = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    if lower.ends_with(".zip") {
        return "zip".into();
    }
    if lower.ends_with(".bxl") {
        return "bxl".into();
    }
    for ext in [
        "kicad_sym",
        "kicad_mod",
        "pretty",
        "step",
        "stp",
        "wrl",
        "dxf",
        "brd",
        "sch",
        "pcbdoc",
        "prj",
        "lbr",
    ] {
        if lower.ends_with(&format!(".{ext}")) {
            return ext.into();
        }
    }
    lower.rsplit('.').next().unwrap_or("unknown").to_string()
}

pub fn is_eda_asset(name: &str) -> bool {
    matches!(
        format_from_filename(name).as_str(),
        "bxl"
            | "kicad_sym"
            | "kicad_mod"
            | "pretty"
            | "step"
            | "stp"
            | "wrl"
            | "dxf"
            | "zip"
            | "brd"
            | "sch"
            | "pcbdoc"
            | "lbr"
    )
}
