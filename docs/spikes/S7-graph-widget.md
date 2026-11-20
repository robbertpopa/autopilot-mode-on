<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# S7: egui-snarl or our own graph?

**Question:** can `egui-snarl` (0.12, updated August 2026) host our step nodes (settings forms inside nodes, a checkbox, status pills, arrows with labels, a selected state, a loop node) or is a small custom canvas simpler? **Gates:** M5. **Spending:** none.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S7-graph-widget.md`."

`egui_node_graph2` was last updated in 2024: not considered. Reference design: the Claude Design canvas linked from `docs/ui.md`.

## Prototype (one day, throwaway crate under `spikes/`, not part of the workspace gate)
- [x] Compare the six-step pipeline, edge labels, the bent transition to Plan, and the Build–Check loop.
- [x] Render the four settings controls, checkbox, status, footer and warning chip. Exercise selection and the checkbox.
- [x] Render idle, running, asking, skipped, passed and failed states, with a blue selected outline.
- [x] Exercise step selection, the detail panel and keyboard navigation. Check whether dragging can move the fixed nodes.
- [x] Check idle repaint scheduling and wake the graph with a synthetic event.
- [x] Compare the graph-specific code with a custom canvas using egui rectangles and routed arrows.

## Decide
Choose snarl if node widgets and edge labels fit without hacks; otherwise the custom canvas (the graph is small and mostly fixed). Write the decision and the reason in `docs/decisions.md`.

## Result
Use a fixed egui canvas for M5. On 2026-11-21, a headless input and layout probe ran on Windows and Linux with egui `0.36.0` and egui-snarl `0.12.0`. It followed the three-by-two layout in the local `Main.dc.html` export. Both implementations rendered six cards and 24 settings combos and passed click selection, Right Arrow navigation and checkbox input. After settling, each reported no scheduled repaint. A synthetic event requested a new frame.

| Measured at 1260 × 780 logical pixels | Fixed canvas | egui-snarl |
|---|---|---|
| Visible label widgets | All six, with no label intersecting a card | Zero of six |
| Card movement from the probe drag | 0 × 0 px | 44 × 14 px |
| Form layout | Shared form worked inside card rectangles | Shared form needed an explicit vertical layout inside the node body |
| Graph-specific Rust lines after formatting | 108 | 201 |

The fixed canvas routed the elbow and self-loop directly from card geometry. In egui-snarl `0.12.0`, the viewer declares wire widget hooks and the tested wire drawing loop omits calls to them. Node-frame dragging is wired directly to primary pointer movement. Matching the reference with this release would require separate label drawing and a way to restore fixed node positions. [Wire widget hooks](https://github.com/zakarumych/egui-snarl/blob/23e7489b51b56e9f51e756bae8efdabc49f12fec/src/ui/viewer.rs#L215-L230) [Wire drawing](https://github.com/zakarumych/egui-snarl/blob/23e7489b51b56e9f51e756bae8efdabc49f12fec/src/ui.rs#L1114-L1193) [Node drag](https://github.com/zakarumych/egui-snarl/blob/23e7489b51b56e9f51e756bae8efdabc49f12fec/src/ui.rs#L1865-L1884)

The probe measured layout and input responses. Native window rendering, accessibility and high-DPI checks belong to M5. Recheck the egui API against the version selected for M5.
