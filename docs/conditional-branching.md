# Conditional Branching

KruxiaFlow runs an activity only when its dependencies are *applicable*.
By default, "applicable" means "completed." Adding `conditions:` to a
dependency edge gates that edge on a runtime expression — and
understanding how multiple dependencies interact with conditions is the
key to writing branching workflows correctly.

This guide is a sibling to [Workflow Loops](loops-guide.md), which
covers the looping case (back-edges with conditions). Here we cover
forward-edge conditional execution: "run activity X only when condition
Y is true."

## The model

Every dependency edge has two states for the dependent activity:

- **Applicable**: the prerequisite has completed AND every condition on
  that edge evaluates to `true`.
- **Not applicable**: the prerequisite has completed AND at least one
  condition evaluates to `false`. (Or the prerequisite hasn't completed
  yet — but that's just "not ready" rather than "skipped.")

Decision rule:

> An activity is scheduled when **at least one** of its dependency
> edges is applicable. Edges with failed conditions are simply
> ignored — they do not block scheduling.

In other words, multiple `depends_on` edges form an **OR** at the
edge level. Multiple conditions on a single edge form an **AND**
within that edge.

## Worked example: A → B (conditional) → C

```yaml
activities:
  - key: classify_email
    activity_name: llm_prompt
    # ... outputs result with should_archive: true|false ...

  - key: archive_message
    activity_name: graph_archive
    depends_on:
      - activity_key: classify_email
        conditions:
          - "{{classify_email.result.should_archive == true}}"
```

`archive_message` schedules only when `classify_email` completes AND
its output has `should_archive == true`. If `should_archive` is false,
the edge is not applicable, and since it's `archive_message`'s only
edge, the activity is skipped entirely.

## The non-obvious part: mixing conditional and unconditional dependencies

Here's the pattern that bites people:

```yaml
- key: archive_message
  activity_name: graph_archive
  depends_on:
    - log_action                          # unconditional
    - activity_key: classify_email
      conditions:
        - "{{classify_email.result.should_archive == true}}"
```

This **does not gate** `archive_message` on the condition. The
`log_action` edge is unconditional — it's always applicable once
`log_action` completes. Per the OR-of-edges rule, that single
applicable edge schedules the activity, regardless of what the
conditional edge says. The condition is effectively ignored.

The fix is to put the condition on the edge that actually gates the
activity, and rely on transitive ordering for the other dependency:

```yaml
- key: archive_message
  activity_name: graph_archive
  depends_on:
    - activity_key: log_action
      conditions:
        - "{{classify_email.result.should_archive == true}}"
```

`log_action` already (transitively) depends on `classify_email`, so
its outputs are available in the condition. With only one edge —
conditional — the activity is properly gated.

If you genuinely need *all* of multiple distinct dependencies to be
satisfied AND the same condition to hold, repeat the condition on each
edge:

```yaml
depends_on:
  - activity_key: dep_a
    conditions: ["{{ classify.result.should_archive == true }}"]
  - activity_key: dep_b
    conditions: ["{{ classify.result.should_archive == true }}"]
```

## Branching: archive vs. draft (mutually exclusive)

Forward-edge conditions are how you implement classic if/else fanout:

```yaml
- key: archive_message
  depends_on:
    - activity_key: classify_email
      conditions:
        - "{{classify_email.result.should_archive == true}}"

- key: draft_reply
  depends_on:
    - activity_key: classify_email
      conditions:
        - "{{classify_email.result.should_archive == false}}"
        - "{{classify_email.result.should_draft_reply == true}}"
```

Both activities depend on `classify_email`. The first runs when
`should_archive` is true; the second runs when both `should_archive`
is false AND `should_draft_reply` is true. Multiple conditions on a
single edge are AND'd.

## Common pitfalls

| Pattern | What you might think | What actually happens |
| --- | --- | --- |
| Conditional edge + unconditional sibling edge | Activity gated on condition | Always runs (unconditional sibling is applicable) |
| `condition:` (singular) at activity level | Activity-level gate | Silently ignored — no such field. Use `conditions:` under `depends_on:` |
| `conditions: ["{{x == true}}", "{{y == true}}"]` on one edge | Either condition gates | Both must be true (AND within edge) |
| Multiple `depends_on` edges, each with conditions | Both conditions must hold | Either condition is enough (OR across edges) |

## See also

- [Workflow Loops](loops-guide.md) — back-edges with conditions
  (looping use case)
- [API Reference](api-reference.md#workflow-definition) — full
  workflow YAML schema
