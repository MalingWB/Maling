use serde_json::Value;
use url::Url;

const WB_BASE_URL: &str = "GOLD_BAND_WB_BASE_URL";
const WB_MULTICA_URL: &str = "GOLD_BAND_WB_MULTICA_URL";
const BASE_TOKEN: &str = "${GOLD_BAND_WB_BASE_URL}";
const MULTICA_TOKEN: &str = "${GOLD_BAND_WB_MULTICA_URL}";

pub struct WbUrls {
    base: String,
    multica: String,
}

impl WbUrls {
    pub fn new(base: &str, multica: &str) -> Result<Self, String> {
        Ok(Self {
            base: origin_from(WB_BASE_URL, base)?,
            multica: origin_from(WB_MULTICA_URL, multica)?,
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

/// 校验并归一化构建期注入的 HTTP(S) origin；错误只暴露变量名，不回显值。
fn origin_from(name: &str, raw: &str) -> Result<String, String> {
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
    {
        return Err(invalid_origin(name));
    }
    let url = Url::parse(raw).map_err(|_| invalid_origin(name))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_origin(name));
    }
    Ok(url.origin().ascii_serialization())
}

fn invalid_origin(name: &str) -> String {
    format!(
        "invalid {name}: expected an HTTP(S) origin without credentials, path, query or fragment"
    )
}

pub fn wb_urls() -> Result<WbUrls, String> {
    let base = std::env::var(WB_BASE_URL).map_err(|_| format!("missing {WB_BASE_URL}"))?;
    let multica = std::env::var(WB_MULTICA_URL).map_err(|_| format!("missing {WB_MULTICA_URL}"))?;
    WbUrls::new(&base, &multica)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_wb_channel_and_bookmarks_with_separate_origins() {
        let urls = WbUrls::new(
            "https://example.invalid/",
            "https://multica.example.invalid/",
        )
        .unwrap();
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
        // multica 是独立注入的 origin，不再从服务基础地址派生端口。
        assert_eq!(config["multicaBaseUrl"], "https://multica.example.invalid");
        assert_eq!(config["multicaAppUrl"], "https://multica.example.invalid");
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
            let error = WbUrls::new(raw, "https://multica.example.invalid")
                .err()
                .expect("must reject invalid service origin");
            assert!(error.contains("GOLD_BAND_WB_BASE_URL"));
            assert!(!error.contains("example.invalid"));
        }
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
            let error = WbUrls::new("https://example.invalid", raw)
                .err()
                .expect("must reject invalid multica origin");
            assert!(error.contains("GOLD_BAND_WB_MULTICA_URL"));
            assert!(!error.contains("example.invalid"));
        }
    }
}
