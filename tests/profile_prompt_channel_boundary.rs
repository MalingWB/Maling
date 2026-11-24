use std::fs;
use std::path::Path;

use walkdir::WalkDir;

#[test]
fn profile_prompts_do_not_embed_release_channel_identity() {
    let prompts = [
        (
            "zh requirement identity",
            include_str!("../src/prompts/zh-CN/profile/overlays/requirement-identity.md"),
        ),
        (
            "en requirement identity",
            include_str!("../src/prompts/en/profile/overlays/requirement-identity.md"),
        ),
        (
            "zh dev-test auto-commit",
            include_str!("../src/prompts/zh-CN/profile/overlays/dev-test-auto-commit.md"),
        ),
        (
            "en dev-test auto-commit",
            include_str!("../src/prompts/en/profile/overlays/dev-test-auto-commit.md"),
        ),
        (
            "zh cicd",
            include_str!("../src/prompts/zh-CN/profile/cicd.md"),
        ),
        ("en cicd", include_str!("../src/prompts/en/profile/cicd.md")),
    ];

    for (name, prompt) in prompts {
        assert!(
            !prompt.to_ascii_lowercase().contains("wb"),
            "profile prompt `{name}` must remain channel-neutral"
        );
    }
}

#[test]
fn all_prompt_assets_are_channel_neutral() {
    let prompts_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/prompts");
    let mut checked = 0;

    for entry in WalkDir::new(&prompts_root)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.expect("prompt tree should be readable");
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("md")
        {
            continue;
        }

        let content = fs::read_to_string(entry.path()).expect("prompt asset should be UTF-8");
        assert!(
            !entry
                .path()
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains("wb"),
            "prompt asset path `{}` must remain channel-neutral",
            entry.path().display()
        );
        assert!(
            !content.to_ascii_lowercase().contains("wb"),
            "prompt asset `{}` must remain channel-neutral",
            entry.path().display()
        );
        checked += 1;
    }

    assert!(checked > 0, "prompt tree must contain markdown assets");
}

#[test]
fn internal_profiles_embed_only_the_build_channel_domain() {
    use gold_band::{
        channel::{RELEASE_CHANNEL, WB_CHANNEL},
        prompts::{PROFILE_CICD, PROFILE_WEAPM},
    };
    for (cicd, weapm) in [
        (PROFILE_CICD.zh_cn, PROFILE_WEAPM.zh_cn),
        (PROFILE_CICD.en, PROFILE_WEAPM.en),
    ] {
        if RELEASE_CHANNEL == WB_CHANNEL {
            let domain = option_env!("GOLD_BAND_WB_INTERNAL_DOMAIN")
                .expect("required for WB builds")
                .to_ascii_lowercase();
            assert!(cicd.contains(&format!("http://wnpm.{domain}:8001")));
            for url in [
                format!("http://weapm.{domain}"),
                format!("http://docs.{domain}/docs/"),
                format!("http://uat.aiep.{domain}/#/assets/plugin?"),
            ] {
                assert!(weapm.contains(&url));
            }
            assert!(!cicd.contains("${INTERNAL_DOMAIN}") && !weapm.contains("${INTERNAL_DOMAIN}"));
        } else {
            // Disabled roles retain channel-neutral templates, even if the builder has WB secrets.
            assert!(cicd.contains("wnpm.${INTERNAL_DOMAIN}:8001"));
            assert!(weapm.contains("weapm.${INTERNAL_DOMAIN}"));
        }
    }
}
