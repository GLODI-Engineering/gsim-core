"""Port-kind compatibility, rejected for real through the Python binding.

`connect()` has always enforced the *block-level* cross-paradigm boundary
(`docs/specs/gsim-sch-format.md` §8.3). This example exercises the finer **per-port** rules
(§8.1/§8.2) that `crates/gsim-core/src/document/port_compat.rs` adds, ported from
`general-simulator-gui`'s `apps/frontend/src/canvas/port-compat.ts`.

Three rejections are proven end to end, each surfacing a distinct keyword-prefixed
`ValueError` in Python:

    port-kind-mismatch      an electrical node wired straight to a thermal one
    port-fan-in-on-signal   two sources driving one causal input port
    port-reserved-kind      a `reserved.*` kind, catalogued for v3 but not connectable in v2

...and, just as importantly, that the *correct* wiring of the very same blocks is accepted
and still compiles to a real netlist. A checker that rejects everything would pass the
first half of this script and fail the second.

The circuit is a textbook-generic RC low-pass with an independent, deliberately
**unconnected** thermal branch (a thermal resistance to a thermal mass) sitting alongside
it. The thermal branch exists purely so there is a genuine second physical domain in the
same document to try to wire across; nothing in this file is modeled on any real design.

Run it:

    python3 examples/port_kind_rejection.py
"""

import json

import gsim_py

R = 1000.0
C = 1e-6
VIN = 5.0
R_TH = 2.5
C_TH = 0.8
GAIN = 2.0


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


def connect(model, a, b, kind):
    r = json.loads(
        gsim_py.connect(
            model,
            json.dumps({"block": a[0], "port": a[1]}),
            json.dumps({"block": b[0], "port": b[1]}),
            kind,
            "{}",
        )
    )
    return json.dumps(r["model"])


def rejects(model, a, b, kind):
    """Attempts a connection that must fail; returns the raised message."""
    try:
        connect(model, a, b, kind)
    except ValueError as e:
        return str(e)
    raise AssertionError(f"connecting {a} -> {b} as {kind!r} should have been rejected")


def build():
    """Blocks only -- each test below wires them itself, from this shared starting point."""
    model = gsim_py.new_model("port_kind_rejection", "Port-kind rejection", 10)
    # Electrical domain: a plain RC low-pass.
    model = add(model, "Vin", "source.voltage.dc", "physical-network", "electrical",
                (0, 0), {"V": VIN})
    model = add(model, "R1", "passive.resistor", "physical-network", "electrical",
                (60, 0), {"R": R})
    model = add(model, "C1", "passive.capacitor", "physical-network", "electrical",
                (120, 0), {"C": C})
    model = add(model, "gnd", "routing.ground", "physical-network", "electrical", (60, 80))
    # Thermal domain: a separate branch, same document.
    model = add(model, "Rth", "thermal.resistance", "physical-network", "thermal",
                (0, 200), {"R": R_TH})
    model = add(model, "Cth", "thermal.mass", "physical-network", "thermal",
                (80, 200), {"C": C_TH})
    # Signal-flow: two independent sources and one single-input gain.
    model = add(model, "U1", "source.const", "signal-flow", "signal", (0, 320),
                {"value": 1.0})
    model = add(model, "U2", "source.const", "signal-flow", "signal", (0, 380),
                {"value": 2.0})
    model = add(model, "K1", "control.gain", "signal-flow", "signal", (100, 320),
                {"k": GAIN})
    model = add(model, "K2", "control.gain", "signal-flow", "signal", (100, 380),
                {"k": GAIN})
    return model


def check_kind_mismatch(model):
    """An electrical node wired straight to a thermal one.

    Both blocks are `physical-network`, so the older block-level cross-paradigm check has
    nothing to say here -- this is exactly the gap the per-port rule closes.
    """
    msg = rejects(model, ("R1", "p2"), ("Rth", "a"), "physical-electrical")
    assert msg.startswith("port-kind-mismatch: "), msg
    assert "'Rth.a' is declared 'thermal'" in msg, msg
    assert "as a 'physical-electrical' wire" in msg, msg
    print("port-kind-mismatch  ->", msg)

    # The reverse direction is refused just as firmly: the rule is on the pair, not on
    # which endpoint the caller happened to name first.
    msg = rejects(model, ("Rth", "a"), ("R1", "p2"), "physical-thermal")
    assert msg.startswith("port-kind-mismatch: "), msg
    assert "'R1.p2' is declared 'electrical'" in msg, msg


def check_fan_in(model):
    """Two sources driving one causal input port (§8.2: fan-in is forbidden)."""
    model = connect(model, ("U1", "out"), ("K1", "in"), "signal")
    msg = rejects(model, ("U2", "out"), ("K1", "in"), "signal")
    assert msg.startswith("port-fan-in-on-signal: "), msg
    assert "causal port 'K1.in' is already driven" in msg, msg
    print("port-fan-in-on-signal ->", msg)

    # Fan-*out* from that same driver is still fine -- one output, many inputs.
    model = connect(model, ("U1", "out"), ("K2", "in"), "signal")
    assert len(json.loads(model)["wires"]) == 2

    # And the acausal side has no such rule at all: three electrical wires may meet at one
    # node, which is what makes a net a net.
    model = connect(model, ("Vin", "p"), ("R1", "p1"), "physical-electrical")
    model = connect(model, ("C1", "p1"), ("R1", "p1"), "physical-electrical")
    assert len(json.loads(model)["wires"]) == 4


def check_reserved_kind(model):
    """A `reserved.*` kind is catalogued in §8 but not connectable until v3."""
    msg = rejects(model, ("R1", "p2"), ("C1", "p1"), "reserved.physical-magnetic")
    assert msg.startswith("port-reserved-kind: "), msg
    assert "reserved.physical-magnetic" in msg, msg
    assert "reserved for a future schema version" in msg, msg
    print("port-reserved-kind  ->", msg)


def check_the_correct_wiring_still_compiles(model):
    """The other half of the proof: valid connections are accepted and produce a netlist."""
    # Electrical loop, all one domain.
    model = connect(model, ("Vin", "p"), ("R1", "p1"), "physical-electrical")
    model = connect(model, ("R1", "p2"), ("C1", "p1"), "physical-electrical")
    model = connect(model, ("C1", "p2"), ("gnd", "p"), "physical-electrical")
    model = connect(model, ("Vin", "n"), ("gnd", "p"), "physical-electrical")
    # Thermal branch, all one domain -- and note it is a *different* domain in the same
    # document, wired without complaint because it never crosses.
    model = connect(model, ("Rth", "b"), ("Cth", "a"), "physical-thermal")
    # Signal chain, one driver per input.
    model = connect(model, ("U1", "out"), ("K1", "in"), "signal")

    assert len(json.loads(model)["wires"]) == 6

    # It round-trips through real schema validation, thermal branch included...
    doc = json.dumps({"schema_version": 2, "model": json.loads(model)})
    gsim_py.load_document(doc)

    # ...and the electrical part compiles to a real netlist. The thermal branch and the
    # unused signal blocks are dropped first only because `compile::spec::REGISTRY` has no
    # emitter for `thermal.*` yet -- that is a netlist-target gap, not a connectivity one,
    # and the wires above were accepted regardless. `remove_block` cascades their wires.
    for dropped in ("Rth", "Cth", "U1", "U2", "K1", "K2"):
        model = json.dumps(json.loads(gsim_py.remove_block(model, dropped)))
    netlist = json.loads(gsim_py.compile(model, "general-simulator"))["netlist"]
    lines = netlist.splitlines()
    assert any(line.startswith("R1 ") for line in lines), netlist
    assert any(line.startswith("C1 ") for line in lines), netlist
    print()
    print("the correct wiring of the same blocks compiles:")
    print(netlist)


def main():
    check_kind_mismatch(build())
    check_fan_in(build())
    check_reserved_kind(build())
    check_the_correct_wiring_still_compiles(build())
    print()
    print("all three port-kind rejections raised the right Python error, "
          "and the valid wiring still compiles")


if __name__ == "__main__":
    main()
