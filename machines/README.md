# The machine shop

Small workflows, called **machines**, each inspired by a physical machine: a domino run, a prism, a railway switch. Each one makes sense on its own and exercises one or a few engine functions. They run against a real engine: the API server, the orchestrator and the std worker, with real activities.

- **Simple machines** exercise one engine function.
- **Compound machines** combine about three. Interactions are where bugs hide.
- **A trap** is a machine of either kind that reproduces a known bug, labelled with its issue number. It's expected to fail until the fix lands. When it passes, take the label off.

## Running

```bash
DATABASE_URL=postgres://... cargo test -p kruxiaflow-api --test machine_shop -- --nocapture
```

There are two modes, one test each:

| Test                 | Mode  | What it does                                                |
|----------------------|-------|-------------------------------------------------------------|
| `machine_shop_bench` | bench | Runs the machines one at a time, so nothing else is running |
| `machine_shop_shop`  | shop  | Starts every run of every machine at once, in one engine    |

- **Pick machines:** set `MACHINE_SHOP_ONLY` to a comma-separated list of directory names, such as `MACHINE_SHOP_ONLY=level-crossing`, to run only those.
- **Scoreboard:** each mode prints one and writes it to `target/tmp/machine-shop/<mode>.md`.

Each run ends as one of four results:

| Result       | Meaning                                                  | Fails the test |
|--------------|----------------------------------------------------------|----------------|
| works        | It did what its card says                                | No             |
| broken       | It didn't, and it isn't a trap                           | Yes            |
| known broken | A trap that still shows its bug                          | No             |
| fixed        | A trap that now passes: take the trap label off its card | No             |

## A machine

Each machine is a directory with two files:

- **`workflow.yaml`** is an ordinary workflow definition, with a unique `name`.
- **`card.yaml`** is its spec card:

```yaml
name: Railway switch
kind: simple                # simple or compound
picture: >
  A lever moves the points at a fork in the line ...
functions:                  # the engine functions it exercises
  - a condition on a dependency, reading the dependency's output
trap:                       # only for a trap
  issue: 25
  broken_looks_like: >
    what the bug does to this machine
runs:                       # one or more runs, each with its own input
  - name: lever to the left
    input: { track: left }
    timeout_seconds: 15     # optional; default 15
    expect:
      status: completed     # the workflow's final status
      steps:                # every step, with its final status and run count
        lever: { status: completed, runs: 1 }
        right_track: { status: skipped, runs: 0 }
      outputs:              # optional checks of a step's named output
        - step: left_track
          output: echo
          equals: { train: arrived on the left }
```

Rules:
- **List every step in every run.** The runner checks the card against the workflow.
- **Write expectations from the docs and the intended behaviour,** never by running the machine and copying what it did.
- **Use real activities and their real output names.** For example, `echo` returns its parameters as the output `echo`.
- **Keep a machine correct whatever else is running.** Use its own inputs, and depend on nothing outside it.
