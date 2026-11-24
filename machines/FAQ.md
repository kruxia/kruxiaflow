# Machine shop FAQ

Questions that come up about the machine shop, with short answers. For how it works in full, see [README.md](README.md).

- [Is the machine shop part of the engine?](#is-the-machine-shop-part-of-the-engine)
- [How do I run it?](#how-do-i-run-it)
- [Why is it a workspace crate, not in `contrib/`?](#why-is-it-a-workspace-crate-not-in-contrib)
- [Is it cross-platform?](#is-it-cross-platform)
- [Why does the Stack look like a classic Mac?](#why-does-the-stack-look-like-a-classic-mac)
- [Does it use Python?](#does-it-use-python)
- [What does it show on 0.12.6?](#what-does-it-show-on-0126)

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
