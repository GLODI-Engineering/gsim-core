"""A `.subckt` whose instances carry their own parameter values — the half of #111 that was
stored and dropped, proven end to end.

One definition, two instances, two resistances:

    RPART     one resistor, valued `{Rv}`, with `Rv=1000` declared as the default
    X1        RPART with `Rv=100`
    X2        RPART with `Rv=220`

Until `gsim-core 0.9.0` the definition's formal list and each instance's values were held in
the document and emitted nowhere, so these two instances compiled **byte-identically** — and
with `notes: []`, so nothing said a word. The deck below is what makes one definition a
reusable component rather than a named group of statements, and it is what the composite
component (a real resistor with its own series inductance) is built out of.

The solver half is `general-mna#63`, shipped in `general-simulator 0.11.0`: the definition's
declared defaults are bound, the `X` card's `name=value` tail overrides them, and the body's
`{Rv}` resolves per instance. Both halves have to be present for this example to run, which
is the point of running it.

The numbers are a plain series divider: 100 Ω and 220 Ω across 5 V, so the source carries
5/320 A = 15.625 mA and the mid-node sits at 5 × 220/320 = 3.4375 V. If the instance values
were dropped and both resistors took the declared default of 1000 Ω, the same deck would read
2.5 mA and a mid-node at exactly half — which this script also asserts is *not* what came out.

Run it:

    GSIM_SIMULATOR=/path/to/general-simulator python3 examples/parameterised_rpart.py
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
RDEFAULT = 1000.0
R_X1 = 100.0
R_X2 = 220.0


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


def rpart_def():
    """`p --[R]-- n`, the resistance written as the definition's own parameter.

    `{Rv}` is deliberately *not* resolved by the compiler: `Rv` is this definition's
    parameter, so the instance supplies it and the solver binds it where the subcircuit is
    expanded (gsim-core#121 is the rule that leaves it alone while still resolving a sheet
    parameter in the same body).
    """
    body = gsim_py.new_model("__rpart_body", "rpart", GRID)
    body = add_block(body, "rpart", BOUNDARY_TYPE, "spice-subckt", "mixed", (0, 0))
    body = add_block(body, "R", "passive.resistor", "physical-network", "electrical",
                     (40, 0), {"R": "{Rv}"})
    body = elec(body, ("rpart", "p"), ("R", "p"))
    body = elec(body, ("R", "n"), ("rpart", "n"))
    m = json.loads(body)
    return {
        "id": "rpart",
        "name": "RPART",
        "ports": [{"id": p, "kind": "physical-electrical"} for p in ("p", "n")],
        "params": [{"name": "Rv", "value": RDEFAULT}],
        "blocks": m["blocks"],
        "wires": m["wires"],
        "instances": [],
    }


def instantiate(model, def_ref, block_id, position, params):
    r = json.loads(
        gsim_py.instantiate_subckt(
            model,
            json.dumps(
                {
                    "def_ref": def_ref,
                    "position": list(position),
                    "id": block_id,
                    "params": params,
                }
            ),
        )
    )
    assert r["block_id"] == block_id
    return json.dumps(r["model"])


def build():
    model = gsim_py.new_model("parameterised_rpart", "Parameterised resistor subcircuit", GRID)
    r = json.loads(gsim_py.add_subckt_def(model, json.dumps(rpart_def())))
    assert r["def_id"] == "rpart"
    model = json.dumps(r["model"])

    model = add_block(model, "gnd", "routing.ground", "physical-network", "electrical",
                      (0, 200))
    model = add_block(model, "Vin", "source.voltage.dc", "physical-network", "electrical",
                      (0, 100), {"V": VIN})
    model = instantiate(model, "rpart", "X1", (120, 60), {"Rv": R_X1})
    model = instantiate(model, "rpart", "X2", (260, 60), {"Rv": R_X2})

    model = elec(model, ("Vin", "p"), ("X1", "p"), net="VIN")
    model = elec(model, ("X1", "n"), ("X2", "p"), net="MID")
    model = elec(model, ("X2", "n"), ("gnd", "p"))
    model = elec(model, ("Vin", "n"), ("gnd", "p"))
    return model


def main():
    model = build()
    result = json.loads(gsim_py.compile(model, "general-simulator"))
    netlist = result["netlist"]
    assert result["notes"] == [], result["notes"]

    # The definition declares its default after the port list.
    assert ".subckt RPART p n Rv=1000" in netlist, netlist
    # The body keeps the reference, for the solver to bind per instance.
    body = netlist.split(".subckt RPART p n Rv=1000")[1].split(".ends RPART")[0]
    assert "R p n {Rv}" in body, netlist
    # And each instance carries its own value. These two cards used to be identical except for
    # their names and nets, which is the whole defect in one line.
    assert "X1 VIN MID RPART Rv=100" in netlist, netlist
    assert "X2 MID 0 RPART Rv=220" in netlist, netlist

    out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "parameterised_rpart.cir")
    with open(out_path, "w") as fh:
        fh.write(netlist)
    print(f"wrote {out_path} ({len(netlist.splitlines())} lines)")

    simulator = os.environ.get("GSIM_SIMULATOR") or shutil.which("general-simulator")
    if not simulator:
        print("general-simulator not found (set GSIM_SIMULATOR) -- compiled only, not run")
        return
    proc = subprocess.run(
        [simulator, out_path, "--dt", "1e-6"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print(proc.stderr[:2000], file=sys.stderr)
        raise SystemExit(f"general-simulator failed with exit code {proc.returncode}")
    rows = proc.stdout.splitlines()
    header = rows[0].split(",")
    first = rows[1].split(",")
    v_mid = float(first[header.index("V(MID)")])
    i_in = float(first[header.index("I(Vin)")])

    expected_mid = VIN * R_X2 / (R_X1 + R_X2)
    expected_i = -VIN / (R_X1 + R_X2)
    print(f"ran clean: {len(rows) - 1} row(s), {len(header)} columns")
    print(f"V(MID) = {v_mid:.5f} V  (divider {R_X2:.0f}/{R_X1 + R_X2:.0f} of {VIN} V: "
          f"{expected_mid:.5f} V)")
    print(f"I(Vin) = {i_in:.7f} A  (5 V over {R_X1 + R_X2:.0f} ohm: {expected_i:.7f} A)")
    assert abs(v_mid - expected_mid) < 1e-9, v_mid
    assert abs(i_in - expected_i) < 1e-12, i_in

    # The falsifier: if both instances had silently taken the declared default, this deck
    # would be two 1000 ohm resistors -- 2.5 mA and a mid-node at exactly half.
    both_default = VIN * RDEFAULT / (2 * RDEFAULT)
    assert abs(v_mid - both_default) > 1e-3, (
        f"V(MID) = {v_mid} is what two {RDEFAULT} ohm resistors would give: the instance "
        "values were dropped"
    )
    print("and not the all-defaults circuit, which would read "
          f"{both_default:.5f} V and {-VIN / (2 * RDEFAULT):.7f} A")


if __name__ == "__main__":
    main()
