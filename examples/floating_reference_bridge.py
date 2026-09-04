"""No ground symbol anywhere — the reference-net fallback heuristic, run for real.

`docs/specs/gsim-sch-format.md` §9 says that when no `is_reference` net is declared for an
acausal sub-network, the compiler selects one heuristically and emits an info-level note.
`gsim-core` implements that (the net with the most endpoints wins, ties broken by name so
it is deterministic), but until now it was **unit-tested only**: every checked-in example
declares its ground explicitly, so the heuristic had never produced a netlist that a real
simulator actually solved.

This circuit declares no `routing.ground` and no `is_reference` net. Its return rail is the
node with the most endpoints by a clear margin — four branch resistors, the source's own
negative terminal, and a snubber leg — so the heuristic must pick it, and the emitted
netlist must have a real node `0` or `general-simulator` cannot solve it at all (an MNA
system with no reference row is singular).

Node counts, by construction:

    RAIL  Rb.p2 Rc.p2 Rd.p2 Re.p2 Vsrc.n Rf.p1   -> 6   <- must win
    MID   Ra.p2 Rb.p1 Rc.p1 Rd.p1 Re.p1          -> 5
    TOP   Vsrc.p Ra.p1 Cf.p2                     -> 3
    SNUB  Rf.p2 Cf.p1                            -> 2

Expected DC solution, with RAIL as node 0: the four parallel 400 ohm branches are 100 ohm,
in series with Ra = 100 ohm, so V(MID) = 12 * 100/200 = 6 V and V(TOP) = 12 V.

Run it:

    GSIM_SIMULATOR=/path/to/general-simulator python3 examples/floating_reference_bridge.py
"""

import json
import os
import shutil
import subprocess
import sys

import gsim_py

VSRC = 12.0
RA = 100.0
RBRANCH = 400.0
BRANCHES = 4
RSNUB = 1000.0
CSNUB = 1e-7


def add(model, block_id, type_, position, params=None):
    spec = {
        "type": type_,
        "paradigm": "physical-network",
        "domain": "electrical",
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


def build():
    model = gsim_py.new_model("floating_ref", "Floating-reference resistor bank", 10)
    model = add(model, "Vsrc", "source.voltage.dc", (0, 100), {"V": VSRC})
    model = add(model, "Ra", "passive.resistor", (100, 40), {"R": RA})
    branches = ["Rb", "Rc", "Rd", "Re"]
    for i, rid in enumerate(branches):
        model = add(model, rid, "passive.resistor", (200 + 60 * i, 120), {"R": RBRANCH})
    model = add(model, "Rf", "passive.resistor", (460, 160), {"R": RSNUB})
    model = add(model, "Cf", "passive.capacitor", (460, 60), {"C": CSNUB})

    # TOP -- source positive, the series resistor, the snubber's capacitor return.
    model = elec(model, ("Vsrc", "p"), ("Ra", "p1"), net="TOP")
    model = elec(model, ("Cf", "p2"), ("Vsrc", "p"))
    # MID -- Ra's far end fans out to every branch.
    model = elec(model, ("Ra", "p2"), ("Rb", "p1"), net="MID")
    for rid in branches[1:]:
        model = elec(model, ("Ra", "p2"), (rid, "p1"))
    # RAIL -- every branch return, the source's own negative terminal, the snubber leg.
    # No ground block, and no nets[] entry marked is_reference: this is the whole point.
    model = elec(model, ("Rb", "p2"), ("Vsrc", "n"), net="RAIL")
    for rid in branches[1:]:
        model = elec(model, (rid, "p2"), ("Vsrc", "n"))
    model = elec(model, ("Rf", "p1"), ("Vsrc", "n"))
    model = elec(model, ("Rf", "p2"), ("Cf", "p1"), net="SNUB")
    return model


def main():
    model = build()
    parsed = json.loads(model)
    assert not any(b["type"] == "routing.ground" for b in parsed["blocks"]), \
        "this example must declare no ground at all"
    assert parsed.get("nets", []) == [], parsed.get("nets")

    result = json.loads(gsim_py.compile(model, "general-simulator"))
    netlist = result["netlist"]
    # §9's mandated info-level note, not an error.
    assert len(result["notes"]) == 1, result["notes"]
    assert "no reference net declared" in result["notes"][0], result["notes"]
    print(f"note: {result['notes'][0]}")

    # The heuristic must have chosen RAIL: it is the only net that becomes node 0, and the
    # explicitly-named TOP/MID/SNUB must all survive as themselves.
    assert "RAIL" not in netlist, netlist
    assert "Vsrc TOP 0 12" in netlist, netlist
    assert "Ra TOP MID 100" in netlist, netlist
    for rid in ("Rb", "Rc", "Rd", "Re"):
        assert f"{rid} MID 0 400" in netlist, netlist
    assert "Rf 0 SNUB 1000" in netlist, netlist

    out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "floating_reference_bridge.cir")
    with open(out_path, "w") as fh:
        fh.write(netlist)
    print(f"wrote {out_path} ({len(netlist.splitlines())} lines)")

    simulator = os.environ.get("GSIM_SIMULATOR") or shutil.which("general-simulator")
    if not simulator:
        print("general-simulator not found (set GSIM_SIMULATOR) -- compiled only, not run")
        return
    proc = subprocess.run(
        [simulator, out_path, "--mode", "transient", "--tfinal", "1e-3", "--dt", "1e-6"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print(proc.stderr[:2000], file=sys.stderr)
        raise SystemExit(f"general-simulator failed with exit code {proc.returncode}")
    rows = proc.stdout.splitlines()
    header = rows[0].split(",")
    final = rows[-1].split(",")
    top = float(final[header.index("V(TOP)")])
    mid = float(final[header.index("V(MID)")])
    expected_mid = VSRC * (RBRANCH / BRANCHES) / (RA + RBRANCH / BRANCHES)
    print(f"ran clean: {len(rows) - 1} steps, {len(header)} columns")
    print(f"V(TOP)={top:.6f} V (source {VSRC} V), V(MID)={mid:.6f} V "
          f"(divider {expected_mid:.6f} V)")
    # A wrong reference choice would either fail to solve or float the whole network.
    assert abs(top - VSRC) < 1e-9, top
    assert abs(mid - expected_mid) < 1e-9, mid


if __name__ == "__main__":
    main()
