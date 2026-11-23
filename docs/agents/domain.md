# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Where to start

Read the [engineering standards](../engineering-standards.md) before adding features, fixing defects, changing a call flow or reviewing architecture. The [root glossary](../../CONTEXT.md) defines Kivio's domain terms. Before changing a domain, check the [architecture decisions](../adr/) that apply to it.

Some decisions supersede earlier ones. For example, [ADR-0007](../adr/0007-subagents-follow-omp.md) defines the OMP subagent design and supersedes ADR-0005.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

For a concept the glossary does not cover, check the terminology in the owning module and its ADRs.

## Flag ADR conflicts

If a proposed change conflicts with an accepted decision, identify the decision, explain the conflicting behavior and raise it before changing the implementation.
