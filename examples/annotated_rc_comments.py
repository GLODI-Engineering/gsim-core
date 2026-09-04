"""Per-block authoring comments, from `gsim_py` through to the emitted `.cir` text.

A block may carry two independent, optional comment slots -- `comment_before` and
`comment_after` -- and the netlist compiler renders each as full-line `*` SPICE comment
lines immediately above and below that block's own card(s). This script proves the whole
path for real: comments authored in Python, compiled by `gsim_py.compile`, then asserted
*positionally* against the emitted netlist (the line directly above/below the card), not
merely "the text appears somewhere".

Both emitter sub-grammars are covered, because they are different code paths:

  * `R1` / `C1` -- SPICE **element** cards (`NAME node node value`)
  * `K1`        -- a signal-flow **`kind=`** block card (`NAME kind=gain k=... in=...`)

`K1` also carries a multi-line comment written as one Python string containing `\\n`: it
must come out as two separate `*` lines, since an embedded newline inside a single emitted
line would drop raw text where the line-oriented lexer expects a card.

The circuit itself is a textbook RC low-pass across a DC source, with an unconnected
constant-times-gain signal pair -- deliberately trivial, because the comments are the
subject here.

Run it:

    cd crates/gsim-py
    python3 -m venv .venv && source .venv/bin/activate   # any venv works
    maturin develop
    cd ../..
    GSIM_SIMULATOR=/path/to/general-simulator python3 examples/annotated_rc_comments.py
"""

import json
import os
import shutil
import subprocess
import sys

import gsim_py

VSRC = 5.0
R = 1000.0
C = 1e-6
GAIN = 2.5

R_BEFORE = ["series element of the RC low-pass", "1 kohm with C1 -> f_c = 159 Hz"]
R_AFTER = ["(tolerance is not modeled here)"]
C_AFTER = ["storage element -- ic defaults to 0 V"]
K_BEFORE = "loop gain\nre-tuned after the step test"


def add(model, block_id, type_, paradigm, position, params=None,
        comment_before=None, comment_after=None):
    spec = {
        "type": type_,
        "paradigm": paradigm,
        "domain": "electrical",
        "position": list(position),
        "rotation": 0,
        "id": block_id,
        "params": params or {},
    }
    if comment_before is not None:
        spec["comment_before"] = comment_before
    if comment_after is not None:
        spec["comment_after"] = comment_after
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
            json.dumps({"waypoints": []}),
        )
    )
    return json.dumps(r["model"])


def build():
    model = gsim_py.new_model("annotated_rc", "Annotated RC low-pass", 10)
    model = add(model, "Vin", "source.voltage.dc", "physical-network", (0, 0), {"V": VSRC})
    # A list of lines in each slot; both slots on the same block, independently.
    model = add(model, "R1", "passive.resistor", "physical-network", (60, 0), {"R": R},
                comment_before=R_BEFORE, comment_after=R_AFTER)
    # `comment_after` alone -- the two slots really are independent.
    model = add(model, "C1", "passive.capacitor", "physical-network", (120, 0), {"C": C},
                comment_after=C_AFTER)
    model = add(model, "gnd", "routing.ground", "physical-network", (60, 80))
    # A single string carrying its own newline, in the `kind=` sub-grammar.
    model = add(model, "K1", "control.gain", "signal-flow", (0, 160), {"k": GAIN},
                comment_before=K_BEFORE)
    model = add(model, "U1", "source.const", "signal-flow", (-80, 160), {"value": 1.0})

    model = connect(model, ("Vin", "p"), ("R1", "p1"), "physical-electrical")
    model = connect(model, ("R1", "p2"), ("C1", "p1"), "physical-electrical")
    model = connect(model, ("C1", "p2"), ("gnd", "p"), "physical-electrical")
    model = connect(model, ("Vin", "n"), ("gnd", "p"), "physical-electrical")
    model = connect(model, ("U1", "out"), ("K1", "in"), "signal")
    return model


def card_index(lines, prefix):
    """The single line starting with `prefix` -- ambiguity here would void the assertions."""
    hits = [i for i, line in enumerate(lines) if line.startswith(prefix)]
    assert len(hits) == 1, f"expected exactly one {prefix!r} card, got {hits}"
    return hits[0]


def main():
    model = build()

    # The fields round-trip through the .gsim_sch document JSON like any other Block field.
    parsed = json.loads(model)
    r1 = next(b for b in parsed["blocks"] if b["id"] == "R1")
    assert r1["comment_before"] == R_BEFORE, r1
    assert r1["comment_after"] == R_AFTER, r1
    vin = next(b for b in parsed["blocks"] if b["id"] == "Vin")
    assert "comment_before" not in vin and "comment_after" not in vin, vin

    result = json.loads(gsim_py.compile(model, "general-simulator"))
    netlist = result["netlist"]
    lines = netlist.splitlines()

    # --- sub-grammar 1: an element card, comments directly above and below it ---
    i = card_index(lines, "R1 ")
    assert lines[i - 2] == "* series element of the RC low-pass", lines[i - 3:i + 3]
    assert lines[i - 1] == "* 1 kohm with C1 -> f_c = 159 Hz", lines[i - 3:i + 3]
    assert lines[i + 1] == "* (tolerance is not modeled here)", lines[i - 3:i + 3]

    # `comment_after` with no `comment_before`: the line above C1 must be R1's own trailer,
    # not a stray comment leaking down from its neighbour.
    j = card_index(lines, "C1 ")
    assert lines[j - 1] == "* (tolerance is not modeled here)", lines[j - 3:j + 3]
    assert lines[j + 1] == "* storage element -- ic defaults to 0 V", lines[j - 3:j + 3]

    # An uncommented block gets nothing of its own: the only `*` lines in the whole netlist are
    # the two-line generated header plus exactly the five lines authored above.
    stars = [line for line in lines if line.startswith("*")]
    assert len(stars) == 2 + len(R_BEFORE) + len(R_AFTER) + len(C_AFTER) \
        + len(K_BEFORE.split("\n")), stars

    # --- sub-grammar 2: a `kind=` block card, multi-line comment split into two lines ---
    g = card_index(lines, "K1 kind=gain")
    assert lines[g - 2] == "* loop gain", lines[g - 3:g + 2]
    assert lines[g - 1] == "* re-tuned after the step test", lines[g - 3:g + 2]
    assert not lines[g + 1].startswith("*"), lines[g:g + 2]

    # Nothing authored ever reaches the netlist as an uncommented line.
    for line in lines:
        for needle in ("tolerance is not modeled", "re-tuned after", "1 kohm with"):
            assert needle not in line or line.startswith("*"), line

    print("comment placement verified around both card grammars:")
    print("\n".join(lines[i - 2:i + 2]))
    print("\n".join(lines[g - 2:g + 1]))

    out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "annotated_rc_comments.cir")
    with open(out_path, "w") as fh:
        fh.write(netlist)
    print(f"wrote {out_path} ({len(lines)} lines)")

    simulator = os.environ.get("GSIM_SIMULATOR") or shutil.which("general-simulator")
    if not simulator:
        print("general-simulator not found (set GSIM_SIMULATOR) -- compiled only, not run")
        return
    # The real point of running it: the comment lines must be inert to the parser.
    proc = subprocess.run(
        [simulator, out_path, "--mode", "transient", "--tfinal", "1e-2", "--dt", "1e-5"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print(proc.stderr[:2000], file=sys.stderr)
        raise SystemExit(f"general-simulator failed with exit code {proc.returncode}")
    rows = proc.stdout.splitlines()
    print(f"ran clean with the comments in place: {len(rows) - 1} steps")


if __name__ == "__main__":
    main()
