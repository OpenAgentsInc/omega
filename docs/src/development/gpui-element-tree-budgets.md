---
title: GPUI element-tree budgets
description: Deterministic element-count budgets and source attribution for dense GPUI UI.
---

# GPUI element-tree budgets

Phone-pairing once froze the UI because a QR code was built as roughly
8,000–10,000 individual elements and rebuilt on every transcript update.
Element-tree budgets catch that class of regression with **work counts**,
not wall-clock time.

## What is counted

In `debug_assertions`, `test`, and `test-support` builds, each element that
crosses GPUI's drawable layout boundary increments a per-frame counter and is
bucketed by the construction site from `#[track_caller]` /
`core::panic::Location` (`file`, `line`, `column`).

Release builds keep this path disabled or no-op so shipping overhead stays
negligible.

## Budget warnings

Default threshold: **10,000** elements per frame.

When a frame first exceeds the budget, GPUI logs the total count and the five
hottest construction sites. Override with:

```bash
export GPUI_ELEMENT_TREE_BUDGET=2500
```

Use a lower budget in focused debugging; use the default (or higher) in
normal development if you are not hunting dense trees.

## Test APIs

After rendering into a test window:

```rust
let snapshot = window
    .update(cx, |_, window, _| window.debug_render_snapshot())
    .expect("window open");

assert!(
    snapshot.element_count() <= budget,
    "hotspots: {:?}",
    snapshot.element_hotspots()
);
```

- `element_count()` — nodes in that frame
- `element_hotspots()` — hottest construction sites, highest count first

## Recommended shapes

| Dense UI | Do this |
| --- | --- |
| QR codes / bitmaps / heatmaps | One `RenderImage` (or SVG / canvas), not one element per pixel/module |
| Charts | Path geometry or a pre-rasterized image |
| Long lists (transcript, outline, files) | `uniform_list` / virtualized `List` |
| Icon grids | Shared sprite/image atlas |

### Pairing QR example

The pairing card rasterizes the QR modules into a single `RenderImage`. A
realistic 57×57 module payload stays under a **40-element** test budget; the
previous per-module element tree exceeded **3,000** elements.

Coverage:

- `gpui`: `debug_render_snapshot_counts_elements_and_attributes_hotspots`
- `agent_ui` (test-support): `test_pairing_content_stays_within_element_budget`

## Follow-ups

This slice is **element construction only**. Later stress scenes can reuse
the same attribution pattern for paint ops, resource creation, and
task/subscription churn.
