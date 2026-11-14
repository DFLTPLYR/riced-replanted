//! Lucide icon extension: name -> glyph (PUA char) and charset lookup.
//!
//! The host renders the resolved glyph natively via `iced.icon(glyph, {...})`
//! with the bundled `lucide` font family. This process owns no font bytes;
//! it owns the `lucide-icons` lookup, so scripts can validate names or fetch
//! the unicode codepoint out of process. The companion lucide.lua owns iced.lucide.

use lucide_icons::Icon;
use riced_extension_sdk::Plugin;
use serde_json::{Value, json};

/// Every lucide icon name, one per line, sorted. Regenerate with
/// `just lucide-icons` after a `lucide-icons` dependency bump.
const ICONS: &str = include_str!("../icons.txt");

fn main() -> std::io::Result<()> {
    Plugin::new("lucide")
        .state(["count"])
        .method("resolve", true, |args| {
            let name = args
                .get(0)
                .and_then(Value::as_str)
                .ok_or_else(|| "resolve requires a string icon name".to_owned())?;
            if name.is_empty() {
                return Err("icon name must not be empty".to_owned());
            }
            let icon = Icon::try_from(name).map_err(|error: String| error)?;
            Ok(json!({
                "name": name,
                "char": char::from(icon).to_string(),
                "unicode": format!("U+{:04X}", char::from(icon) as u32),
            }))
        })
        .method("exists", true, |args| {
            let name = args.get(0).and_then(Value::as_str).unwrap_or("");
            Ok(json!(Icon::try_from(name).is_ok()))
        })
        .method("list", true, |args| {
            let prefix = args.get(0).and_then(Value::as_str).unwrap_or("");
            Ok(json!(
                ICONS
                    .lines()
                    .filter(|name| name.starts_with(prefix))
                    .take(512)
                    .collect::<Vec<_>>()
            ))
        })
        .serve(|context| {
            // Static catalog: publish once; the host never subscribes in
            // practice, but the SDK requires a state write to stay alive.
            context.publish(json!({"count": 1}));
            std::thread::park();
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_icon_resolves() {
        let names = ICONS.lines().collect::<Vec<_>>();
        assert!(names.len() > 2000, "{}", names.len());
        assert!(
            names.windows(2).all(|pair| pair[0] < pair[1]),
            "the name list must be sorted without duplicates"
        );
        for name in &names {
            assert!(Icon::try_from(*name).is_ok(), "{name}");
        }
    }

    #[test]
    fn resolves_known_icons() {
        let icon = Icon::try_from("activity").unwrap();
        assert_eq!(char::from(icon).to_string().len(), 3);
        assert!(Icon::try_from("not-a-lucide-icon").is_err());
    }
}
