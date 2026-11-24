# The machine shop

Small workflows, called **machines**, each inspired by a physical machine: a domino run, a prism, a railway switch. Each one makes sense on its own and exercises one or a few engine functions. They run against a real engine: the API server, the orchestrator and the std worker, with real activities.

**The inspiration is Rube Goldberg machines:** contraptions that do a simple job through a chain of parts, each one setting off the next. A marble rolls down a ramp, tips a lever, knocks over a row of dominoes, and rings a bell. A workflow is that chain: each step starts when the steps it `depends_on` finish. A fault anywhere along it shows up the way it does in a contraption: the bell never rings, or rings twice. The simplest machines here are a Rube Goldberg machine's parts (the domino run, the funnel, the switch), and the others borrow how real machines behave (a pump's strokes, a thermostat's cycles, a revolving door's turns). So what a workflow should do is plain from the picture, and a bug looks like a jam.

The name is still open. "Machine shop" was the first one. "Makerspace" may fit better: it's where people build contraptions like these from shared parts, and it invites anyone to build a machine of their own.

The shop is an R&D exploration. Its opportunity and value are still being discovered: which machines, modes and views earn a place, and what they show about the engine that nothing else does.

- **Simple machines** exercise one engine function.
- **Compound machines** combine about three. Interactions are where the most interesting behaviour shows up.
- **A trap** is a machine of either kind that reproduces a known bug, labelled with its issue number, or several when it needs them all fixed. It's expected to fail until the fix lands. When it passes, take the label off. A trap can name the runs that show its bug; the card's other runs are judged as works or broken.

## Running

The shop is a developer tool, separate from the engine: its own crate, `kruxiaflow-machine-shop` (`machines/Cargo.toml`), a member of the workspace that's never published. Its runner is the integration test `machines/tests/machine_shop.rs`, which starts an engine in-process from the workspace's crates. Nothing in it is part of the `kruxiaflow` binary or its Docker image, so it runs from a clone of the repository.

```bash
DATABASE_URL=postgres://... cargo test -p kruxiaflow-machine-shop --test machine_shop -- --nocapture
```

There are two modes, one test each:

| Test                 | Mode  | What it does                                                |
|----------------------|-------|-------------------------------------------------------------|
| `machine_shop_bench` | bench | Runs the machines one at a time, so nothing else is running |
| `machine_shop_shop`  | shop  | Starts every run of every machine at once, in one engine    |

- **Pick machines:** set `MACHINE_SHOP_ONLY` to a comma-separated list of directory names, such as `MACHINE_SHOP_ONLY=level-crossing`, to run only those.
- **Scoreboard:** each mode prints one and writes it to `target/tmp/machine-shop/<mode>.md`, with each run's result and footprint, a footprint per machine, and the twins' outcomes in plug mode.
- **Results file:** each mode also writes `results/<tree>-<mode>-<time>.json` there. They accumulate, so runs on different trees and days can be compared.
- **Views:** each mode then writes every page in `machines/views/` next to the results, with the machines and all the results files embedded. Open it from disk; it needs no server and loads nothing from the network. `machine_shop_views` rewrites the pages from the results already there, without running the machines.

Environment variables:

| Variable                  | Default                   | What it does                                                                                     |
|---------------------------|---------------------------|--------------------------------------------------------------------------------------------------|
| `MACHINE_SHOP_ONLY`       | every machine             | Runs only these machines (comma-separated directory names)                                       |
| `MACHINE_SHOP_OUT`        | `target/tmp/machine-shop` | Where scoreboards, results and views go; share one across trees and hosts                        |
| `MACHINE_SHOP_TREE`       | the branch name           | The tree's label in the results, such as `main` or `merged-fixes`                                |
| `MACHINE_SHOP_HOST`       | `<os>/<arch>`             | Where it ran, such as `pi-5` or `m4`; the default is `std::env::consts`, such as `macos/aarch64` |
| `MACHINE_SHOP_PLUG`       | off                       | `on`, or a seed (a whole number), gives every run a twin that has the plug pulled; see below     |
| `MACHINE_SHOP_PLUG_GRACE` | `75`                      | Seconds past its card's timeout that a twin may take, when the uninterrupted run finished        |

To compare two trees, run the shop on each with the same `MACHINE_SHOP_OUT` and different `MACHINE_SHOP_TREE` labels. The views written by the second run show both. The same works across hosts: the host label is all the results record about the machine, never its hostname or user.

### Footprint

Each run records what it cost, read from the test database when it ends:

| Field                     | What it is                                                                                              |
|---------------------------|---------------------------------------------------------------------------------------------------------|
| `seconds`                 | Wall time, from submitting the workflow until it ended or the run's timeout passed                      |
| `events`                  | Rows in `workflow_events` for the workflow                                                              |
| `payload_bytes`           | Those events' payloads as stored (`pg_column_size` of the jsonb), without row headers or indexes        |
| `queue_rows`              | Rows in `activity_queue` for the workflow: one per step and round, since a retry reuses its row         |
| `peak_engine_connections` | The most connections the engine held on the test database at once, sampled every 25 ms; bench mode only |

The engine's connections are the ones whose `application_name` is `kruxiaflow-machine-shop-engine`: its pool and its `LISTEN` connection, not the runner's, and not those a machine's `postgres_query` opens. Shop-mode runs overlap, so their connections are reported only for the whole set. A sample can miss a peak shorter than 25 ms. The engine's pool is the size of the test's pool (5).

### Pull the plug

**Pulling the plug tests durable execution:** a workflow should survive the engine stopping at any moment, and pick up where it left off once the engine is back. `MACHINE_SHOP_PLUG=on` gives every run a twin. After the uninterrupted runs, the runner starts a fresh engine on the same database and runs each twin, and partway through, it pulls the plug:

1. **It stops the engine abruptly.** The API server, orchestrator and std worker run in a Tokio runtime of their own, with their own connection pool. The runner shuts that runtime down at once, which drops every task the engine spawned: the poller, the partition handlers, the timeout checker, and any activity a worker is running.
2. **It waits until PostgreSQL has seen the engine's connections go,** then reads how many events the twin has.
3. **It restarts the engine** on the same database and the same port, and the twin runs to the end. A request the runner had in flight gets no answer; it asks again.
4. **It compares the twin's outcome with the uninterrupted run's:** the workflow's status, each step's status and run count, and each output the card checks.

**What this simulates, exactly:** aborting in-process tasks, which drops their connections and rolls back their open transactions. It is not a process kill, since the runner's own process carries on, and it is not a database power loss: PostgreSQL keeps running throughout, and nothing it committed is lost.

Where the plug is pulled:
- **Bench mode:** once the twin has `k` events, where `k` is picked from the seed, the machine and the run, between 1 and one less than the uninterrupted run's events. The same seed picks the same points; another seed picks others.
- **Shop mode:** once, for every twin at once, when the twins together have a share of the uninterrupted runs' events, picked from the seed between 25% and 75%. Each twin records how many of its own events were in when the plug was pulled.

Each twin ends as one of four outcomes:

| Outcome   | Meaning                                                                                          |
|-----------|--------------------------------------------------------------------------------------------------|
| identical | Same workflow status, step statuses and run counts, and checked outputs as the uninterrupted run |
| different | Something differs; the differences are listed                                                    |
| hung      | The uninterrupted run finished, and the twin hadn't within its card's timeout plus the grace     |
| missed    | The plug wasn't pulled mid-run: the twin had already finished                                    |

A twin that hasn't finished but doesn't match yet is read again until it matches, ends, or its card's timeout plus the grace passes; this matters most for a twin of a run that never finished, which stops waiting for the workflow to end five seconds past the card's timeout. A different or hung twin doesn't fail the test: it's a candidate engine finding, a place where durable execution doesn't hold yet, printed as a `PLUG:` line and listed in the scoreboard, with the twin's activity queue rows added to its differences.

**Recovery takes time, and the twin's footprint shows it.** A step whose worker was stopped mid-activity keeps its claim on the queue row until the activity's timeout runs out (the engine's default is 60 seconds), and only then runs again. So a twin can come out identical and still take a minute longer, and the grace is 75 seconds by default.

### Views

There are two views of the same results, each for its own audience: the Shop Floor for developers working on edge computing, and the Stack for electrical engineers and engineers who read circuits, timing diagrams and datasheets. Neither is part of the engine's own dashboard, and they share nothing with it.

| Page         | What it is                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
|--------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `stack.html` | **The Stack:** one card per machine, in homage to HyperCard. A 1-bit look, a pixel picture and pixel title for each machine, its story and functions, and a stamp for each run in each results set. Datasheet, ladder, timing and code sheets too (below). Clicking a coil on the ladder, or a step's name on a timing diagram, opens the machine's `workflow.yaml` with that step's activity highlighted. Arrow keys flip cards; Find searches the stack; a compare card lists runs whose result changed.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `floor.html` | **The Shop Floor:** every machine on one page, each drawn as a schematic of its own workflow, with every step lit by what it did: completed, skipped, never ran or failed. A red mark shows a step that didn't do what the card expects; a blue mark shows a step that changed since the results set you compare with. Written for edge developers: each bay shows the machine's edge reading, each run's footprint and the machine's added up, and, in plug mode, a plug mark per run and per machine (green identical, red different, orange hung, grey missed), with a tally and the twin's details. With results from several hosts, each bay shows the machine's latest result on each. Under each bay, one disclosure triangle opens the card (its story, engine functions and recent results) and another opens the machine's `workflow.yaml`; clicking a step in the schematic opens the source with that step's activity highlighted. The button at a bay's top right maximizes that machine: the bay fills the page with its schematic drawn larger (up to twice its size, as far as the window's height allows), its card and source beside it on a wide screen, and the URL names it (`floor.html#max-thermostat`) so a link opens it there. Esc or the same button goes back to the floor; F, or the button beside it, goes full screen. A note says that the page works offline and that the engine needs only PostgreSQL. Uses the engine dashboard's colours. |


**Terms.** Both pages share a glossary, `views/glossary.json`: for each word the pages use, what it means in Kruxia Flow, and links to the docs that explain it. Kruxia Flow's own words (step, `depends_on`, signal, round...) are listed apart from the analogies (schematic, coil, pinout, title block...), and an analogy also says what the word means in hardware. **Explain terms** on the Floor, or the **?** button in the Stack's tool bar, marks the first use of each word in each bay or panel; pointing at one, focusing it or tapping it opens a card with its meaning and docs links. The choice is remembered in the browser. The Floor lists every term under its notes, and the Stack on its legend card. The runner puts the glossary into each page, and `glossary_is_valid` checks that every term is complete and every docs link names a file and heading in the tree.

`views/rendered/` holds both pages already rendered, so you can open them in a browser without running anything. They are a snapshot from 2026-11-24 on one host (m4), with eight results sets: two trees, each in bench and shop mode, plain and with the plug pulled. `main` is the engine as released (v0.12.6); `mecha` is `main` with the open fix branches merged. The snapshot isn't kept current: a run writes fresh pages into `MACHINE_SHOP_OUT`.

#### The Stack's sheets for engineers

Each machine card has five sheets, picked with the buttons at its top; the arrow keys keep the sheet while they flip machines, and the URL names it (`stack.html#thermostat/ladder`). On the code sheet the URL can name a step too (`stack.html#thermostat/code/heater`), which opens the sheet with that activity lit; clicking an activity puts it there. A legend card, after the compare card, explains every symbol.

| Sheet     | What it shows                                                                                                                                                                                                                                                                                                                                                                                              |
|-----------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Card      | The story, functions, runs and notes                                                                                                                                                                                                                                                                                                                                                                       |
| Datasheet | Features (the card's functions), a pinout, absolute maximum ratings, min/typ/max characteristics over every run in every results set, and one run's timing diagram                                                                                                                                                                                                                                         |
| Ladder    | One rung per step, drawn from `workflow.yaml`                                                                                                                                                                                                                                                                                                                                                              |
| Timing    | A logic-analyzer trace per run of one results set: a channel per step, high from `ActivityScheduled` to `ActivityCompleted` or `ActivityFailed`, dashed mid-level while it waits for a signal, an active-low `/INT` line for signals taken, and a time axis in ms or s. In plug mode each run's twin is drawn under it, with the engine's downtime hatched as `PLUG` and a stamp for how the twin came out |
| Code      | The machine's whole `workflow.yaml`, one block per activity. The ladder's coils and the timing diagrams' step names open a panel at that activity, with a button that brings you here with it highlighted                                                                                                                                                                                                  |

The ladder follows the engine's dependency rules (`docs/conditional-branching.md`, `docs/loops-guide.md`), so it shows what the workflow should do, and the timing diagram what it did:

- **A `depends_on` entry with no conditions** is a contact, closed once its step completes. Contacts in series all have to close, so a fan-in is an AND.
- **An entry with conditions** is a "finished" contact in series (completed, failed or skipped), and its conditions sit on a branch of their own. The engine runs a step when any entry applies, so those branches are in parallel (OR). Beside an unconditional entry they don't gate the step, and a shunt across them shows it.
- **A loop's back-edge** (the `depends_on` entry that closes the loop, found by walking `depends_on` from the steps with none, in file order) is a seal-in branch, in parallel with the way in, which carries a first-round contact. A loop step's rounds pass through a CTU counter preset to `iteration_limit`, or the engine's default of 100.
- **`settings.delay`** is an on-delay (TON) in series.
- **A signal wait** drives a TON timer preset to `timeout_seconds`, and the coil through the signal's interrupt contact; the timer's done contact drives what `on_timeout` says.

The pinout's inputs are the `INPUT` fields and secrets the workflow reads, and each signal it waits for, as an interrupt pin. Its outputs are the declared outputs of the steps nothing depends on. The ratings are the limits the workflow sets: `iteration_limit`, signal timeouts, activity timeouts, retry attempts, delays and budgets.

Every sheet carries a drafting title block: a drawing number that stays with the machine (`MS-` and five digits of a hash of its directory name), the revision (the tree label and commit of the results set shown), drawn by (the Stack, from the workflow), checked by (that results set) and its date. The sheets show the newest results set that ran the machine, or the one picked on the datasheet or timing sheet. A results set without timelines says "no timeline in this result set".

#### The results file

The results file (`machine-shop-results/2`) is the contract between runners and views:

- `runner`, `mode`, `engine_version`, `tree` (`label`, `branch`, `commit`), `host`, `started_at`, `seconds`
- `footprint`: the set's `peak_engine_connections` (sampled, or null) and `engine_pool_max_connections`
- `plug`: null, or the `seed`, `grace_seconds` and, in shop mode, `shop_fraction`
- `machine_footprints`: per machine, its `runs`, `measured` runs, `seconds` (all runs), `seconds_max`, `events`, `payload_bytes`, `queue_rows` and `peak_engine_connections`
- `runs`: one entry per run, with `machine`, `run`, `result` (`works`, `broken`, `known_broken` or `fixed`), `trap` (a trap's first issue, or null), `trap_issues` (all of them, or empty), `seconds`, the workflow's final `status` (`not_deployed` when the engine rejected the definition), `steps` (each step's `status` and `runs`), `problems`, `footprint` (see above; null when not deployed), `twin`, and optionally `timeline`
- `timeline`: the run's events from `workflow_events`, in order, each with `ms` (milliseconds after the workflow's `created_at`, from the event's stored `timestamp`), `event` (the engine's event type, such as `ActivityScheduled`), and when the event has them: `step`, `round` (the event's `iteration`), `attempt`, `reason` (`timeout` for a signal wait that timed out, or a failure's `error_code`), `on_timeout` and `will_retry`. A run whose workflow wasn't deployed has none
- `twin`: null, or its `outcome`, `plug` (`after_event` in bench mode, `events_at_plug`, `of_events`, `down_seconds`, `settled_before`, and `at_ms` and `back_ms`: when the plug was pulled and when the restarted API answered, on the twin's timeline; null if never pulled), `seconds`, `status`, `steps`, `differences`, `problems`, `footprint` and `timeline`, the twin's own events in the same form as the run's

Version 1 is version 2 without `host`, `footprint`, `plug`, `machine_footprints`, or a run's `footprint` and `twin`; a run may also lack `timeline`. The runner bundles both versions into the views, and the views render a version 1 set without those parts. Readers treat a file without `trap_issues` as having `[trap]`, and a run without `timeline` as having none, so the oldest results files still read.

Each run ends as one of four results:

| Result       | Meaning                                                  | Fails the test |
|--------------|----------------------------------------------------------|----------------|
| works        | It did what its card says                                | No             |
| broken       | It didn't, and it isn't a trap                           | Yes            |
| known broken | A trap that still shows its bug                          | No             |
| fixed        | A trap that now passes: take the trap label off its card | No             |

**A workflow the engine won't deploy** is a result too, not a crash. When `POST /api/v1/workflow_definitions` rejects a machine's `workflow.yaml`, each of its runs ends with the workflow `not_deployed`, every step `absent`, and the validator's message as the problem: known broken for a trap, broken otherwise. The other machines still run. A bug can show this way: the ferris wheel's valid loop fails validation on an engine that picks the back-edge by key order (#26).

## Two ways to use it

### One issue, one branch

When you're working on one fix, the shop tells you when the fix works and whether it holds up.

1. Find the issue's trap, or build one: a machine whose card says what should happen, with `trap:` naming the issue. On `main`, its runs come out known broken.
2. While you work, run only that machine, with `MACHINE_SHOP_ONLY`. When the fix works, its runs come out fixed. Take the trap label off the card in the same branch.
3. Before the pull request, run every machine in both modes.
   - Bench mode shows that nothing else broke.
   - Shop mode shows that the fix holds when everything runs at once, which is where timing bugs show. #28's mill passed in bench mode by luck.
   - If the fix changes what the engine writes, or when, pull the plug too.
4. To see the change, run `main` and your branch into the same `MACHINE_SHOP_OUT`, with `MACHINE_SHOP_TREE` set to each. The pages show both and mark the steps that changed.

### Many branches, one integration tree

When several fixes are merged into one tree to test together, the shop shows how they behave as a whole.

1. Run every machine in bench and shop mode, plain and with the plug pulled, labelled with the tree's name.
2. Every trap whose fix is merged should come out fixed, and every other run should come out as before. A run that breaks only in the merged tree points to where two fixes meet in the engine.
3. Run a combined pull request the same way. When its results match the integration tree's, the pull request carries every fix the tree does.

The shop doesn't replace unit or integration tests. It runs alongside them, and shows in one picture what the merged engine does with workflows a person can read.

## A machine

Each machine is a directory with two files:

- **`workflow.yaml`** is an ordinary workflow definition, with a unique `name`.
- **`card.yaml`** is its spec card:

```yaml
name: Railway switch
kind: simple                # simple or compound
picture: >
  A lever moves the points at a fork in the line ...
edge: >                     # the same machine in an edge setting
  A point-of-sale terminal sends a payment down one of two paths ...
functions:                  # the engine functions it exercises
  - a condition on a dependency, reading the dependency's output
trap:                       # only for a trap
  issue: 25                 # or issues: [29, 37], when it needs them all fixed
  runs: [lever to the left] # optional: the runs that show the bug; default every run
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
- **Give every card an `edge` reading:** one or two sentences that put the machine where the network, the power or the operator can't be trusted, such as a shop, a clinic, a vehicle or a gateway. It must describe what the machine actually exercises, and claim nothing the engine can't do. The parser treats it as optional; `machine_cards_are_valid` requires it of every card in `machines/`.
- **Write expectations from the docs and the intended behaviour,** never by running the machine and copying what it did.
- **Use real activities and their real output names.** For example, `echo` returns its parameters as the output `echo`.
- **Keep a machine correct whatever else is running.** Use its own inputs, and depend on nothing outside it.

### Signals

A step with `settings.wait_for_signal` waits until it is signaled or its wait times out. A run's `signals` are sent in order, each one like this:

1. The runner polls `GET /api/v1/workflows/{id}` until the step's status is `waiting`.
2. It sends `POST /api/v1/workflows/{id}/signal` with `activity_key`, `event_name` and, when the card gives one, `data`. It tries again while the step is still waiting and the API answers `signaled: false`.
3. Then it goes on to the next signal, or waits for the workflow to finish.

| Field   | What it is                                                                                                                                                             |
|---------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `step`  | The step to signal; it must exist and have `settings.wait_for_signal`                                                                                                  |
| `event` | The `event_name` to send                                                                                                                                               |
| `data`  | Optional. Left out, the request has no `data` field; `data: null` sends an explicit null                                                                               |
| `early` | Optional. `true` sends the signal at once, before the step waits; the API must answer `signaled: false`                                                                |
| `late`  | Optional. `true` sends the signal once the step has stopped waiting without one (its wait timed out), or the workflow has ended; the API must answer `signaled: false` |

A later signal to a step already signaled in the run is for the step's next round in a loop: the runner waits through the round's `pending`, `running` and `completed` until the step is `waiting` again.

A signal that can't be sent is one of the run's problems: the workflow finished first, the step got past waiting without one, or the run's timeout passed. So is an `early` or `late` signal the API accepts. The signals after it aren't sent. A run with no `signals` relies on the step's own timeout and `on_timeout`.

The runner's orchestrator checks for expired waits every second (the engine's default is 30 seconds), so a run that waits out a short signal timeout finishes in a few seconds. Give such a run a `timeout_seconds` above the wait's own timeout.

### State across attempts

A machine that needs state outside the workflow, such as a step that fails on its first attempts and succeeds on a retry, keeps it in the test's own database with the std worker's `postgres_query`. The runner gives the orchestrator one secret, `{{SECRET.shop_db_url}}`, the URL of the database `#[sqlx::test]` created for the run. Name anything a machine creates there after `{{WORKFLOW.id}}`, so runs of the same machine don't share it. The pull-start mower counts pulls in a sequence: `nextval` isn't rolled back when the query fails, so each retry sees the count go up.
