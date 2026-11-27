# Decisions

Small decisions that do not need an ADR.

- 2026-11-27: The issue-list row hides the assignee column when showing it would leave the title fewer than 16 cells; the detail pane always shows the assignee. (HL-29, decided with Jinwoo)
- 2026-11-27: The issue list and detail panes stack whenever a side-by-side layout would leave All-accounts queue titles fewer than 16 cells after the longest configured Account prefix, identifier and Issue state. With `work` and `personal` the panes sit side by side from 110 columns. Rejected: shrinking the prefix and state column, a fixed cutoff of 100, and a wider list share. (HL-34, decided with Jinwoo)
- 2026-11-27: The pane-stacking cutoff applies on every issue-list view, Account tabs included, so pressing `a` never moves the panes. (HL-34, decided with Jinwoo)
- 2026-11-27: The tab bar starts with the Issue scope label (`Assigned` or `Team`); on narrow panes the All-accounts queue tab may be cut off at the right edge. (HL-31, decided with Jinwoo)
- 2026-11-27: A hidden-scope fetch running when a state change starts or finishes is dropped, and that scope refreshes as soon as it is shown again. (HL-32, decided with Jinwoo)
- 2026-11-27: On an Account tab the footer shows only that Account's errors; the All-accounts queue shows every Account's errors with their names. (HL-32, decided with Jinwoo)
