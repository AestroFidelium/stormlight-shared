//! Invariants of UI adoption (`AdoptedVisuals::adopt`, stormlight/server#67) —
//! the point where a cosmetic mod's declared widget trees are accepted or refused.
//! Directional/structural, total over hostile input:
//!   - **A broken tree is refused, never drawn**: adoption errs exactly when some
//!     declared root fails `UiRoot::validate`, and never panics. This is the last
//!     place a break can be reported *with a reason*; past it the tree reaches a
//!     renderer that would draw a blank rectangle and say nothing.
//!   - **Author order is preserved**: the roots come back in declaration order,
//!     unmerged and unsorted — a HUD is drawn back-to-front in the order its author
//!     wrote it, so reordering them here would reorder the interface.
//!   - **Declaring a HUD is independent of dressing a unit**: a bundle that ships
//!     only widget trees still adopts, and is not "empty".

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::ui::{
    Layout, RootVisibility, Style, TextSource, UiRoot, UiSubject, Widget, WidgetKind,
};
use stormlight_mod_abi::visuals::ClientRegistration;
use stormlight_modloader::client::AdoptedVisuals;

/// How each generated root is broken, if at all. One break per root, chosen from
/// the classes `validate` judges — the classification is `ui_validation`'s
/// business; what is pinned here is that adoption *propagates* it.
#[derive(Debug, TypeGenerator, Clone, Copy, PartialEq, Eq)]
enum Break {
    /// Well-formed.
    None,
    /// No name, so nothing can name it in a diagnostic.
    Unnamed,
    /// Reads the hovered unit without being gated on a hover.
    Unreachable,
    /// An icon that names no picture.
    Pictureless,
}

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// One root per entry, broken the way it says.
    roots: Vec<Break>,
}

fn a_root(i: usize, brk: Break) -> UiRoot {
    let widget = |kind| Widget {
        name: String::new(),
        layout: Layout::default(),
        style: Style::default(),
        kind,
    };
    let mut root = UiRoot {
        name: format!("hud{i}"),
        when: RootVisibility::Always,
        subject: UiSubject::LocalPlayer,
        root: widget(WidgetKind::Text { text: TextSource::Literal("hp".into()) }),
    };
    match brk {
        Break::None => {}
        Break::Unnamed => root.name = String::new(),
        Break::Unreachable => root.subject = UiSubject::HoveredUnit,
        Break::Pictureless => root.root = widget(WidgetKind::Icon),
    }
    root
}

fn build(s: &Scenario) -> ClientRegistration {
    ClientRegistration {
        abi: ABI_VERSION,
        names: Names::default(),
        ui: s.roots.iter().enumerate().map(|(i, brk)| a_root(i, *brk)).collect(),
        ..ClientRegistration::default()
    }
}

#[test]
fn a_broken_tree_is_refused_and_a_sound_one_is_kept_in_author_order() {
    check!().with_type::<Scenario>().for_each(|s| {
        let reg = build(s);
        let any_broken = s.roots.iter().any(|b| *b != Break::None);
        let adopted = AdoptedVisuals::adopt(&reg, "pack");
        assert_eq!(adopted.is_err(), any_broken, "adoption disagrees with tree soundness");

        if let Ok(adopted) = adopted {
            assert_eq!(
                adopted.ui().to_vec(),
                reg.ui,
                "adopted roots must be exactly what was declared, in that order",
            );
        }
    });
}

#[test]
fn a_bundle_that_only_ships_an_interface_still_adopts() {
    check!().with_type::<u8>().for_each(|&count| {
        // Nothing dressed, nothing animated — a mod may ship a HUD alone, and
        // "declares nothing" must not be true of it or the host would treat a
        // pure-interface package as an empty one.
        let roots = usize::from(count % 4);
        let reg = ClientRegistration {
            abi: ABI_VERSION,
            ui: (0..roots).map(|i| a_root(i, Break::None)).collect(),
            ..ClientRegistration::default()
        };
        let adopted = AdoptedVisuals::adopt(&reg, "pack").expect("sound trees adopt");
        assert_eq!(adopted.ui().len(), roots);
        assert_eq!(
            adopted.is_empty(),
            roots == 0,
            "a bundle declaring only widget trees is not an empty bundle",
        );
    });
}
