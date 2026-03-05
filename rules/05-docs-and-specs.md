# Specs Vs Rules Vs Docs

Use the right mechanism for the right kind of truth.

## Specs (`specs/`)

Specs are durable product and technical requirements.

Use specs when:

1. You are defining behavior, semantics, or public API constraints.
2. You want to prevent silent drift across refactors.
3. You need to communicate "what must be true" to future contributors and agents.

Rules:

1. Each spec is a separate domain topic file.
2. Update `SPECS.md` when adding a new spec.

## Rules (`rules/`)

Rules are operational constraints and repeated steering lessons.

Use rules when:

1. A failure mode keeps recurring.
2. You want deterministic "how to work here" guidance for agents.
3. You need verification discipline and context hygiene.

Rules should be small and composable. Avoid giant omnibus documents.
## Docs (`docs/`)

Docs are explanations, rationale, and teaching materials. They are not requirements by default.

Use docs when:

1. You are explaining why a decision exists.
2. You are providing tutorials or walkthroughs.
3. You are recording research or analysis.

If a doc contains a requirement, it should be promoted into `specs/`.
