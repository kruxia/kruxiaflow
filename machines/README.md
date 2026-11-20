# The machine shop

Small workflows, called **machines**, each inspired by a physical machine: a domino run, a prism, a railway switch. Each one makes sense on its own and exercises one or a few engine functions. They run against a real engine: the API server, the orchestrator and the std worker, with real activities.

- **Simple machines** exercise one engine function.
- **Compound machines** combine about three. Interactions are where bugs hide.
- **A trap** is a machine of either kind that reproduces a known bug, labelled with its issue number, or several when it needs them all fixed. It's expected to fail until the fix lands. When it passes, take the label off.

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
- **Results file:** each mode also writes `results/<tree>-<mode>-<time>.json` there. They accumulate, so runs on different trees and days can be compared.
- **Views:** each mode then writes every page in `machines/views/` next to the results, with the machines and all the results files embedded. Open it from disk; it needs no server and loads nothing from the network. `machine_shop_views` rewrites the pages from the results already there, without running the machines.

Environment variables:

| Variable            | Default                   | What it does                                                      |
|---------------------|---------------------------|-------------------------------------------------------------------|
| `MACHINE_SHOP_ONLY` | every machine             | Runs only these machines (comma-separated directory names)        |
| `MACHINE_SHOP_OUT`  | `target/tmp/machine-shop` | Where scoreboards, results and views go; share one across trees   |
| `MACHINE_SHOP_TREE` | the branch name           | The tree's label in the results, such as `main` or `merged-fixes` |

To compare two trees, run the shop on each with the same `MACHINE_SHOP_OUT` and different `MACHINE_SHOP_TREE` labels. The views written by the second run show both.

### Views: two prototypes

There are two views of the same results, built to compare before choosing one. Neither is part of the engine's own dashboard, and they share nothing with it.

| Page         | What it is                                                                                                                                                                                                                                                                                                                                                  |
|--------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `stack.html` | **The Stack:** one card per machine, in homage to HyperCard. A 1-bit look, a pixel picture and pixel title for each machine, its story and functions, and a stamp for each run in each results set. Arrow keys flip cards; Find searches the stack; a compare card lists runs whose result changed.                                                         |
| `floor.html` | **The Shop Floor:** every machine on one page, each drawn as a schematic of its own workflow, with every step lit by what it did: completed, skipped, never ran or failed. A red mark shows a step that didn't do what the card expects; a blue mark shows a step that changed since the results set you compare with. Uses the engine dashboard's colours. |

The results file (`machine-shop-results/1`) is the contract between runners and views:

- `runner`, `mode`, `engine_version`, `tree` (`label`, `branch`, `commit`), `started_at`, `seconds`
- `runs`: one entry per run, with `machine`, `run`, `result` (`works`, `broken`, `known_broken` or `fixed`), `trap` (a trap's first issue, or null), `trap_issues` (all of them, or empty), `seconds`, the workflow's final `status`, `steps` (each step's `status` and `runs`), and `problems`
- Readers treat a file without `trap_issues` as having `[trap]`, so older results files still read

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
  issue: 25                 # or issues: [29, 37], when it needs them all fixed
  broken_looks_like: >
    what the bug does to this machine
runs:                       # one or more runs, each with its own input
  - name: lever to the left
    input: { track: left }
    timeout_seconds: 15     # optional; default 15; covers the whole run
    signals:                # optional; see "Signals" below
      - step: approve
        event: go
        data: { approved: true }
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

### Signals

A step with `settings.wait_for_signal` waits until it is signaled or its wait times out. A run's `signals` are sent in order, each one like this:

1. The runner polls `GET /api/v1/workflows/{id}` until the step's status is `waiting`.
2. It sends `POST /api/v1/workflows/{id}/signal` with `activity_key`, `event_name` and, when the card gives one, `data`. It tries again while the step is still waiting and the API answers `signaled: false`.
3. Then it goes on to the next signal, or waits for the workflow to finish.

| Field   | What it is                                                                                         |
|---------|----------------------------------------------------------------------------------------------------|
| `step`  | The step to signal; it must exist and have `settings.wait_for_signal`                              |
| `event` | The `event_name` to send                                                                           |
| `data`  | Optional. Left out, the request has no `data` field; `data: null` sends an explicit null           |

A signal that can't be sent is one of the run's problems: the workflow finished first, the step got past waiting without one, or the run's timeout passed. The signals after it aren't sent. A run with no `signals` relies on the step's own timeout and `on_timeout`.

The runner's orchestrator checks for expired waits every second (the engine's default is 30 seconds), so a run that waits out a short signal timeout finishes in a few seconds. Give such a run a `timeout_seconds` above the wait's own timeout.
