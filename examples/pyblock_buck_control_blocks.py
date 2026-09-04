"""Controller blocks for examples/pyblock_buck_control.py.

`duty_feedforward` is the `kind=pyfunc` contract: a plain function, called positionally with
one argument per declared `inputs=` entry, returning one value per declared `outputs=` entry.

`start`/`output`/`update` are the `kind=pyblock` contract: a stateful block. This one is a
slew-rate limiter, i.e. a soft start -- the duty command may only rise so fast, which is a
genuine thing a real converter controller does and a stateless function cannot express.
"""

SLEW_PER_SECOND = 400.0


def duty_feedforward(v_ref, v_in):
    """Open-loop buck feedforward: d = Vref/Vin, plus the headroom left over."""
    if v_in <= 1e-9:
        return 0.0, 0.0
    duty = v_ref / v_in
    duty = min(max(duty, 0.0), 0.95)
    return duty, v_in - v_ref


def start():
    return {"duty": 0.0}


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
