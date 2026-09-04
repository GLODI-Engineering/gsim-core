"""A real buck converter regulated by a **state-space** PI compensator.

The acceptance case for the linear-dynamics families the Phase 2 registry originally left
out (`control.state_space`, `control.transfer_function`), plus the three already-registered
signal-flow families whose `in=`/`inputs=` spelling had never actually been run
(`control.saturation`, `power.pwm1`, `converter.phys2sig`).

Power stage: a 24 V link, an ideal switch, a freewheeling ideal diode, a 100 uH / 100 uF LC
filter into a 5 ohm load, switched at 100 kHz. Control loop:

    V(VOUT) --phys2sig--> VSENSE --tf(1/(1+10us))--> VFILT --.
                                                             |
    REF (12 V) ---------------------------------------------- PI (2-input statespace)
                                                             |
                                          duty <--saturation-'  --> pwm --> sig2phys --> gate

The compensator is a genuine **MIMO** state-space: rather than needing a separate summing
block, it takes `[ref, measurement]` as two inputs and forms the error itself --
`b=[[ki,-ki]]`, `d=[[kp,-kp]]`, `a=[[0]]`, `c=[[1]]` is exactly a PI controller, with the
integrator as its one state. That is also what makes it a real test of the `inputs=` list
whose length comes from `b`'s own column count.

Run it:

    GSIM_SIMULATOR=/path/to/general-simulator python3 examples/buck_state_space_pi.py

Writes `buck_state_space_pi.cir` next to itself. With no simulator it compiles and asserts
but does not run.
"""

import json
import os
import shutil
import subprocess
import sys

import gsim_py

VIN = 24.0
VREF = 12.0
RLOAD = 5.0
LF = 100e-6
CF = 100e-6
FSW = 100e3
KP = 0.005
KI = 50.0

# Generic ideal-switch / ideal-diode model constants chosen for this example: 10 mOhm on,
# 1 nS off, a 0.5 V gate threshold against the 0/1 V gate drive, and a breakdown well clear
# of the 24 V link so it never participates. Nothing here is copied from another circuit.
SWITCH_PARAMS = {
    "r_on": 0.01,
    "g_breakdown": 0.0,
    "v_breakdown": -500.0,
    "g_off": 1e-9,
    "v_th": 0.5,
    "g_on": 200.0,
}
DIODE_PARAMS = {
    "g_breakdown": 0.0,
    "v_breakdown": -500.0,
    "g_off": 1e-9,
    "v_th": 0.0,
    "g_on": 200.0,
}


def add(model, block_id, type_, paradigm, position, params=None):
    spec = {
        "type": type_,
        "paradigm": paradigm,
        "domain": "electrical",
        "position": list(position),
        "rotation": 0,
        "id": block_id,
        "params": params or {},
    }
    r = json.loads(gsim_py.add_block(model, json.dumps(spec)))
    assert r["block_id"] == block_id
    return json.dumps(r["model"])


def wire(model, frm, to, kind, net=None):
    opts = {"waypoints": []}
    if net is not None:
        opts["net"] = net
    r = json.loads(
        gsim_py.connect(
            model,
            json.dumps({"block": frm[0], "port": frm[1]}),
            json.dumps({"block": to[0], "port": to[1]}),
            kind,
            json.dumps(opts),
        )
    )
    return json.dumps(r["model"])


def build():
    model = gsim_py.new_model("buck_ss_pi", "Buck with a state-space PI", 10)

    # -- power stage ----------------------------------------------------------
    model = add(model, "gnd", "routing.ground", "physical-network", (0, 300))
    model = add(model, "Vdc", "source.voltage.dc", "physical-network", (0, 150), {"V": VIN})
    model = add(model, "DSW", "passive.switch.ideal", "physical-network", (120, 100),
                SWITCH_PARAMS)
    model = add(model, "Dfw", "passive.diode.ideal", "physical-network", (120, 220),
                DIODE_PARAMS)
    model = add(model, "Lf", "passive.inductor", "physical-network", (240, 100), {"L": LF})
    model = add(model, "Cf", "passive.capacitor", "physical-network", (360, 220), {"C": CF})
    model = add(model, "Rload", "passive.resistor", "physical-network", (460, 220),
                {"R": RLOAD})

    e = "physical-electrical"
    # The `node=` a phys2sig reads is a *net name*, so this circuit names its three real nets
    # explicitly rather than letting the compiler generate `n_<block>_<port>` ids.
    model = wire(model, ("Vdc", "p"), ("DSW", "d"), e, net="VDCP")
    model = wire(model, ("Vdc", "n"), ("gnd", "p"), e)
    model = wire(model, ("DSW", "s"), ("Dfw", "k"), e, net="SW")
    model = wire(model, ("Dfw", "a"), ("gnd", "p"), e)
    model = wire(model, ("DSW", "s"), ("Lf", "p1"), e)
    model = wire(model, ("Lf", "p2"), ("Cf", "p1"), e, net="VOUT")
    model = wire(model, ("Cf", "p2"), ("gnd", "p"), e)
    model = wire(model, ("Lf", "p2"), ("Rload", "p1"), e)
    model = wire(model, ("Rload", "p2"), ("gnd", "p"), e)

    # -- control loop ---------------------------------------------------------
    model = add(model, "VSENSE", "converter.phys2sig", "converter", (460, 60),
                {"node": "VOUT"})
    # A 100 kHz single-pole measurement filter: num=[1], den=[1e-5, 1] is 1/(1 + 10us).
    model = add(model, "VFILT", "control.transfer_function", "signal-flow", (360, 20),
                {"num": [1.0], "den": [1e-5, 1.0]})
    model = add(model, "REF", "source.const", "signal-flow", (360, -60), {"value": VREF})
    model = add(
        model, "PI", "control.state_space", "signal-flow", (240, -20),
        # xdot = ki*(ref - meas);  y = x + kp*(ref - meas)
        {"a": [[0.0]], "b": [[KI, -KI]], "c": [[1.0]], "d": [[KP, -KP]]},
    )
    model = add(model, "DUTY", "control.saturation", "signal-flow", (140, -20),
                {"limit": 0.95})
    model = add(model, "MOD", "power.pwm1", "signal-flow", (40, -20), {"freq": FSW})
    model = add(model, "GATE", "converter.sig2phys", "converter", (40, 60),
                {"domain": "voltage"})

    s = "signal"
    model = wire(model, ("REF", "out"), ("PI", "in0"), s)
    model = wire(model, ("VSENSE", "out"), ("VFILT", "in"), s)
    model = wire(model, ("VFILT", "out"), ("PI", "in1"), s)
    model = wire(model, ("PI", "out0"), ("DUTY", "in"), s)
    model = wire(model, ("DUTY", "out"), ("MOD", "in"), s)
    model = wire(model, ("MOD", "main"), ("GATE", "in"), s)
    model = wire(model, ("GATE", "out"), ("DSW", "g"), e)
    return model


def main():
    model = build()
    result = json.loads(gsim_py.compile(model, "general-simulator"))
    netlist = result["netlist"]
    assert result["notes"] == [], result["notes"]

    # The shapes this example exists to prove, asserted before anything is run.
    assert "PI kind=statespace a=[[0]] b=[[50,-50]] c=[[1]] d=[[0.005,-0.005]] " \
           "inputs=REF,VFILT" in netlist, netlist
    assert "VFILT kind=tf num=[1] den=[1e-5,1] in=VSENSE" in netlist, netlist
    assert "DUTY kind=saturation limit=0.95 in=PI" in netlist, netlist
    assert "MOD kind=pwm freq=100000 in=DUTY outputs=MOD,MOD_comp" in netlist, netlist
    assert "VSENSE kind=phys2sig node=VOUT" in netlist, netlist
    assert "Lf SW VOUT 0.0001" in netlist, netlist

    out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "buck_state_space_pi.cir")
    with open(out_path, "w") as fh:
        fh.write(netlist)
    print(f"wrote {out_path} ({len(netlist.splitlines())} lines)")

    simulator = os.environ.get("GSIM_SIMULATOR") or shutil.which("general-simulator")
    if not simulator:
        print("general-simulator not found (set GSIM_SIMULATOR) -- compiled only, not run")
        return
    proc = subprocess.run(
        [simulator, out_path, "--mode", "transient", "--tfinal", "2e-2", "--dt", "5e-8"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print(proc.stderr[:2000], file=sys.stderr)
        raise SystemExit(f"general-simulator failed with exit code {proc.returncode}")
    rows = proc.stdout.splitlines()
    header = rows[0].split(",")
    print(f"ran clean: {len(rows) - 1} steps, {len(header)} columns")

    vout = header.index("V(VOUT)")
    duty = header.index("DUTY")
    tail = [r.split(",") for r in rows[-4000:]]
    settled = sum(float(r[vout]) for r in tail) / len(tail)
    print(f"V(VOUT) settled mean over the last 4000 steps: {settled:.4f} V "
          f"(reference {VREF} V)")
    print(f"duty at the final step: {float(tail[-1][duty]):.4f} "
          f"(open-loop ideal {VREF / VIN:.4f})")
    # The loop must actually regulate -- a netlist that parses but does not close the loop
    # would sit at the open-loop operating point instead.
    assert abs(settled - VREF) < 0.2, settled


if __name__ == "__main__":
    main()
