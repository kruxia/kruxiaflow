# Machine shop FAQ

Questions that come up about the machine shop, with short answers. For how it works in full, see [README.md](README.md).

- [Is the machine shop part of the engine?](#is-the-machine-shop-part-of-the-engine)
- [How do I run it?](#how-do-i-run-it)
- [Can I run a machine from the pages, or on its own?](#can-i-run-a-machine-from-the-pages-or-on-its-own)
- [How do I add a machine, or a compound one?](#how-do-i-add-a-machine-or-a-compound-one)
- [Why is it a workspace crate, not in `contrib/`?](#why-is-it-a-workspace-crate-not-in-contrib)
- [Is it cross-platform?](#is-it-cross-platform)
- [Why does the Stack look like a classic Mac?](#why-does-the-stack-look-like-a-classic-mac)
- [Does it use Python?](#does-it-use-python)
- [What does it show on 0.12.6?](#what-does-it-show-on-0126)
- [What's next?](#whats-next)
- [How can I help?](#how-can-i-help)

## Is the machine shop part of the engine?

No. It's a developer tool in its own crate, and none of it ships in the `kruxiaflow` binary or the Docker image.

- **Its own crate:** `kruxiaflow-machine-shop` (`machines/Cargo.toml`), a member of the workspace that's never published. Its library is empty.
- **The runner is an integration test,** `machines/tests/machine_shop.rs`. It starts an engine inside the test process, using only the workspace crates' public APIs. A release build doesn't build tests, so none of it reaches the binary.
- **Docker:** the released image holds only the `kruxiaflow` binary. `.dockerignore` leaves the shop out of the build context, except its manifest and empty library, which the workspace needs to resolve.

## How do I run it?

From a clone of the repository, with Rust and a PostgreSQL database:

```bash
DATABASE_URL=postgres://... cargo test -p kruxiaflow-machine-shop --test machine_shop -- --nocapture
```

| Test                                | Mode  | What it does                                                   | Time, roughly  |
|-------------------------------------|-------|----------------------------------------------------------------|----------------|
| `machine_shop_bench`                | bench | One machine at a time, so nothing else is running              | 1 minute       |
| `machine_shop_shop`                 | shop  | Every run of every machine at once, in one engine              | 15 seconds     |
| either, with `MACHINE_SHOP_PLUG=on` | plug  | Every run gets a twin that has the plug pulled partway through | 9 minutes      |

`MACHINE_SHOP_ONLY=domino-run,level-crossing` runs only those machines. Each run writes a results file and the two views, the Shop Floor and the Stack, under `target/tmp/machine-shop/` or `MACHINE_SHOP_OUT`. The views open from disk and load nothing from the network.

## Can I run a machine from the pages, or on its own?

**Not from the pages.** The Shop Floor and the Stack show results that are already written; they start nothing and talk to no server.

**One machine, with its card checked:** run the shop with `MACHINE_SHOP_ONLY` set to the machine's directory name.

```bash
MACHINE_SHOP_ONLY=railway-switch DATABASE_URL=postgres://... \
  cargo test -p kruxiaflow-machine-shop --test machine_shop machine_shop_bench -- --exact --nocapture
```

**On any running engine, without the card:** a machine's `workflow.yaml` is an ordinary workflow, so you can deploy it and submit a run, the same way as the main README's quickstart. Take the input from one of the card's runs:

```bash
curl -s -X POST http://localhost:8080/api/v1/workflow_definitions \
  -H "Content-Type: text/yaml" --data-binary @machines/railway-switch/workflow.yaml

curl -s -X POST http://localhost:8080/api/v1/workflows \
  -H "Content-Type: application/json" \
  -d '{"definition_name": "machine_railway_switch", "input": {"track": "left"}}'
```

If your server requires a login, add `-H "Authorization: Bearer $TOKEN"` to both. Done that way, nothing checks the result against the card, and you send any signals yourself. Three machines (`pull-start-mower`, `printing-press` and `vending-machine`) also keep state in a database through the secret `shop_db_url`, which only the runner provides.

## How do I add a machine, or a compound one?

A machine is a directory in `machines/` with two files. A compound machine is the same, with a workflow that combines about three engine functions instead of one.

1. **Pick a physical machine** whose behaviour is plain from the picture, so a bug looks like a jam: a step that never runs, or runs twice.
2. **Write `workflow.yaml`,** an ordinary workflow with a unique `name`, using real activities such as `echo`.
3. **Write `card.yaml`:**
   - `kind`, `simple` or `compound`;
   - a `picture`, and an `edge` reading that puts the machine in an edge setting;
   - the engine `functions` it exercises;
   - one or more `runs`, each with its input and what should happen: the workflow's status, and every step's status and run count.
4. **Write the expectations from the docs and the intended behaviour,** never by running the machine and copying what it did.
5. **Run it alone** with `MACHINE_SHOP_ONLY`, then every machine in both modes.

The full card format, signals, and state kept across attempts are in [README.md](README.md#a-machine).

**To turn a bug into a trap,** add `trap:` with the issue number to the card of a machine that shows it. Its runs come out "known broken" until the fix lands, then "fixed". Take the label off in the branch that fixes it.

## Why is it a workspace crate, not in `contrib/`?

It's kept visibly separate from the engine, as its own crate, but not in `contrib/`, for two reasons:

1. **`contrib/` holds add-ons that users run beside the engine,** such as a sandboxed worker. The shop is something developers run against the engine.
2. **A crate outside the workspace builds the engine a second time.** It has its own lock file and its own `target/`. As a workspace member, the shop shares the one build.

Where it lives and its name are still open.

## Is it cross-platform?

Yes.

- **No OS-specific code.** The runner needs Rust, PostgreSQL and `git` (for the branch name). Pulling the plug aborts the engine's tasks inside the test process; it sends no operating-system signal.
- **Linux:** CI's test job on Ubuntu runs bench and shop mode with the rest of the workspace.
- **Not yet tried:** plug mode has run on macOS only, and Windows hasn't been tried. Results record where they ran (`MACHINE_SHOP_HOST`, or `<os>/<arch>`), so runs from different hosts can sit side by side in the same views.

## Why does the Stack look like a classic Mac?

It's a style: a homage to HyperCard, and any browser renders it.

- **The Stack** draws one card per machine in 1-bit ink on paper, on a dithered desk. It borrows the look, not Apple's artwork, names or fonts. It asks for Geneva and Monaco, and falls back to Verdana, DejaVu Sans and the system monospace font. It has a light and a dark theme.
- **The Shop Floor** has a look of its own and uses the system's fonts.
- Both are plain HTML files with everything embedded, so they open the same way on any OS.

## Does it use Python?

No. Every step of every machine runs on the Rust std worker: 106 steps are `echo` and 7 are `postgres_query`. There's no Python file or Python project in `machines/`. A machine that ran steps on the Python worker would bring Python in; none does today.

## What does it show on 0.12.6?

Every run does what its card says. With the plug pulled, the open crash fixes close the last gaps. These are the pages in `views/rendered/`, from 2026-11-24:

| Tree                                 | Works | Fixed | Twins identical, bench | Twins identical, shop |
|--------------------------------------|-------|-------|------------------------|-----------------------|
| `main` at v0.12.6                    | 14    | 33    | 44 of 47               | 47 of 47              |
| `mecha` (`main` with the open fixes) | 14    | 33    | 47 of 47               | 47 of 47              |

- **Fixed** means a trap, a machine that reproduces a filed bug, now does what its card says.
- **Two twins hung on `main`:** `level-crossing` ("no train coming") and `prism-and-funnel` ("white light"). Their steps stayed pending after the restart. #59 and #60 fix that kind of loss, and neither hung with them merged.
- **The third** (`vending-machine`) had its plug pulled after the workflow had already finished, so its twin tested nothing. It's not a failure.

## What's next?

The shop is an R&D exploration, and these questions are open in its pull request:

1. **Where it lives:** `machines/` at the repo root, `examples/`, or a repo of its own.
2. **Its name:** "machine shop" was the first one; "makerspace" may fit the Rube Goldberg inspiration better.
3. **CI:** bench and shop mode run with every push today, in about 70 seconds. Keep that, or move them to nightly or on demand? Plug mode is too slow for every pull request.
4. **The rendered pages:** refreshed by hand today. CI could build them as a run artifact instead.
5. **Trap labels** for fixes now on `main`: take them off, or keep them while the fixes are new.

Beyond those, two things haven't been tried yet: plug mode on Linux, and anything on Windows or on small edge hardware, such as a Raspberry Pi.

## How can I help?

- **Run it on another host,** and share the results file. Set `MACHINE_SHOP_HOST` to a label for the hardware, such as `pi-5`; the label is all the results record about the machine. Runs from several hosts appear side by side in the same views.
- **Add a machine,** especially a compound one that combines functions no machine combines yet. Interactions are where the most interesting behaviour shows up.
- **Turn a bug into a trap** when you file an issue, so its fix shows up as "fixed".
- **Read the views and say what's unclear:** a term the glossary doesn't explain, or a picture that doesn't match what the machine does.
- **Weigh in on the open questions above** in the pull request.
