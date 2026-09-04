"""Compiles and runs the flat (non-subckt) path: the `h_bridge_3phase.py` stamp, completed.

`h_bridge_3phase.gsim_sch` on its own is deliberately **not** a runnable circuit -- it is a
library *stamp*, an incomplete fragment exposing 11 boundary attachment points (6 gate drives,
3 phase taps, DC+/DC-) that a caller is expected to wire. Compiling it standalone correctly
fails with `unconnected-ref: port 'lega_high.g' must reference another block's output`.

This script takes that same stamp and completes it into a real 3-phase inverter -- DC link,
ground, one phase-shift PWM modulator per leg, a gate-drive converter per switch, and a
star-connected R-L load -- then compiles it with `gsim_py.compile` and (if
`general-simulator` is on PATH or GSIM_SIMULATOR is set) runs the result.

It exercises two things the subckt example does not:

* the **flat path** -- no `SubcktDef`, no `.SUBCKT`/`X` cards at all;
* **per-output naming** for real: each leg's high-side switch is gated by its modulator's
  *first* output (which aliases the modulator block's own name) and its low-side switch by the
  *second* (`<name>_comp`), so a wrong second-output name would produce a shoot-through or a
  dead leg rather than a silent no-op.
"""

import json
import os
import shutil
import subprocess
import sys

import gsim_py

from h_bridge_3phase import (
    DC_MINUS_Y,
    DC_PLUS_Y,
    GRID,
    build as build_stamp,
    connect as connect_elec,
)

FSW = 20000.0
DUTY = 0.48


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


def wire(model, from_ep, to_ep, kind):
    r = json.loads(
        gsim_py.connect(
            model,
            json.dumps({"block": from_ep[0], "port": from_ep[1]}),
            json.dumps({"block": to_ep[0], "port": to_ep[1]}),
            kind,
            json.dumps({"waypoints": []}),
        )
    )
    return json.dumps(r["model"])


def build():
    model, boundary = build_stamp()

    # -- DC link and reference ------------------------------------------------
    model = add(model, "gnd", "routing.ground", "physical-network", "electrical",
                (-200, DC_MINUS_Y))
    model = add(model, "Vdc", "source.voltage.dc", "physical-network", "electrical",
                (-200, 60), {"V": 400.0})
    model = connect_elec(model, {"block": "Vdc", "port": "p"}, boundary["dc_plus"])
    model = connect_elec(model, {"block": "Vdc", "port": "n"}, {"block": "gnd", "port": "p"})
    model = connect_elec(model, boundary["dc_minus"], {"block": "gnd", "port": "p"})

    # -- gate drive: one modulator per leg, high side on `main`, low side on `comp` --
    model = add(model, "FSW", "source.const", "signal-flow", "signal",
                (-200, -220), {"value": FSW})
    model = add(model, "DUTY", "source.const", "signal-flow", "signal",
                (-200, -160), {"value": DUTY})

    legs = ["a", "b", "c"]
    for i, leg in enumerate(legs):
        phase_id = f"PH_{leg}"
        mod_id = f"MOD_{leg}"
        model = add(model, phase_id, "source.const", "signal-flow", "signal",
                    (-200, -100 + 60 * i), {"value": i / 3.0})
        model = add(model, mod_id, "power.pwm2", "signal-flow", "signal",
                    (i * 150, -260), {"f_min": FSW, "f_max": FSW, "red": 0.0, "fed": 0.0})
        model = wire(model, ("FSW", "out"), (mod_id, "freq"), "signal")
        model = wire(model, (phase_id, "out"), (mod_id, "phase"), "signal")
        model = wire(model, ("DUTY", "out"), (mod_id, "duty"), "signal")

        # main -> high-side gate, comp -> low-side gate. The `comp` output is the one that
        # needs its own emitted name (`MOD_a_comp`); getting it wrong breaks the leg.
        for side, mod_port in (("high", "main"), ("low", "comp")):
            gate_id = f"GATE_{leg}_{side}"
            model = add(model, gate_id, "converter.sig2phys", "converter", "electrical",
                        (i * 150 - 60, -180 if side == "high" else 220),
                        {"domain": "voltage"})
            model = wire(model, (mod_id, mod_port), (gate_id, "in"), "signal")
            model = connect_elec(
                model,
                {"block": gate_id, "port": "out"},
                boundary[f"gate_{leg}_{side}"],
            )

    # -- star-connected R-L load ---------------------------------------------
    for i, leg in enumerate(legs):
        model = add(model, f"Rload{leg}", "passive.resistor", "physical-network", "electrical",
                    (i * 150, DC_PLUS_Y + 420), {"R": 10.0})
        model = add(model, f"Lload{leg}", "passive.inductor", "physical-network", "electrical",
                    (i * 150, DC_PLUS_Y + 480), {"L": 2e-3})
        model = connect_elec(model, boundary[f"phase_{leg}"],
                             {"block": f"Rload{leg}", "port": "p1"})
        model = connect_elec(model, {"block": f"Rload{leg}", "port": "p2"},
                             {"block": f"Lload{leg}", "port": "p1"})
        # star point: chained onto leg a's inductor, the same fan-out convention the stamp uses
        model = connect_elec(model, {"block": f"Lload{leg}", "port": "p2"},
                             {"block": "Lloada", "port": "p2"})

    return model


def main():
    model = build()

    result = json.loads(gsim_py.compile(model, "general-simulator"))
    netlist = result["netlist"]
    for note in result["notes"]:
        print(f"note: {note}")

    # The flat path must produce no hierarchy at all.
    assert ".subckt" not in netlist.lower(), "this example must not emit any subckt"
    # Per-output naming: the low side is gated by the modulator's *second* output.
    for leg in ("a", "b", "c"):
        assert f"outputs=MOD_{leg},MOD_{leg}_comp" in netlist, netlist
        assert f"GATE_{leg}_low kind=sig2phys domain=voltage in=MOD_{leg}_comp" in netlist, netlist
        assert f"GATE_{leg}_high kind=sig2phys domain=voltage in=MOD_{leg}" in netlist, netlist

    out_path = os.path.join(os.path.dirname(__file__), "h_bridge_3phase_driven.cir")
    with open(out_path, "w") as f:
        f.write(netlist)
    print(f"wrote {out_path} ({len(netlist.splitlines())} lines)")

    simulator = os.environ.get("GSIM_SIMULATOR") or shutil.which("general-simulator")
    if not simulator:
        print("general-simulator not found (set GSIM_SIMULATOR) -- compiled only, not run")
        return
    proc = subprocess.run(
        [simulator, out_path, "--mode", "transient", "--tfinal", "2e-3", "--dt", "5e-8"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print(proc.stderr[:2000], file=sys.stderr)
        raise SystemExit(f"general-simulator failed with exit code {proc.returncode}")
    rows = proc.stdout.splitlines()
    header = rows[0].split(",")
    last = [float(v) for v in rows[-1].split(",")]
    print(f"ran clean: {len(rows) - 1} steps, {len(header)} columns")
    idx = header.index("I(Vdc)")
    currents = [float(r.split(",")[idx]) for r in rows[1:]]
    print(f"I(Vdc): min={min(currents):.4g} A  max={max(currents):.4g} A  final t={last[0]:.3g} s")


if __name__ == "__main__":
    main()
