# S7: egui-snarl or our own graph?

**Question:** can `egui-snarl` (0.12, updated August 2026) host our step nodes (settings forms inside nodes, a checkbox, status pills, arrows with labels, a selected state, a loop node) or is a small custom canvas simpler? **Gates:** M5. **Spending:** none.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S7-graph-widget.md`."

`egui_node_graph2` was last updated in 2024: not considered. Reference design: the Claude Design canvas linked from `docs/ui.md`.

## Prototype (one day, throwaway crate under `spikes/`, not part of the workspace gate)
- [ ] The 6-node pipeline: Understand, Clarify, Design, Plan, Build ⟲ Check, Pull request, with arrows, a bent arrow back to the second row, and edge labels.
- [ ] Inside each node: combo boxes (auth, model, effort, limit), the "Ask me after this step" checkbox, a status pill, a footer line, a warning chip. Can snarl draw all that without fighting its layout?
- [ ] States: idle, running, asking (amber), skipped (dashed), passed, failed; a selected node with a blue outline.
- [ ] Interaction: click to select a step and open its detail; keyboard navigation; zoom and pan only if cheap; nodes are fixed, so dragging nodes is not needed.
- [ ] Repaint: confirm it repaints only on events or input.
- [ ] A custom version (rectangles, `painter` lines, arrows computed from node edges) for comparison: how many lines and how much effort?

## Decide
Choose snarl if node widgets and edge labels fit without hacks; otherwise the custom canvas (the graph is small and mostly fixed). Write the decision and the reason in `docs/decisions.md`.

## Result
(fill in)
