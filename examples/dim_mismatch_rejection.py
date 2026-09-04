"""Signal-dimension and per-port kind rejection, through a real contract registry.

`examples/port_kind_rejection.py` proved the three rules `connect()` could enforce from the
document alone. Two more needed something the document does not contain: **what ports does
this block's *type* declare, and how wide is each one?** That answer lives in a GSF v2
component contract, and `gsim_core::contract::ContractRegistry` is now how `connect()` reaches
one -- surfaced here as `gsim_py.connect`'s optional sixth argument, a
`{"<block type>": <contract>}` map.

Two rejections are proven end to end, each a distinct keyword-prefixed `ValueError`:

    port-dim-mismatch     a 3-wide constant driving a 1-wide gain input
    port-kind-mismatch    a `signal` wire onto a converter's `physical-electrical` port

Both were *accepted* before this registry existed, and both are still accepted when the
registry is not supplied -- which this script also asserts, because "the fallback is preserved"
is a claim that deserves a test rather than a sentence. `port-compat.ts`'s rules are ported in
`crates/gsim-core/src/document/port_compat.rs`; this file is the end-to-end proof of the two
that needed contracts.

The circuit is a textbook-generic signal chain (a constant, a gain, a signal-to-physical
converter) driving a single resistor to ground. Nothing here is modeled on any real design.

Run it:

    python3 examples/dim_mismatch_rejection.py
"""

import json

import gsim_py

WIDTH = 3
GAIN = 2.0
R = 1000.0

# --------------------------------------------------------------------------------------------
# The contract registry: one GSF v2 contract per block `type` used below.
#
# `dim` is the field that makes `port-dim-mismatch` possible at all. It is a `${{ }}` expression
# like any other, so it resolves against each *block's own* params -- two `control.gain` blocks
# sharing this one contract can have different port widths, which is exactly what the rejection
# below turns on. A port that declares no `dim` (the resistor's, below) stays *unknown*, never
# "assumed scalar", so it never causes a false rejection.
# --------------------------------------------------------------------------------------------

CONTRACTS = {
    "source.const": {
        "name": "source.const",
        "label": "Constant",
        "parameters": [{"name": "value", "type": "array", "default": [0]}],
        "ports": [
            {
                "id": "out",
                "side": "right",
                "kind": "signal",
                "direction": "out",
                "dim": ["${{ count(params.value) }}"],
            }
        ],
        "geometry": {"bbox": [40, 30], "primitives": []},
    },
    "control.gain": {
        "name": "control.gain",
        "label": "Gain",
        "parameters": [
            {"name": "k", "type": "number", "default": 1},
            {"name": "width", "type": "number", "default": 1},
        ],
        "ports": [
            {
                "id": "in",
                "side": "left",
                "kind": "signal",
                "direction": "in",
                "dim": ["${{ params.width }}"],
            },
            {
                "id": "out",
                "side": "right",
                "kind": "signal",
                "direction": "out",
                "dim": ["${{ params.width }}"],
            },
        ],
        "geometry": {"bbox": [40, 30], "primitives": []},
    },
    "converter.sig2phys": {
        "name": "converter.sig2phys",
        "label": "Sig2Phys",
        "parameters": [{"name": "domain", "type": "enum", "default": "voltage"}],
        # The two sides genuinely differ in kind -- which is why a block-wide `domain` field
        # can never judge either of them, and why this block only became checkable with a
        # contract.
        "ports": [
            {"id": "in", "side": "left", "kind": "signal", "direction": "in"},
            {"id": "out", "side": "right", "kind": "physical-electrical", "direction": "out"},
        ],
        "geometry": {"bbox": [50, 30], "primitives": []},
    },
    "passive.resistor": {
        "name": "passive.resistor",
        "label": "Resistor",
        "parameters": [{"name": "R", "type": "number", "default": 1000}],
        # No `dim` anywhere: a conserving port has no signal width. Both dim checks against
        # this block must therefore skip rather than guess.
        "ports": [
            {"id": "p1", "side": "left", "kind": "physical-electrical", "direction": "inout"},
            {"id": "p2", "side": "right", "kind": "physical-electrical", "direction": "inout"},
        ],
        "geometry": {"bbox": [60, 20], "primitives": []},
    },
}

REGISTRY = json.dumps(CONTRACTS)


def add(model, block_id, type_, paradigm, domain, position, params=None):
    spec = {
        "type": type_,
        "paradigm": paradigm,
        "domain": domain,
        "position": list(position),
        "rotation": 0,
        "id": block_id,
        "params": params or {},
    }
    r = json.loads(gsim_py.add_block(model, json.dumps(spec)))
    assert r["block_id"] == block_id
    return json.dumps(r["model"])


def connect(model, a, b, kind, contracts=REGISTRY):
    """`contracts=None` reproduces exactly what `connect` did before the registry existed."""
    r = json.loads(
        gsim_py.connect(
            model,
            json.dumps({"block": a[0], "port": a[1]}),
            json.dumps({"block": b[0], "port": b[1]}),
            kind,
            "{}",
            contracts,
        )
    )
    return json.dumps(r["model"])


def rejects(model, a, b, kind, contracts=REGISTRY):
    """Attempts a connection that must fail; returns the raised message."""
    try:
        connect(model, a, b, kind, contracts)
    except ValueError as e:
        return str(e)
    raise AssertionError(f"connecting {a} -> {b} as {kind!r} should have been rejected")


def build():
    """Blocks only -- each check below wires them itself, from this shared starting point."""
    model = gsim_py.new_model("dim_mismatch_rejection", "Dim mismatch rejection", 10)
    # A 3-wide constant and a 3-wide gain: these two agree.
    model = add(model, "C3", "source.const", "signal-flow", "signal", (0, 0),
                {"value": [1.0, 2.0, 3.0]})
    model = add(model, "K3", "control.gain", "signal-flow", "signal", (80, 0),
                {"k": GAIN, "width": WIDTH})
    # A 1-wide gain, sharing K3's contract and differing only in its own params.
    model = add(model, "K1", "control.gain", "signal-flow", "signal", (80, 80),
                {"k": GAIN, "width": 1})
    # A scalar constant to drive it, so the "correct wiring" half has a valid chain.
    model = add(model, "C1", "source.const", "signal-flow", "signal", (0, 80),
                {"value": [1.0]})
    # The signal/physical boundary, and a resistor to ground on the far side.
    model = add(model, "S2P", "converter.sig2phys", "converter", "electrical", (160, 80),
                {"domain": "voltage"})
    model = add(model, "R1", "passive.resistor", "physical-network", "electrical", (240, 80),
                {"R": R})
    model = add(model, "gnd", "routing.ground", "physical-network", "electrical", (240, 160))
    return model


def check_dim_mismatch(model):
    """A 3-wide source driving a 1-wide input.

    Nothing in the document says so: both blocks are `signal-flow`, both `domain: "signal"`,
    and the wire is an ordinary `signal`. The width lives only in each block's `params`, read
    through its registered contract.
    """
    msg = rejects(model, ("C3", "out"), ("K1", "in"), "signal")
    assert msg.startswith("port-dim-mismatch: "), msg
    assert "cannot connect 'C3.out' to 'K1.in'" in msg, msg
    assert "the source produces dim [3], but the destination requires dim [1]" in msg, msg
    print("port-dim-mismatch   ->", msg)

    # Matching widths connect -- a checker that rejected everything would pass the line above.
    model = connect(model, ("C3", "out"), ("K3", "in"), "signal")
    assert len(json.loads(model)["wires"]) == 1

    # Two blocks of the *same* type, differing only in their own params: proof the dimension is
    # resolved per block rather than per type.
    msg = rejects(model, ("K3", "out"), ("K1", "in"), "signal")
    assert msg.startswith("port-dim-mismatch: "), msg

    # A port whose contract declares no `dim` at all is unknown, not scalar -- so a 3-wide
    # signal reaching the converter's dimensionless input is not rejected on width.
    connect(model, ("K3", "out"), ("S2P", "in"), "signal")

    # ...and with no registry supplied, the very first rejection above simply does not happen.
    connect(model, ("C3", "out"), ("K1", "in"), "signal", contracts=None)
    print("                       (accepted without a registry, as before)")


def check_kind_mismatch_on_a_bridging_block(model):
    """A `signal` wire onto the converter's `physical-electrical` output.

    A converter has genuinely different kinds on its two sides, so its block-wide `domain`
    names at most one of them and the pre-registry check deliberately said nothing about
    either port. Only the contract can tell these two ports apart.
    """
    msg = rejects(model, ("K1", "out"), ("S2P", "out"), "signal")
    assert msg.startswith("port-kind-mismatch: "), msg
    assert "'S2P.out' is declared 'physical-electrical'" in msg, msg
    assert "as a 'signal' wire" in msg, msg
    print("port-kind-mismatch  ->", msg)

    # The same wire onto the port that *is* signal-kind is accepted.
    connect(model, ("K1", "out"), ("S2P", "in"), "signal")

    # And without the registry the bad one is accepted too -- the fallback is unchanged.
    connect(model, ("K1", "out"), ("S2P", "out"), "signal", contracts=None)
    print("                       (accepted without a registry, as before)")


def check_the_correct_wiring_still_compiles(model):
    """The other half of the proof: a fully valid chain, built with the registry active."""
    model = connect(model, ("C1", "out"), ("K1", "in"), "signal")
    model = connect(model, ("K1", "out"), ("S2P", "in"), "signal")
    model = connect(model, ("S2P", "out"), ("R1", "p1"), "physical-electrical")
    model = connect(model, ("R1", "p2"), ("gnd", "p"), "physical-electrical")
    assert len(json.loads(model)["wires"]) == 4

    # It round-trips through real schema validation...
    gsim_py.load_document(json.dumps({"schema_version": 2, "model": json.loads(model)}))

    # ...and compiles, once the blocks that are not part of this chain are dropped.
    for dropped in ("C3", "K3"):
        model = json.dumps(json.loads(gsim_py.remove_block(model, dropped)))
    netlist = json.loads(gsim_py.compile(model, "general-simulator"))["netlist"]
    lines = netlist.splitlines()
    assert any(line.startswith("R1 ") for line in lines), netlist
    assert any("kind=gain" in line for line in lines), netlist
    print()
    print("the correct wiring, built with the registry active, compiles:")
    print(netlist)


def main():
    check_dim_mismatch(build())
    check_kind_mismatch_on_a_bridging_block(build())
    check_the_correct_wiring_still_compiles(build())
    print()
    print("both contract-backed rejections raised the right Python error, both are absent "
          "without a registry, and the valid wiring still compiles")


if __name__ == "__main__":
    main()
