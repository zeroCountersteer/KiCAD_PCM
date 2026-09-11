use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BxlDocument {
    pub version: Option<String>,
    pub records: Vec<BxlRecord>,
    pub raw_text: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BxlRecord {
    pub kind: String,
    pub fields: Vec<String>,
    pub offset: usize,
}
