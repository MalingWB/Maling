#[path = "build_support/internal_domain.rs"]
mod internal_domain;

use internal_domain::InternalDomain;
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=GOLD_BAND_RELEASE_CHANNEL");
    println!("cargo:rerun-if-env-changed={}", InternalDomain::ENV);
    let domain = (env::var("GOLD_BAND_RELEASE_CHANNEL").as_deref() == Ok("wb"))
        .then(InternalDomain::from_env)
        .transpose()
        .unwrap_or_else(|error| panic!("{error}"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"));
    for language in ["zh-CN", "en"] {
        let dest = out.join(language).join("profile");
        fs::create_dir_all(&dest).expect("create generated prompt directory");
        for role in ["cicd.md", "weapm.md"] {
            let path = format!("src/prompts/{language}/profile/{role}");
            println!("cargo:rerun-if-changed={path}");
            let template = fs::read_to_string(&path).expect("read internal role template");
            // Other channels disable both roles; never inject a WB value into their assets.
            let text = domain
                .as_ref()
                .map(|d| d.render(&template))
                .unwrap_or(template);
            fs::write(dest.join(role), text).expect("write generated role prompt");
        }
    }
}
