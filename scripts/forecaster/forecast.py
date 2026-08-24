#!/usr/bin/env python3
"""One-shot TimesFM 2.5 forecaster. Reads a batch on stdin, writes one on stdout.

Runs on the M1 mini (see scripts/forecaster-bootstrap-m1.sh). Deliberately NOT a
daemon: the spike measured a 2.06 s warm start against 1.93 GB resident, and a
weekly sweep pays that once rather than holding the memory all week on a machine
that also serves Ollama.

Protocol, both directions newline-free JSON:

  in   [{"series_id": "...", "values": [1.0, ...], "horizon": 7}, ...]
  out  [{"series_id": "...", "point": [...], "p10": [...], "p90": [...]}, ...]

Anything that goes wrong exits non-zero with a message on stderr. The caller
(crates/goose/src/forecaster/remote.rs) treats every failure the same way: fall
back to the Rust baseline and relabel the method. It never reports a TimesFM
forecast that TimesFM did not produce.
"""
import json
import sys

MAX_CONTEXT = 1024
MAX_HORIZON = 256
CHECKPOINT = "google/timesfm-2.5-200m-pytorch"


def fail(msg: str) -> "NoReturn":  # noqa: F821
    print(msg, file=sys.stderr)
    sys.exit(1)


def main() -> None:
    try:
        batch = json.load(sys.stdin)
    except Exception as e:  # noqa: BLE001
        fail(f"could not read the batch: {e}")
    if not isinstance(batch, list) or not batch:
        fail("the batch must be a non-empty list")

    reqs = []
    for item in batch:
        try:
            sid = str(item["series_id"])
            values = [float(v) for v in item["values"]]
            horizon = int(item["horizon"])
        except (KeyError, TypeError, ValueError) as e:
            fail(f"malformed request: {e}")
        if horizon < 1 or horizon > MAX_HORIZON:
            fail(f"{sid}: horizon {horizon} outside 1..{MAX_HORIZON}")
        if len(values) < 32:
            # TimesFM 2.5's input patch is 32. Below two patches it sees almost
            # nothing, and the Rust side already refuses far earlier than this;
            # this is the backstop, not the gate.
            fail(f"{sid}: {len(values)} points is below one input patch")
        if any(v != v or v in (float("inf"), float("-inf")) for v in values):
            fail(f"{sid}: a non-finite value reached the model")
        reqs.append((sid, values[-MAX_CONTEXT:], horizon))

    import numpy as np
    import timesfm

    model = timesfm.TimesFM_2p5_200M_torch.from_pretrained(CHECKPOINT)
    model.compile(
        timesfm.ForecastConfig(
            max_context=MAX_CONTEXT,
            max_horizon=MAX_HORIZON,
            normalize_inputs=True,
            # The continuous quantile head is the calibrated one. TimesFM
            # 1.0/2.0's quantile heads are explicitly uncalibrated and must not
            # be used for intervals.
            use_continuous_quantile_head=True,
        )
    )

    # `forecast` takes one horizon per call, so group by it — 20 projects x 5
    # series usually collapses to one or two calls, which is where the measured
    # 178 ms/series batching win comes from.
    by_horizon: "dict[int, list[tuple[str, list[float]]]]" = {}
    for sid, values, horizon in reqs:
        by_horizon.setdefault(horizon, []).append((sid, values))

    out = []
    for horizon, group in by_horizon.items():
        inputs = [np.asarray(v, dtype=np.float32) for _, v in group]
        point, quant = model.forecast(horizon=horizon, inputs=inputs)
        point = np.asarray(point)
        quant = np.asarray(quant)
        # Shape is (N, H, 10): column 0 is the mean, columns 1..9 are the 0.1
        # through 0.9 quantiles. Read it defensively — a checkpoint that
        # changes shape must fail loudly, not hand back the wrong column.
        if quant.ndim != 3 or quant.shape[2] < 10:
            fail(f"unexpected quantile shape {quant.shape}; refusing to guess a column")
        for i, (sid, _) in enumerate(group):
            out.append(
                {
                    "series_id": sid,
                    "point": [float(x) for x in point[i]],
                    "p10": [float(x) for x in quant[i, :, 1]],
                    "p90": [float(x) for x in quant[i, :, 9]],
                }
            )

    json.dump(out, sys.stdout)
    sys.stdout.flush()


if __name__ == "__main__":
    main()
