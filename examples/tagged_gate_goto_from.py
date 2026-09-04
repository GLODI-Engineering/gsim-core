"""Goto/From implicit connections, lowered away completely by the netlist compiler.

`docs/specs/gsim-sch-format.md` §10 defines a Simulink-style implicit connection: a
`routing.goto` block broadcasts whatever drives its `in` port under a string `tag`, and a
`routing.from` block -- which has no wire input at all -- resolves to the `Goto` sharing that
tag and behaves exactly as if it had been wired straight to *that* Goto's own driver. Two
distant parts of a schematic connect with no wire crossing the canvas.

Neither block is a device, so neither may appear in the emitted netlist. This script proves
the whole path end to end:

  1. A gate command (`CMD -> KGATE`) is broadcast under the tag `GATE_CMD` by `Ggate`.
  2. `Fgate` picks it up and feeds a `converter.sig2phys` gate driver -- there is **no wire**
     between `KGATE` and `GATE` anywhere in the document.
  3. `Fmon` picks up the *same* tag and feeds a monitor gain, proving one Goto fans out to
     many Froms.
  4. The compiled `.cir` names the real upstream driver (`in=KGATE`) at both consumers and
     contains no `Ggate`/`Fgate`/`Fmon`/`routing.*` token at all.

The strongest assertion here is the **twin**: the same circuit is built a second time with
`KGATE` wired directly to both consumers and no routing blocks whatsoever, and the two
netlists must come out byte-identical. A Goto/From pair is then provably nothing but a
drawing convenience.

Two negative cases are asserted too, because §10.3 makes them hard errors rather than
dangling inputs: a `From` whose tag matches nothing (`unresolved-from-tag`) and a tag
broadcast by two Gotos at once (`ambiguous-from-tag`).

The circuit itself is a textbook single-switch chopper: a 24 V rail through an ideal switch
into a 10 ohm load. The routed command is a steady 1.0, above the switch's 0.5 V threshold,
so the switch is closed for the whole run and V(SW) must settle at the r_on/Rload divider --
a wrong or missing gate reference would leave the switch open and V(SW) at 0.

Run it:

    cd crates/gsim-py
    python3 -m venv .venv && source .venv/bin/activate   # any venv works
    maturin develop
    cd ../..
    GSIM_SIMULATOR=/path/to/general-simulator python3 examples/tagged_gate_goto_from.py
"""

import json
import os
import shutil
import subprocess
import sys

import gsim_py

VDC = 24.0
RLOAD = 10.0
R_ON = 0.01
GATE_CMD = 1.0
MON_GAIN = 2.0
TAG = "GATE_CMD"

SWITCH_PARAMS = {
    "r_on": R_ON,
    "g_breakdown": 0.0,
    "v_breakdown": -500.0,
    "g_off": 1e-9,
    "v_th": 0.5,
    "g_on": 200.0,
}


def add(model, block_id, type_, paradigm, position, params=None, domain="electrical"):
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


def wire(model, a, b, kind, net=None):
    opts = {"waypoints": []}
    if net is not None:
        opts["net"] = net
    r = json.loads(
        gsim_py.connect(
            model,
            json.dumps({"block": a[0], "port": a[1]}),
            json.dumps({"block": b[0], "port": b[1]}),
            kind,
            json.dumps(opts),
        )
    )
    return json.dumps(r["model"])


def routing(model, block_id, type_, position, tag, visibility=None):
    params = {"tag": tag}
    if visibility is not None:
        params["visibility"] = visibility
    return add(model, block_id, type_, "routing", position, params, domain="signal")


def power_stage_and_control(model):
    """Everything both variants share, added in the same order so emission order matches."""
    e = "physical-electrical"
    model = add(model, "gnd", "routing.ground", "physical-network", (0, 300))
    model = add(model, "Vdc", "source.voltage.dc", "physical-network", (0, 150), {"V": VDC})
    model = add(model, "DSW", "passive.switch.ideal", "physical-network", (120, 100),
                SWITCH_PARAMS)
    model = add(model, "Rload", "passive.resistor", "physical-network", (240, 200),
                {"R": RLOAD})
    model = wire(model, ("Vdc", "p"), ("DSW", "d"), e, net="VDCP")
    model = wire(model, ("Vdc", "n"), ("gnd", "p"), e)
    model = wire(model, ("DSW", "s"), ("Rload", "p1"), e, net="SW")
    model = wire(model, ("Rload", "p2"), ("gnd", "p"), e)

    model = add(model, "CMD", "source.const", "signal-flow", (-200, 0), {"value": GATE_CMD})
    model = add(model, "KGATE", "control.gain", "signal-flow", (-100, 0), {"k": 1.0})
    model = wire(model, ("CMD", "out"), ("KGATE", "in"), "signal")
    return model


def consumers(model):
    """The two blocks that read the broadcast command, added after the routing blocks so that
    both variants list their `kind=` cards in the same order."""
    model = add(model, "GATE", "converter.sig2phys", "converter", (40, 60),
                {"domain": "voltage"})
    model = add(model, "MON", "control.gain", "signal-flow", (40, -80), {"k": MON_GAIN})
    model = wire(model, ("GATE", "out"), ("DSW", "g"), "physical-electrical")
    return model


def build_tagged(mon_tag=TAG):
    """`KGATE` broadcasts under a tag; two Froms pick it up. No wire joins the two halves.

    `mon_tag` exists only so the unresolved-tag negative case can be the *same* circuit with
    one From listening on a tag nobody broadcasts."""
    model = gsim_py.new_model("tagged_gate", "Tag-routed gate command", 10)
    model = power_stage_and_control(model)
    model = routing(model, "Ggate", "routing.goto", (0, 0), TAG, visibility="global")
    model = routing(model, "Fgate", "routing.from", (-40, 60), TAG)
    model = routing(model, "Fmon", "routing.from", (-40, -80), mon_tag)
    model = consumers(model)
    model = wire(model, ("KGATE", "out"), ("Ggate", "in"), "signal")
    model = wire(model, ("Fgate", "out"), ("GATE", "in"), "signal")
    model = wire(model, ("Fmon", "out"), ("MON", "in"), "signal")
    return model


def build_directly_wired():
    """The same circuit with real wires and no routing blocks at all -- the twin."""
    model = gsim_py.new_model("tagged_gate", "Tag-routed gate command", 10)
    model = power_stage_and_control(model)
    model = consumers(model)
    model = wire(model, ("KGATE", "out"), ("GATE", "in"), "signal")
    model = wire(model, ("KGATE", "out"), ("MON", "in"), "signal")
    return model


def compile_netlist(model):
    result = json.loads(gsim_py.compile(model, "general-simulator"))
    assert result["notes"] == [], result["notes"]
    return result["netlist"]


def expect_compile_error(model, keyword):
    try:
        gsim_py.compile(model, "general-simulator")
    except ValueError as exc:
        assert str(exc).startswith(f"{keyword}:"), str(exc)
        return str(exc)
    raise AssertionError(f"expected a {keyword} error, got a clean compile")


def main():
    model = build_tagged()

    # The routing blocks really are in the document -- this is a lowering pass, not a builder
    # that refuses to store them.
    parsed = json.loads(model)
    types = [b["type"] for b in parsed["blocks"]]
    assert types.count("routing.goto") == 1 and types.count("routing.from") == 2, types
    # ...and no wire ever joins KGATE to either consumer.
    for w in parsed["wires"]:
        pair = {w["from"]["block"], w["to"]["block"]}
        assert pair != {"KGATE", "GATE"} and pair != {"KGATE", "MON"}, w

    netlist = compile_netlist(model)

    # Both consumers name the real upstream driver, reached only through the tag.
    assert "GATE kind=sig2phys domain=voltage in=KGATE" in netlist, netlist
    assert "MON kind=gain k=2 in=KGATE" in netlist, netlist
    assert "DSW kind=ideal_switch" in netlist and "ctrl=GATE" in netlist, netlist
    # Zero netlist footprint for either routing block.
    for token in ("Ggate", "Fgate", "Fmon", "routing.goto", "routing.from", TAG):
        assert token not in netlist, (token, netlist)

    # The twin: implicit and explicit connections compile to exactly the same text.
    twin = compile_netlist(build_directly_wired())
    assert netlist == twin, "\n--- tagged ---\n" + netlist + "\n--- direct ---\n" + twin
    print("tag-routed and directly-wired variants compile to byte-identical netlists")

    # --- §10.3's two hard errors ------------------------------------------------------------
    stray = build_tagged(mon_tag="NO_SUCH_TAG")
    msg = expect_compile_error(stray, "unresolved-from-tag")
    assert 'Unresolved From tag "NO_SUCH_TAG"' in msg, msg
    print(f"unresolved: {msg}")

    dup = build_tagged()
    dup = routing(dup, "Ggate2", "routing.goto", (0, 40), TAG)
    dup = wire(dup, ("KGATE", "out"), ("Ggate2", "in"), "signal")
    msg = expect_compile_error(dup, "ambiguous-from-tag")
    assert f'Ambiguous From tag "{TAG}"' in msg and "Ggate, Ggate2" in msg, msg
    print(f"ambiguous:  {msg}")

    out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "tagged_gate_goto_from.cir")
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
    sw = float(final[header.index("V(SW)")])
    expected = VDC * RLOAD / (RLOAD + R_ON)
    print(f"ran clean: {len(rows) - 1} steps, {len(header)} columns")
    print(f"V(SW)={sw:.6f} V (closed-switch divider {expected:.6f} V)")
    # A gate reference that failed to reach the real driver would leave the switch open and
    # V(SW) pinned at 0 -- this is the end-to-end proof that the tag carried a live signal.
    assert abs(sw - expected) < 1e-3, sw


if __name__ == "__main__":
    main()
