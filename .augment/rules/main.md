---
type: "always_apply"
---

# Varda Project Rules

## The Flow: Intent → Vision → Spec → Source

This project follows a strict documentation-to-code pipeline. Each layer derives from and must remain consistent with the layer above it. Never skip layers.

```
/intent   (WHY)     → Motivations, beliefs, problems being solved, target users
    ↓
/vision   (WHAT)    → North star, success criteria, parity gaps, non-goals
    ↓
/spec     (HOW)     → Architecture, data models, routing, protocols, algorithms
    ↓
/src      (BUILD)   → Rust implementation that realizes the spec
```

### Rules of the Flow

1. **Downward derivation**: Every spec decision must trace back to a vision statement. Every line of code must trace back to a spec. If you can't point to the upstream justification, stop and discuss it with the human first.

2. **No spec-less code**: Do not write new subsystems, abstractions, or architectural changes in `/src` without a corresponding document in `/spec` that describes what is being built and why it relates back to /intent and /vision. Bug fixes are exempt.

3. **No vision-less specs**: Do not write specs for features that aren't grounded in `/vision` or `/intent`. If a feature seems necessary but isn't captured upstream, ask the human and update the docs first.

4. **Upstream changes cascade down**: If `/intent` or `/vision` changes in a way that contradicts existing `/spec` or `/src`, the downstream layers must be updated or flagged as stale and sent to the human for their input. Note conflicts explicitly — never leave silent contradictions.

5. **Specs are living documents**: Specs start rough (open questions, alternatives) and get refined through conversation. Mark status clearly: `NEEDS DESIGN`, `DRAFT`, `AGREED`, `IMPLEMENTED`. Code should only be written against `AGREED` or `IMPLEMENTED` specs.

6. **Unstructured is fine**: Documents in `/intent`, `/vision`, and `/spec` are unstructured markdown. They don't need to follow a template. Capture the thinking, not the format.

7. **Conversation drives refinement**: The user and AI refine specs through dialog. The AI should ask clarifying questions before writing specs, and should present design options with tradeoffs rather than making unilateral decisions.

## When planning work:
- **NEVER** esimate difficulty in terms of time it would take an unassisted human developer to implement. you are helping me implement so your time estimates will be wildly off (ie. you say a feature will take 2 weeks but we implement it together in 20 mins). We should esimatelye based on the complexity and use t shirt sizes (XS, S, M, L, XL) to indicate complexity.

## Working with Code
- **ALWAYS** Use domain driven design paradigms to produce clean modular code. we should segment domains and usecases a la Clean Architecture (Robert Martin / "Uncle Bob"). 
- **ALWAYS** use test driven development loops. 
- **ALWAYS** clean up old code paths. prefer replacing in place and cleaning up vs creating new files. 
- **ALWAYS** just replace old code with new code. We dont have users yet so we dont have to worry about backwards compatibility. 
- **ALWAYS** us an abstracted logger so logging is fully module with whatever data we need across the codebase using the same logging format and tooling. call this over native .log.

### Before Writing Code
- Review relevant `/spec` documents to understand what is being built
- Check `/vision/parity-gap.md` to understand priority and context
- Check `/spec/roadmap.md` to understand which phase this work belongs to and its dependencies
- Verify the feature/change is covered by spec; if not, write the spec first
- Check existing code patterns in `/src` before introducing new ones

### When Writing Code
- Follow the Rust skill guidelines in `.augment/skills/rust/SKILL.md`
- Match existing project conventions (module structure, naming, error handling)
- Keep `main.rs` thin — it is an orchestrator, not a logic container
- The UI layer (`/src/ui/`) reads state snapshots and emits actions; it never mutates engine state directly
- The engine layer (deck, stage, renderer) owns state and applies mutations
- **Remove dead code**: When refactoring, delete old code paths that no longer serve the target architecture. No dead code, no backwards compatibility shims — there are no existing users.
- **Refactor in place**: The existing prototype is the base. Modify and update existing code rather than rewriting from scratch. See the migration table in `/spec/architecture-overview.md`.

### After Writing Code
- Update the relevant `/spec` doc to mark features as `IMPLEMENTED`
- Update `/spec/roadmap.md` to reflect completed tasks (mark rows, update phase status)
- Flag any spec deviations discovered during implementation and notify the human
- **Alignment review**: After every change, review `/intent`, `/vision`, `/spec`, and `/src` to ensure they are all aligned. If a change in one layer creates a contradiction in another, flag it explicitly and notify the human

## Document Hygiene

- **One concept per file** in `/spec` — don't merge unrelated specs into one doc
- **Cross-reference** between docs using relative paths when one spec depends on another
- **Open questions** should be captured inline with `### Open Questions` sections — never silently defer a decision
- **Decisions** should be captured with rationale — not just "we chose X" but "we chose X because Y, and rejected Z because W"

## AI Behavior

- When the user asks for a feature: check spec first, ask clarifying questions, then propose approach
- When the user describes a problem: trace it through the flow (is it an intent gap? vision gap? spec gap? implementation bug?)
- When in doubt about a design decision: present options with tradeoffs, don't guess
- Never fabricate spec compliance — if code doesn't match spec, say so
- Treat `/intent` and `/vision` as the user's voice — don't contradict them without explicit discussion

