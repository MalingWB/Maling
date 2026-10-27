use serde_json::Value;
use url::Url;

const WB_BASE_URL: &str = "GOLD_BAND_WB_BASE_URL";
const BASE_TOKEN: &str = "${GOLD_BAND_WB_BASE_URL}";
const MULTICA_TOKEN: &str = "${WB_MULTICA_URL}";

pub struct WbUrls {
    base: String,
    multica: String,
}

impl WbUrls {
    pub fn new(raw: &str, port: u16) -> Result<Self, &'static str> {
        let authority = raw
            .split_once("://")
            .map(|(_, part)| part.strip_suffix('/').unwrap_or(part));
        if raw.trim() != raw
            || authority.is_none_or(|part| {
                part.is_empty()
                    || part
                        .chars()
                        .any(|c| c.is_whitespace() || matches!(c, '/' | '\\' | '?' | '#' | '@'))
            })
            || port == 0
        {
            return Err("invalid GOLD_BAND_WB_BASE_URL or multicaPort");
        }
        let url = Url::parse(raw).map_err(|_| "invalid GOLD_BAND_WB_BASE_URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "invalid GOLD_BAND_WB_BASE_URL: expected HTTP(S) origin without credentials, path, query or fragment",
            );
        }
        let base = url.origin().ascii_serialization();
        let mut multica = url;
        multica
            .set_port(Some(port))
            .map_err(|_| "invalid multicaPort")?;
        Ok(Self {
            base,
            multica: multica.origin().ascii_serialization(),
        })
    }

    pub fn resolve(&self, value: &mut Value) {
        match value {
            Value::String(text) => {
                *text = text
                    .replace(BASE_TOKEN, &self.base)
                    .replace(MULTICA_TOKEN, &self.multica)
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.resolve(item)),
            Value::Object(items) => items.values_mut().for_each(|item| self.resolve(item)),
            _ => {}
        }
    }
}

pub fn wb_urls(config: &Value) -> Result<WbUrls, &'static str> {
    let port = config
        .get("multicaPort")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .ok_or("invalid WB multicaPort")?;
    let raw = std::env::var(WB_BASE_URL).map_err(|_| "missing GOLD_BAND_WB_BASE_URL")?;
    WbUrls::new(&raw, port)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_wb_channel_and_bookmarks_with_one_origin() {
        let urls = WbUrls::new("https://example.invalid/", 5005).unwrap();
        let mut config: Value =
            serde_json::from_str(include_str!("../configs/channels/wb.json")).unwrap();
        let mut bookmarks: Value =
            serde_json::from_str(include_str!("../configs/browser-bookmarks.json")).unwrap();
        urls.resolve(&mut config);
        urls.resolve(&mut bookmarks);
        assert_eq!(
            config["updaterEndpoint"],
            "https://example.invalid/api/file/download/latest.json"
        );
        assert_eq!(
            config["releaseBaseUrl"],
            "https://example.invalid/api/file/download"
        );
        assert_eq!(config["metricsBaseUrl"], "https://example.invalid");
        assert_eq!(config["multicaBaseUrl"], "https://example.invalid:5005");
        assert_eq!(config["multicaAppUrl"], "https://example.invalid:5005");
        assert_eq!(
            config["builtinMcpServers"][0]["transport"]["url"],
            "https://example.invalid/mcp/"
        );
        assert!(
            config["builtinMcpServers"][0]["helpMessage"]
                .as_str()
                .unwrap()
                .contains("https://example.invalid")
        );
        assert_eq!(bookmarks["wb"][0]["url"], "https://example.invalid/");
    }

    #[test]
    fn rejects_invalid_origins_without_disclosing_them() {
        for raw in [
            "",
            "  ",
            "ftp://example.invalid",
            "https://example.invalid/path",
            "https://user:pass@example.invalid",
            "https://@example.invalid",
            "https://example.invalid/a/..",
            "https://example.invalid/?secret=1",
        ] {
            let error = WbUrls::new(raw, 5005)
                .err()
                .expect("must reject invalid origin");
            assert!(error.contains("GOLD_BAND_WB_BASE_URL"));
            assert!(!error.contains("example.invalid"));
        }
    }
}
