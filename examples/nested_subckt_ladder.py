"""A `.subckt` that instantiates another `.subckt` — the nesting case the Phase 2 emitter
left unimplemented, proven end to end.

Four-section RC delay line, built as two levels of reuse rather than four copies:

    SECTION   R + C, one ladder section          (blocks + wires)
    STAGE     two SECTION instances in series    (nothing *but* nested X cards)
    top       two STAGE instances in series      (ordinary top-level X cards)

`STAGE`'s body is deliberately empty of ordinary blocks: it is nothing but two `X` cards of
`SECTION`. That is exactly the shape the old emitter dropped silently — the `.SUBCKT` card
and `.ENDS` were written with nothing between them, producing a netlist that parsed and
simulated an open circuit.

Final DC value is a plain resistive divider once the capacitors are charged: four 1k
sections into a 10k load, so `V(out) = 5 * 10k / 14k = 3.5714 V`. The script asserts that,
which is what makes it a real check rather than a text-shape check.

Run it:

    GSIM_SIMULATOR=/path/to/general-simulator python3 examples/nested_subckt_ladder.py
"""

import json
import os
import shutil
import subprocess
import sys

import gsim_py

GRID = 10
BOUNDARY_TYPE = "spice-subckt.boundary"

VIN = 5.0
RSEC = 1000.0
CSEC = 1e-6
RLOAD = 10000.0
SECTIONS = 4


def add_block(model, block_id, type_, paradigm, domain, position, params=None):
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


def elec(model, a, b, net=None):
    opts = {"waypoints": []}
    if net is not None:
        opts["net"] = net
    r = json.loads(
        gsim_py.connect(
            model,
            json.dumps({"block": a[0], "port": a[1]}),
            json.dumps({"block": b[0], "port": b[1]}),
            "physical-electrical",
            json.dumps(opts),
        )
    )
    return json.dumps(r["model"])


def section_def():
    """One ladder section: `a --[R]-- b`, `b --[C]-- g`."""
    body = gsim_py.new_model("__section_body", "section", GRID)
    body = add_block(body, "section", BOUNDARY_TYPE, "spice-subckt", "mixed", (0, 0))
    body = add_block(body, "Rs", "passive.resistor", "physical-network", "electrical",
                     (40, 0), {"R": RSEC})
    body = add_block(body, "Cs", "passive.capacitor", "physical-network", "electrical",
                     (80, 40), {"C": CSEC})
    body = elec(body, ("section", "a"), ("Rs", "p1"))
    body = elec(body, ("Rs", "p2"), ("section", "b"))
    body = elec(body, ("Rs", "p2"), ("Cs", "p1"))
    body = elec(body, ("Cs", "p2"), ("section", "g"))
    m = json.loads(body)
    return {
        "id": "section",
        "name": "SECTION",
        "ports": [{"id": p, "kind": "physical-electrical"} for p in ("a", "b", "g")],
        "blocks": m["blocks"],
        "wires": m["wires"],
        "instances": [],
    }


def stage_def():
    """Two SECTION instances in series — a body made of nothing but nested X cards.

    A `SubcktInstance`'s `connections` are positional *names* in the enclosing body's own
    namespace (the def's boundary port names, plus any internal net name it invents), the
    same thing a hand-written `X` card inside a `.SUBCKT` carries.
    """
    return {
        "id": "stage",
        "name": "STAGE",
        "ports": [{"id": p, "kind": "physical-electrical"} for p in ("a", "b", "g")],
        "blocks": [],
        "wires": [],
        "instances": [
            {"id": "Xs1", "def_ref": "section", "connections": ["a", "mid", "g"],
             "params": {}},
            {"id": "Xs2", "def_ref": "section", "connections": ["mid", "b", "g"],
             "params": {}},
        ],
    }


def instantiate(model, def_ref, block_id, position):
    r = json.loads(
        gsim_py.instantiate_subckt(
            model,
            json.dumps({"def_ref": def_ref, "position": list(position), "id": block_id}),
        )
    )
    assert r["block_id"] == block_id
    return json.dumps(r["model"])


def build():
    model = gsim_py.new_model("nested_ladder", "Nested-subckt RC ladder", GRID)
    for d in (section_def(), stage_def()):
        r = json.loads(gsim_py.add_subckt_def(model, json.dumps(d)))
        assert r["def_id"] == d["id"]
        model = json.dumps(r["model"])

    model = add_block(model, "gnd", "routing.ground", "physical-network", "electrical",
                      (0, 200))
    model = add_block(model, "Vin", "source.voltage.dc", "physical-network", "electrical",
                      (0, 100), {"V": VIN})
    model = add_block(model, "Rload", "passive.resistor", "physical-network", "electrical",
                      (400, 100), {"R": RLOAD})
    model = instantiate(model, "stage", "X1", (120, 60))
    model = instantiate(model, "stage", "X2", (260, 60))

    model = elec(model, ("Vin", "p"), ("X1", "a"), net="VIN")
    model = elec(model, ("Vin", "n"), ("gnd", "p"))
    model = elec(model, ("X1", "b"), ("X2", "a"), net="MID")
    model = elec(model, ("X2", "b"), ("Rload", "p1"), net="VOUT")
    model = elec(model, ("Rload", "p2"), ("gnd", "p"))
    model = elec(model, ("X1", "g"), ("gnd", "p"))
    model = elec(model, ("X2", "g"), ("gnd", "p"))
    return model


def main():
    model = build()
    result = json.loads(gsim_py.compile(model, "general-simulator"))
    netlist = result["netlist"]
    assert result["notes"] == [], result["notes"]

    # The nested X cards must land *inside* STAGE's own .subckt/.ends block.
    stage_body = netlist.split(".subckt STAGE a b g")[1].split(".ends STAGE")[0]
    assert "Xs1 a mid g SECTION" in stage_body, netlist
    assert "Xs2 mid b g SECTION" in stage_body, netlist
    # ... and SECTION's own body must still be a real ladder section.
    section_body = netlist.split(".subckt SECTION a b g")[1].split(".ends SECTION")[0]
    assert "Rs a b 1000" in section_body, netlist
    assert "Cs b g 1e-6" in section_body, netlist
    assert "X1 VIN MID 0 STAGE" in netlist, netlist
    assert "X2 MID VOUT 0 STAGE" in netlist, netlist

    out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "nested_subckt_ladder.cir")
    with open(out_path, "w") as fh:
        fh.write(netlist)
    print(f"wrote {out_path} ({len(netlist.splitlines())} lines)")

    simulator = os.environ.get("GSIM_SIMULATOR") or shutil.which("general-simulator")
    if not simulator:
        print("general-simulator not found (set GSIM_SIMULATOR) -- compiled only, not run")
        return
    proc = subprocess.run(
        [simulator, out_path, "--mode", "transient", "--tfinal", "0.2", "--dt", "1e-5"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print(proc.stderr[:2000], file=sys.stderr)
        raise SystemExit(f"general-simulator failed with exit code {proc.returncode}")
    rows = proc.stdout.splitlines()
    header = rows[0].split(",")
    final = rows[-1].split(",")
    vout = float(final[header.index("V(VOUT)")])
    expected = VIN * RLOAD / (SECTIONS * RSEC + RLOAD)
    print(f"ran clean: {len(rows) - 1} steps, {len(header)} columns")
    print(f"V(VOUT) settled: {vout:.5f} V  (divider through {SECTIONS} nested "
          f"sections: {expected:.5f} V)")
    # An empty STAGE body would leave X1/X2 as open circuits and V(VOUT) would sit at 0.
    assert abs(vout - expected) < 1e-3, vout


if __name__ == "__main__":
    main()
