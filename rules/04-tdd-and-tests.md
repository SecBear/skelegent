# TDD And Tests

Skelegent is built to be composable, and composability requires backpressure.

## Default Workflow

1. Write the smallest failing test that demonstrates the requirement.
2. Implement the minimum to pass.
3. Refactor only after green.

## Test Strategy

Prefer "swappable mock components":

1. Mock providers for deterministic model outputs.
2. In-memory state stores for repeatable runs.
3. Local orchestrators/environments for fast feedback.

Keep a "real path" available via feature flags or ignored tests, but do not require network access
for the default test suite.

## No Unverifiable Demos

If a demo cannot be tested, it does not count as a proof. Convert demos into:

1. integration tests in `tests/`
2. crate integration tests in `crate/tests/`

