"""A buck converter whose controller is **Python**, run for real.

The acceptance case for the script-hosted registry families (`extensibility.pyfunc`,
`extensibility.pyblock`; `cscript`/`octfunc`/`octblock` share their field shape and are
covered by unit tests only — see the module note at the bottom).

    V(VDCP) --phys2sig--> VINM --.
                                  >--pyfunc(duty_feedforward)--> FF, FF_headroom
    REF (12 V) ------------------'                                |        |
                                                                  |        `--gain--> HEAD
                                                pyblock(soft start slew limiter)
                                                                  |
                                                       pwm --> sig2phys --> gate

Two things this proves that a text assertion cannot:

* `kind=pyfunc` is called **positionally** with one argument per declared `inputs=` entry,
  and returns a tuple matching its declared `outputs=` — so the emitted `inputs=REF,VINM`
  order is load-bearing (swapping them makes `duty = vin/vref`, a duty of 2).
* The **second** output's generated name is real: `FF_headroom` is consumed downstream by a
  gain block, so a wrong second-output name is a hard `MissingSymbol`, not a silent no-op.

The Python controller is written to `pyblock_buck_control_blocks.py` next to the netlist.

Run it:

    GSIM_SIMULATOR=/path/to/general-simulator-built-with---features-python \\
        python3 examples/pyblock_buck_control.py
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

import gsim_py

VIN = 24.0
VREF = 12.0
RLOAD = 5.0
LF = 100e-6
CF = 100e-6
FSW = 100e3
SLEW_PER_SECOND = 400.0

BLOCKS_PY = '''"""Controller blocks for examples/pyblock_buck_control.py.

`duty_feedforward` is the `kind=pyfunc` contract: a plain function, called positionally with
one argument per declared `inputs=` entry, returning one value per declared `outputs=` entry.

`start`/`output`/`update` are the `kind=pyblock` contract: a stateful block. This one is a
slew-rate limiter, i.e. a soft start -- the duty command may only rise so fast, which is a
genuine thing a real converter controller does and a stateless function cannot express.
"""

SLEW_PER_SECOND = {slew}


def duty_feedforward(v_ref, v_in):
    """Open-loop buck feedforward: d = Vref/Vin, plus the headroom left over."""
    if v_in <= 1e-9:
        return 0.0, 0.0
    duty = v_ref / v_in
    duty = min(max(duty, 0.0), 0.95)
    return duty, v_in - v_ref


def start():
    return {{"duty": 0.0}}


def output(state, t, dt, inputs):
    return state["duty"]


def update(state, t, dt, inputs):
    target = inputs[0]
    step = SLEW_PER_SECOND * dt
    duty = state["duty"]
    if target > duty + step:
        duty += step
    elif target < duty - step:
        duty -= step
    else:
        duty = target
    state["duty"] = duty
    return state
'''

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


def build(blocks_ref):
    model = gsim_py.new_model("pyblock_buck", "Buck with a Python controller", 10)

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
    model = wire(model, ("Vdc", "p"), ("DSW", "d"), e, net="VDCP")
    model = wire(model, ("Vdc", "n"), ("gnd", "p"), e)
    model = wire(model, ("DSW", "s"), ("Dfw", "k"), e, net="SW")
    model = wire(model, ("Dfw", "a"), ("gnd", "p"), e)
    model = wire(model, ("DSW", "s"), ("Lf", "p1"), e)
    model = wire(model, ("Lf", "p2"), ("Cf", "p1"), e, net="VOUT")
    model = wire(model, ("Cf", "p2"), ("gnd", "p"), e)
    model = wire(model, ("Lf", "p2"), ("Rload", "p1"), e)
    model = wire(model, ("Rload", "p2"), ("gnd", "p"), e)

    model = add(model, "VINM", "converter.phys2sig", "converter", (460, 60),
                {"node": "VDCP"})
    model = add(model, "REF", "source.const", "signal-flow", (360, -60), {"value": VREF})
    model = add(
        model, "FF", "extensibility.pyfunc", "signal-flow", (240, -20),
        {"path": blocks_ref, "function": "duty_feedforward",
         "input_names": ["v_ref", "v_in"], "output_names": ["duty", "headroom"]},
    )
    model = add(
        model, "SLEW", "extensibility.pyblock", "signal-flow", (140, -20),
        {"path": blocks_ref, "input_names": ["duty"], "output_names": ["duty"]},
    )
    model = add(model, "HEAD", "control.gain", "signal-flow", (240, -120), {"k": 1.0})
    model = add(model, "MOD", "power.pwm1", "signal-flow", (40, -20), {"freq": FSW})
    model = add(model, "GATE", "converter.sig2phys", "converter", (40, 60),
                {"domain": "voltage"})

    s = "signal"
    model = wire(model, ("REF", "out"), ("FF", "in0"), s)
    model = wire(model, ("VINM", "out"), ("FF", "in1"), s)
    model = wire(model, ("FF", "out0"), ("SLEW", "in0"), s)
    # The *second* pyfunc output, by its generated name -- a wrong name is a MissingSymbol.
    model = wire(model, ("FF", "out1"), ("HEAD", "in"), s)
    model = wire(model, ("SLEW", "out0"), ("MOD", "in"), s)
    model = wire(model, ("MOD", "main"), ("GATE", "in"), s)
    model = wire(model, ("GATE", "out"), ("DSW", "g"), e)
    return model


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    blocks_path = os.path.join(here, "pyblock_buck_control_blocks.py")
    with open(blocks_path, "w") as fh:
        fh.write(BLOCKS_PY.format(slew=SLEW_PER_SECOND))

    # The emitted `path=` is the *basename*, so the checked-in netlist stays portable: the
    # simulator resolves it next to wherever the .cir is run from.
    blocks_ref = os.path.basename(blocks_path)
    model = build(blocks_ref)
    result = json.loads(gsim_py.compile(model, "general-simulator"))
    netlist = result["netlist"]
    assert result["notes"] == [], result["notes"]

    assert f"FF kind=pyfunc path={blocks_ref} function=duty_feedforward " \
           "inputs=REF,VINM outputs=FF,FF_headroom" in netlist, netlist
    assert f"SLEW kind=pyblock path={blocks_ref} inputs=FF" in netlist, netlist
    assert "HEAD kind=gain k=1 in=FF_headroom" in netlist, netlist
    assert "MOD kind=pwm freq=100000 in=SLEW outputs=MOD,MOD_comp" in netlist, netlist

    out_path = os.path.join(here, "pyblock_buck_control.cir")
    with open(out_path, "w") as fh:
        fh.write(netlist)
    print(f"wrote {out_path} ({len(netlist.splitlines())} lines)")

    simulator = os.environ.get("GSIM_SIMULATOR") or shutil.which("general-simulator")
    if not simulator:
        print("general-simulator not found (set GSIM_SIMULATOR) -- compiled only, not run")
        return

    # `kind=pyfunc`/`kind=pyblock` resolve `path=` through the embedded interpreter, which on
    # this machine cannot import from a directory whose name contains a space (the simulator's
    # own CLI tests carry the same workaround). Run from a space-free copy.
    with tempfile.TemporaryDirectory(prefix="gsim-pyblock-") as tmp:
        shutil.copyfile(blocks_path, os.path.join(tmp, blocks_ref))
        tmp_cir = os.path.join(tmp, "circuit.cir")
        with open(tmp_cir, "w") as fh:
            fh.write(netlist)
        proc = subprocess.run(
            [simulator, tmp_cir, "--mode", "transient", "--tfinal", "1e-2", "--dt", "5e-8"],
            capture_output=True,
            text=True,
            # `path=` is relative, and the embedded interpreter resolves it against the
            # process CWD -- so the run happens *from* the space-free temp dir.
            cwd=tmp,
        )
        if proc.returncode != 0:
            print(proc.stderr[:2000], file=sys.stderr)
            raise SystemExit(f"general-simulator failed with exit code {proc.returncode}")
        rows = proc.stdout.splitlines()

    header = rows[0].split(",")
    print(f"ran clean: {len(rows) - 1} steps, {len(header)} columns")
    tail = [r.split(",") for r in rows[-4000:]]
    vout = sum(float(r[header.index("V(VOUT)")]) for r in tail) / len(tail)
    duty = float(tail[-1][header.index("SLEW")])
    head = float(tail[-1][header.index("HEAD")])
    print(f"V(VOUT) mean over the last 4000 steps: {vout:.4f} V (feedforward target {VREF} V)")
    print(f"SLEW (slew-limited duty): {duty:.4f}   HEAD (FF_headroom via gain): {head:.4f} V")
    # duty_feedforward(12, 24) = 0.5 -- if the inputs= order were reversed it would be 2.0,
    # clamped to 0.95, and V(VOUT) would sit near 24 V instead.
    assert abs(duty - VREF / VIN) < 1e-6, duty
    assert abs(head - (VIN - VREF)) < 1e-6, head
    assert abs(vout - VREF) < 0.5, vout


if __name__ == "__main__":
    main()
