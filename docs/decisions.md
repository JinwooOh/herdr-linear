# Decisions

Small decisions that do not need an ADR.

- 2026-11-27: The issue-list row hides the assignee column when showing it would leave the title fewer than 16 cells; the detail pane always shows the assignee. (HL-29, decided with Jinwoo)
- 2026-11-27: The issue list and detail panes stack whenever a side-by-side layout would leave All-accounts queue titles fewer than 16 cells after the longest configured Account prefix, identifier and Issue state; the rule holds on every Account tab so pressing `a` never moves the panes. With `work` and `personal` the panes sit side by side from 110 columns. Rejected: shrinking the prefix and state column, a fixed cutoff of 100, and a wider list share. (HL-34, decided with Jinwoo)
