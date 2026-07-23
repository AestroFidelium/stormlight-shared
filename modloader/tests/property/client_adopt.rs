//! Invariants of cosmetic-mod adoption (`AdoptedVisuals::adopt`) — turning a
//! decoded `ClientRegistration` into a unit-name → visual table the client keys
//! on. Directional/structural, total over hostile input:
//!   - **Dangling handle is rejected, never panics**: adoption errs exactly when
//!     some visual references a unit handle with no entry in the `names.units`
//!     table (a cosmetic mod naming a unit that was never interned).
//!   - **Every declared visual is reachable**: on success, each visual's unit
//!     name resolves back to a model in the table.

use bolero::{TypeGenerator, check};
use stormlight_mod_abi::descriptors::Names;
use stormlight_mod_abi::ids::UnitId;
use stormlight_mod_abi::manifest::ABI_VERSION;
use stormlight_mod_abi::visuals::{
    ClientRegistration, PrimitiveShape, VisualDescriptor, VisualModel,
};
use stormlight_modloader::client::AdoptedVisuals;

#[derive(Debug, TypeGenerator)]
struct Scenario {
    /// How many distinct unit names the cosmetic mod interned.
    names_len: u8,
    /// One raw unit handle per visual it declares (mapped near the name range so
    /// both in-range and just-past-the-end handles are exercised).
    handles: Vec<u16>,
}

fn build(s: &Scenario) -> (ClientRegistration, Vec<usize>) {
    // Distinct names `u0..u{n-1}` so name-keying is unambiguous.
    let units: Vec<String> = (0..s.names_len).map(|i| format!("u{i}")).collect();
    // Fold each raw handle into `[0, names_len + 1]` so we see valid indices and
    // one-past-the-end dangling references with good frequency.
    let ceil = u16::from(s.names_len).saturating_add(2).max(1);
    let idx: Vec<usize> = s.handles.iter().map(|h| usize::from(h % ceil)).collect();
    let visuals = idx
        .iter()
        .map(|&i| VisualDescriptor {
            unit: UnitId(i as u32),
            model: VisualModel::Primitive { shape: PrimitiveShape::Cube, color: [0.0; 4] },
        })
        .collect();
    let reg = ClientRegistration {
        abi: ABI_VERSION,
        names: Names { units, ..Names::default() },
        visuals,
        effects: Vec::new(),
    };
    (reg, idx)
}

#[test]
fn a_dangling_unit_handle_is_rejected_not_panicked() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, idx) = build(s);
        let any_dangling = idx.iter().any(|&i| i >= usize::from(s.names_len));
        let adopted = AdoptedVisuals::adopt(&reg);
        assert_eq!(
            adopted.is_err(),
            any_dangling,
            "adoption error disagrees with dangling-handle presence",
        );
    });
}

#[test]
fn every_adopted_visual_is_reachable_by_unit_name() {
    check!().with_type::<Scenario>().for_each(|s| {
        let (reg, idx) = build(s);
        if let Ok(adopted) = AdoptedVisuals::adopt(&reg) {
            for &i in &idx {
                let name = format!("u{i}");
                assert!(
                    adopted.get(&name).is_some(),
                    "declared visual for `{name}` is not reachable after adoption",
                );
            }
        }
    });
}
