pub const RAPIDOCR_WORKER_ARG: &str = "--kivio-rapidocr-worker";

#[derive(Debug, Clone, Copy)]
enum WorkerMode {
    Text,
    Lines,
}

impl WorkerMode {
    #[cfg(not(test))]
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Lines => "lines",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "text" => Ok(Self::Text),
            "lines" => Ok(Self::Lines),
            _ => Err(format!("unsupported RapidOCR worker mode: {value}")),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum WorkerResponse {
    Text(String),
    Lines(Vec<RapidOcrLine>),
    Error(String),
}
