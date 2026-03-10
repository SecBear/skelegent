# Architecture

This section describes the structural design of skelegent in detail:

- **[The 6-Layer Model](layers.md)** -- What each layer does, which crates belong to it, and the dependency rules that keep the system composable.
- **[Protocol Traits](protocol-traits.md)** -- The four protocol traits and two cross-cutting interfaces that form the stability contract.
- **[Design Decisions](design-decisions.md)** -- Key architectural choices and the reasoning behind them.
- **[Dependency Graph](dependency-graph.md)** -- How crates depend on each other, with an ASCII diagram.

For the full design rationale, see `ARCHITECTURE.md` in the repository root and the detailed specifications in `specs/`.
