/// Build input shared by the desktop bookmark catalog and the core prompt assets.
pub struct InternalDomain(String);

impl InternalDomain {
    pub const ENV: &str = "GOLD_BAND_WB_INTERNAL_DOMAIN";
    const TOKEN: &str = "${INTERNAL_DOMAIN}";

    pub fn from_env() -> Result<Self, String> {
        let raw = std::env::var(Self::ENV).map_err(|_| format!("missing {}", Self::ENV))?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        // Reserve room for the longest service prefix in the bookmark catalog.
        let valid = raw.len() + "uat.aompplus.".len() <= 253
            && raw
                .rsplit_once('.')
                .is_some_and(|(_, tld)| tld.starts_with(|c: char| c.is_ascii_alphabetic()))
            && raw.split('.').all(|label| {
                (1..=63).contains(&label.len())
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            });
        if !valid {
            return Err(format!(
                "invalid {}: expected a bare DNS domain suffix",
                Self::ENV
            ));
        }
        Ok(Self(raw.to_ascii_lowercase()))
    }

    pub fn render(&self, text: &str) -> String {
        text.replace(Self::TOKEN, &self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_domain_accepts_dns_suffixes_without_url_or_shell_syntax() {
        let domain = InternalDomain::parse("Rd.Corp-2.Example").unwrap();
        assert_eq!(
            domain.render("http://wnpm.${INTERNAL_DOMAIN}:8001/x"),
            "http://wnpm.rd.corp-2.example:8001/x"
        );
        for value in [
            "",
            "corp",
            "127.0.0.1",
            "corp..invalid",
            "corp.invalid.",
            "user@corp.invalid",
            "https://corp.invalid",
            "corp.invalid:80",
            "corp.invalid/path",
            "corp.invalid#x",
            "corp.invalid\n",
            "corp.invalid;whoami",
            "-bad.invalid",
            "bad-.invalid",
            &format!("{}.invalid", "a".repeat(64)),
        ] {
            assert_eq!(
                InternalDomain::parse(value).err().unwrap(),
                format!(
                    "invalid {}: expected a bare DNS domain suffix",
                    InternalDomain::ENV
                )
            );
        }
    }
}
