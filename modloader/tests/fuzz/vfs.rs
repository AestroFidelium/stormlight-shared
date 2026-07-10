//! `mod://` URLs come from untrusted mods, so parsing must be total and — the
//! security-critical part — a URL that *parses* can never escape the mod root.
//! Laws over any input:
//!   - `parse_mod_url` never panics.
//!   - a successful parse yields no `.`/`..`/empty component and no backslash.
//!   - joining the parsed path under any root stays under that root.

use std::path::Path;

use bolero::check;
use stormlight_modloader::vfs::parse_mod_url;

#[test]
fn parsing_is_total_and_never_escapes_the_root() {
    check!().with_type::<String>().for_each(|url| {
        let Ok(resource) = parse_mod_url(url) else { return };

        assert!(!resource.id.is_empty(), "accepted an empty id");
        assert!(!resource.path.contains('\\'), "accepted a backslash");
        for component in resource.path.split('/') {
            assert!(
                !matches!(component, "" | "." | ".."),
                "accepted a traversal component in {:?}",
                resource.path
            );
        }
        // The resolved path stays within any package root.
        for root in [Path::new("/mods/pkg"), Path::new("relative/root")] {
            assert!(
                root.join(&resource.path).starts_with(root),
                "path {:?} escaped root {:?}",
                resource.path,
                root
            );
        }
    });
}
