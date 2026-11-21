//! `ideal.diode` and `ideal.switch` are their own family, and the old names still open.
//!
//! The rename in #60 is a taxonomy fix, not a model change: `passive.switch.ideal` declares
//! drain, source and a gate with a `v_gs_th` threshold, plus an intrinsic body diode with its
//! own forward drop and breakdown. That is a gate-driven active device filed next to the
//! resistor. `passive.diode.ideal` is a diode -- passive in the energy sense, but not a lumped
//! R/L/C element and not where a reader looks for it.
//!
//! Why `ideal.*` rather than `semi.diode.ideal`: decision 0001 settled that a component
//! targeting general-simulator's ideal diode/switch must be a *distinct catalog leaf* from
//! `semi.diode`/`semi.mosfet.n`, "not a parameter variant", because folding both into one
//! entry would silently conflate two different physical models. A leaf named
//! `semi.diode.ideal` reads as exactly that variant. Putting the distinction in the group
//! instead keeps `semi.*` free for a real SPICE target to claim.
//!
//! Every assertion below was watched fail before it was trusted: the whole workspace suite
//! stayed green through the rename itself, which is the reason this file exists. Nothing
//! pinned either name, so nothing would have noticed had the rename dropped one.

use gsim_core::compile::library::{catalog, find_component};
use gsim_core::compile::spec::lookup;

/// The six lumped elements, and nothing else. A switch or a diode appearing here again is the
/// regression this file is for.
const LUMPED: [&str; 6] = [
    "passive.capacitor",
    "passive.inductor",
    "passive.inductor.coupled",
    "passive.mutual_inductor",
    "passive.resistor",
    "passive.transformer",
];

#[test]
fn the_ideal_devices_compile_under_their_new_names() {
    for name in ["ideal.diode", "ideal.switch"] {
        assert!(
            lookup(name).is_some(),
            "{name} must be in the registry under its own name"
        );
        assert!(
            find_component(name).is_some(),
            "{name} must have a library entry, so a front end can draw it"
        );
    }
}

#[test]
fn a_sheet_saved_under_the_old_names_still_opens() {
    // The model behind each is unchanged, so the old id is kept as an alias rather than
    // retired. A saved sheet naming it must still resolve to the same TypeSpec.
    for (old, new) in [
        ("passive.diode.ideal", "ideal.diode"),
        ("passive.switch.ideal", "ideal.switch"),
    ] {
        let spec = lookup(old).unwrap_or_else(|| panic!("{old} must still resolve, via its alias"));
        assert_eq!(
            spec.type_name, new,
            "{old} must resolve to {new}, not to something else"
        );
    }
}

#[test]
fn the_legacy_catalogue_spellings_of_both_magnetics_resolve() {
    // #62 gave the transformer and the mutual inductor one spelling each, after the library
    // used a hyphen and the registry an underscore and neither half worked. Both spellings a
    // sheet may carry have to land on the surviving entry.
    for (name, expected) in [
        (
            "physical_electrical.passive.transformer",
            "passive.transformer",
        ),
        (
            "physical-electrical.passive.transformer",
            "passive.transformer",
        ),
        (
            "physical_electrical.passive.mutual_inductor",
            "passive.mutual_inductor",
        ),
        (
            "physical-electrical.passive.mutual_inductor",
            "passive.mutual_inductor",
        ),
    ] {
        let found =
            find_component(name).unwrap_or_else(|| panic!("{name} must resolve to a component"));
        assert_eq!(found.id, expected, "{name} resolved to the wrong component");
        assert!(lookup(name).is_some(), "{name} must also compile");
    }
}

#[test]
fn a_voltage_controlled_switch_is_unknown_rather_than_an_ideal_switch() {
    // `passive.switch.voltage` was an alias of the ideal switch, so the name kept answering
    // after #59 deleted the component. A voltage-controlled switch is not the ideal PWL
    // switch; resolving one to the other is the model conflation 0001 forbids, and it is how
    // #58 turned a clear `UnknownType` into a misleading "port 'sw.d' needs a node".
    // Unknown is the honest answer.
    assert!(
        find_component("passive.switch.voltage").is_none(),
        "passive.switch.voltage must not answer to any library entry"
    );
    assert!(
        lookup("passive.switch.voltage").is_none(),
        "passive.switch.voltage must be UnknownType, not the ideal switch"
    );
}

#[test]
fn passive_holds_exactly_the_six_lumped_elements() {
    let catalog = catalog();
    let mut passives: Vec<&str> = catalog
        .components
        .iter()
        .map(|c| c.component.id.as_str())
        .filter(|id| id.starts_with("passive."))
        .collect();
    passives.sort_unstable();
    assert_eq!(
        passives.as_slice(),
        LUMPED.as_slice(),
        "`passive.` is the lumped R/L/C group; anything else belongs in its own family"
    );
}

#[test]
fn the_ideal_family_is_its_own_palette_group() {
    // The group is what carries the distinction, so it has to reach a front end. The diode
    // used to sit under `semiconductor`, which is the one word that invites reading it as a
    // SPICE model's variant.
    for id in ["ideal.diode", "ideal.switch"] {
        let component = find_component(id).expect("present");
        assert_eq!(
            component.category.as_deref(),
            Some("ideal"),
            "{id} must be grouped as `ideal`, not with the SPICE semiconductors"
        );
        assert!(
            !component.tags.iter().any(|t| t == "passive"),
            "{id} must not be tagged `passive`"
        );
    }
}
