"""Proof case: is `gsim-py` mature enough to author a component-library "stamp"?

Builds a 3-phase (6-switch) ideal-switch bridge -- the standard power-electronics
drawing convention: each leg is a vertical stack (high-side switch on top, low-side
switch on bottom, phase tap between them), three legs arranged left-to-right, DC+/DC-
rails shared across all three legs top and bottom. This is a "stamp": a fixed-position,
pre-wired block of components meant to be copied into a parent schematic, not a nested
subsystem reference (gsim-core's document/builder has no subsystem-authoring support
yet -- see docs/decisions/0001-scope-and-naming.md and the 2026-09 journal entries).

Exposes exactly 11 boundary attachment points, matching the user's own count:
6 gate drives (one per switch) + 3 phase taps (one per leg) + DC+ / DC- = 11.

Run with the gsim-py extension built (`maturin develop` from crates/gsim-py, inside a
venv) -- see the repo README for the exact steps. Prints the boundary-port manifest and
writes h_bridge_3phase.gsim_sch next to this script.
"""

import json
import os

import gsim_py

GRID = 10
LEG_SPACING = 150
SWITCH_W, SWITCH_H = 40, 60
# Rail y-offsets, purely for wire waypoints (visual routing) -- the DC+ rail runs in a
# straight line above all three legs, DC- below, matching the standard convention rather
# than however a generic router would connect three separate switch drains/sources.
DC_PLUS_Y = -30
DC_MINUS_Y = SWITCH_H + 40 + SWITCH_H + 30  # below the low-side switches


def leg_positions(index: int) -> tuple[tuple[int, int], tuple[int, int]]:
    x = index * LEG_SPACING
    high = (x, 0)
    low = (x, SWITCH_H + 40)  # gap between high and low leaves room for the phase tap
    return high, low


def add_switch(model_json: str, block_id: str, position: tuple[int, int]) -> str:
    spec = {
        "type": "passive.switch.ideal",
        "paradigm": "physical-network",
        "domain": "electrical",
        "position": list(position),
        "rotation": 0,
        "id": block_id,
        # Defaults matching crates/gsim-core/tests/fixtures/general_simulator/passive.switch.ideal.json
        "params": {
            "r_on": 0.1,
            "g_breakdown": 0.0,
            "v_breakdown": -100.0,
            "g_off": 1e-9,
            "v_th": 0.6,
            "g_on": 5.0,
        },
    }
    result = json.loads(gsim_py.add_block(model_json, json.dumps(spec)))
    assert result["block_id"] == block_id
    return json.dumps(result["model"])


def connect(model_json: str, from_ep: dict, to_ep: dict, waypoints: list | None = None) -> str:
    opts = {"waypoints": waypoints or []}
    result = json.loads(
        gsim_py.connect(
            model_json,
            json.dumps(from_ep),
            json.dumps(to_ep),
            "physical-electrical",
            json.dumps(opts),
        )
    )
    return json.dumps(result["model"])


def build() -> tuple[str, dict]:
    model = gsim_py.new_model("h_bridge_3phase", "3-Phase Ideal-Switch Bridge", GRID)

    legs = ["a", "b", "c"]
    high_ids = [f"leg{leg}_high" for leg in legs]
    low_ids = [f"leg{leg}_low" for leg in legs]

    for i, leg in enumerate(legs):
        high_pos, low_pos = leg_positions(i)
        model = add_switch(model, high_ids[i], high_pos)
        model = add_switch(model, low_ids[i], low_pos)

    # Leg-internal wires: high-side source -> low-side drain = the phase tap net.
    for i in range(3):
        high_x, high_y = leg_positions(i)[0]
        model = connect(
            model,
            {"block": high_ids[i], "port": "s"},
            {"block": low_ids[i], "port": "d"},
            waypoints=[],
        )

    # DC+ rail: chain the three high-side drains together along one straight top line
    # (waypoints route the wire out to the rail y-offset and back in, rather than a
    # diagonal line jumping straight from one leg to the next).
    for i in range(2):
        x0 = leg_positions(i)[0][0] + SWITCH_W // 2
        x1 = leg_positions(i + 1)[0][0] + SWITCH_W // 2
        model = connect(
            model,
            {"block": high_ids[i], "port": "d"},
            {"block": high_ids[i + 1], "port": "d"},
            waypoints=[[x0, DC_PLUS_Y], [x1, DC_PLUS_Y]],
        )

    # DC- rail: same idea, chaining the three low-side sources along the bottom.
    for i in range(2):
        x0 = leg_positions(i)[1][0] + SWITCH_W // 2
        x1 = leg_positions(i + 1)[1][0] + SWITCH_W // 2
        model = connect(
            model,
            {"block": low_ids[i], "port": "s"},
            {"block": low_ids[i + 1], "port": "s"},
            waypoints=[[x0, DC_MINUS_Y], [x1, DC_MINUS_Y]],
        )

    # The 11 boundary attachment points. Each names one (block_id, port) pair a caller
    # wires an external connection to. dc_plus/dc_minus/phase_* point at one specific
    # member of their respective net (any port on the same net is electrically
    # equivalent -- a wire endpoint accepts more than one wire, which is exactly how a
    # 3-way tie/fan-out is expressed in this model -- but a stamp's external interface
    # should name one canonical point so callers don't have to know that).
    boundary_ports = {
        "gate_a_high": {"block": high_ids[0], "port": "g"},
        "gate_a_low": {"block": low_ids[0], "port": "g"},
        "gate_b_high": {"block": high_ids[1], "port": "g"},
        "gate_b_low": {"block": low_ids[1], "port": "g"},
        "gate_c_high": {"block": high_ids[2], "port": "g"},
        "gate_c_low": {"block": low_ids[2], "port": "g"},
        "phase_a": {"block": high_ids[0], "port": "s"},
        "phase_b": {"block": high_ids[1], "port": "s"},
        "phase_c": {"block": high_ids[2], "port": "s"},
        "dc_plus": {"block": high_ids[0], "port": "d"},
        "dc_minus": {"block": low_ids[0], "port": "s"},
    }
    assert len(boundary_ports) == 11

    return model, boundary_ports


def main() -> None:
    model, boundary_ports = build()

    document = gsim_py.save_document(model)
    # Round-trip: proves the produced document is genuinely schema-valid, not just
    # "whatever ModelBuilder happened to accumulate."
    reloaded = gsim_py.load_document(document)
    assert json.loads(reloaded)["schema_version"] == 2

    parsed = json.loads(reloaded)["model"]
    assert len(parsed["blocks"]) == 6, "expected exactly 6 ideal switches"
    assert len(parsed["wires"]) == 7, "expected 3 leg + 2 DC+ chain + 2 DC- chain wires"

    out_path = os.path.join(os.path.dirname(__file__), "h_bridge_3phase.gsim_sch")
    with open(out_path, "w") as f:
        f.write(document)

    print(f"wrote {out_path}")
    print(f"blocks: {len(parsed['blocks'])}, wires: {len(parsed['wires'])}")
    print("boundary ports (11):")
    for name, ep in boundary_ports.items():
        print(f"  {name:12s} -> {ep['block']}.{ep['port']}")


if __name__ == "__main__":
    main()
