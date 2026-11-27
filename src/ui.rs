use std::time::Duration;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{App, IssueScope, View},
    linear::Issue,
    review::{FileDiff, Review},
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let messages = messages(app);
    match &mut app.view {
        View::Review(review) => {
            draw_review(frame, review, &messages);
            draw_merge_picker(frame, review);
            return;
        }
        View::PrPicker(picker) => {
            let rows = picker
                .options
                .iter()
                .map(|pr| {
                    ListItem::new(vec![
                        Line::from(pr.title.clone().unwrap_or_else(|| pr.url.clone())),
                        Line::from(Span::styled(pr.url.clone(), Style::new().fg(Color::Cyan))),
                    ])
                })
                .collect::<Vec<_>>();
            let [body, footer] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
            frame.render_stateful_widget(
                List::new(rows)
                    .block(Block::bordered().title(format!(" {} — choose PR ", picker.title)))
                    .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
                body,
                &mut picker.list,
            );
            frame.render_widget(
                Paragraph::new("j/k move  enter review PR  esc/q cancel"),
                footer,
            );
            return;
        }
        View::StatePicker(picker) => {
            let heading = format!(
                " {} / {} — change state ",
                app.accounts[picker.account].name, picker.title
            );
            let [body, footer] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
            if picker.options.is_empty() {
                let message = picker
                    .error
                    .as_deref()
                    .unwrap_or("Loading team states… (no selectable states yet)");
                frame.render_widget(Block::bordered().title(heading.clone()), body);
                if picker.error.is_some() {
                    frame.render_widget(
                        Paragraph::new(message).block(Block::bordered().title(heading.clone())),
                        body,
                    );
                } else {
                    draw_loading(
                        frame,
                        body,
                        "Loading team states",
                        &picker.title,
                        picker.started.elapsed(),
                        "esc/q cancel",
                    );
                }
            } else {
                let rows = picker
                    .options
                    .iter()
                    .map(|state| {
                        ListItem::new(format!(
                            "{}{}",
                            state.name,
                            if state.id == picker.current_state {
                                " (current)"
                            } else {
                                ""
                            }
                        ))
                    })
                    .collect::<Vec<_>>();
                frame.render_stateful_widget(
                    List::new(rows)
                        .block(Block::bordered().title(heading))
                        .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
                    body,
                    &mut picker.list,
                );
            }
            frame.render_widget(
                Paragraph::new(if picker.confirming {
                    "Confirm terminal state? y apply; any other key cancels"
                } else {
                    "j/k move  enter select  esc/q cancel"
                }),
                footer,
            );
            return;
        }
        View::Issues => {}
    }
    let [body, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(5)]).areas(frame.area());
    // Side by side only while the list keeps room for All-accounts queue titles. The rule holds
    // on every Account tab too, so pressing `a` never moves the panes. The shortest possible
    // minimum (`[] ` + 37) already stacks anything under 80 columns.
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(body);
    let [left, right] = if usize::from(left.width.saturating_sub(2)) < min_queue_row_width(app) {
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body)
    } else {
        [left, right]
    };
    let title = account_tabs(app);
    let all = app.all_accounts_queue();
    // Inside the list's two borders; the list sets no highlight symbol, so rows get the rest.
    let row_width = usize::from(left.width.saturating_sub(2));
    let items: Vec<ListItem> = if all {
        app.queue
            .iter()
            .map(|&(a, i)| {
                let issue = &app.accounts[a].list.issues[i];
                let prefix = Span::styled(
                    account_prefix(&app.accounts[a].name),
                    Style::new().fg(Color::Magenta),
                );
                ListItem::new(list_row(issue, Some(prefix), row_width))
            })
            .collect()
    } else {
        app.accounts[app.active]
            .list
            .issues
            .iter()
            .map(|i| ListItem::new(list_row(i, None, row_width)))
            .collect()
    };
    let list = List::new(items)
        .block(Block::bordered().title(title))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    if all {
        frame.render_stateful_widget(list, left, &mut app.queue_list);
    } else {
        frame.render_stateful_widget(list, left, &mut app.accounts[app.active].list.selection);
    }
    let detail = match app.selected() {
        Some((account, issue)) => {
            let mut lines = vec![Line::from(format!(
                "Account: {}",
                app.accounts[account].name
            ))];
            lines.extend(detail_lines(issue));
            lines
        }
        None => vec![Line::from(match app.scope {
            IssueScope::Assigned => "No open issues assigned to you (or still loading).",
            IssueScope::Team => "No open Team issues (or still loading).",
        })],
    };
    frame.render_widget(
        Paragraph::new(detail)
            .block(Block::bordered().title(" Detail "))
            .wrap(Wrap { trim: false }),
        right,
    );
    let activity = if app.accounts.iter().any(|a| a.list.loading()) {
        "refreshing…"
    } else {
        ""
    };
    let hint = format!("{activity}\n{messages}\nj/k move  g/G ends  tab/1-9 Account  a all  t scope\nenter PR  s state  r refresh  q/Esc quit  Ctrl+C quit");
    frame.render_widget(
        Paragraph::new(crate::display::safe(&hint)).style(Style::new().fg(Color::DarkGray)),
        footer,
    );
    if let (Some(title), Some(started)) = (app.loading_review(), app.loading_since()) {
        let title = title.to_string();
        draw_loading(
            frame,
            body,
            "Loading pull request",
            &title,
            started.elapsed(),
            "esc cancel  q quit",
        );
    }
}

/// Cells advance whole steps at a time so the bar reads as chunky "pixels".
const STEP: Duration = Duration::from_millis(90);
/// Show reassurance once a load is noticeably slow.
const SLOW: Duration = Duration::from_secs(4);

/// An indeterminate bar: a block of filled cells bouncing along an empty track.
pub fn pixel_bar(width: usize, elapsed: Duration) -> String {
    if width == 0 {
        return String::new();
    }
    let block = (width / 4).clamp(1, width);
    let travel = width - block;
    let step = (elapsed.as_millis() / STEP.as_millis()) as usize;
    let start = if travel == 0 {
        0
    } else {
        let cycle = step % (2 * travel);
        if cycle <= travel {
            cycle
        } else {
            2 * travel - cycle
        }
    };
    (0..width)
        .map(|i| {
            if (start..start + block).contains(&i) {
                '█'
            } else {
                '░'
            }
        })
        .collect()
}

/// A centered box over `area` with a title, animated bar, elapsed time, and keys.
fn draw_loading(
    frame: &mut Frame,
    area: Rect,
    heading: &str,
    subject: &str,
    elapsed: Duration,
    keys: &str,
) {
    let width = area.width.min(54);
    let height = area.height.min(8);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let inner = width.saturating_sub(4) as usize;
    let accent = Style::new().fg(Color::Cyan);
    let mut lines = vec![
        Line::from(Span::styled(
            crate::display::safe(subject),
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(pixel_bar(inner, elapsed), accent)),
        Line::from(format!("{}s  ·  {keys}", elapsed.as_secs())),
    ];
    if elapsed >= SLOW {
        lines.push(Line::from(Span::styled(
            "Still working — large PRs can take a while.",
            Style::new().fg(Color::DarkGray),
        )));
    }
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(accent)
                .title(Span::styled(
                    format!(" {heading} "),
                    accent.add_modifier(Modifier::BOLD),
                ))
                .padding(ratatui::widgets::Padding::horizontal(1)),
        ),
        popup,
    );
}

fn messages(app: &App) -> String {
    // Dedicated lines: transient success must never replace account failures.
    // An Account tab shows only its own errors; the All-accounts queue shows every Account's.
    let all = app.all_accounts_queue();
    let errors = app
        .accounts
        .iter()
        .enumerate()
        .filter(|(index, _)| all || *index == app.active)
        .filter_map(|(_, a)| {
            a.list
                .error
                .as_ref()
                .map(|e| format!("{}: {}", a.name, e.lines().next().unwrap_or(e)))
        })
        .collect::<Vec<_>>()
        .join(" | ");
    let status = app
        .status
        .as_deref()
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");
    crate::display::safe(&format!("{errors}\n{status}"))
}

fn account_tabs(app: &App) -> Line<'static> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            format!("{} ", app.scope.label()),
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
    ];
    for (index, account) in app.accounts.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" | "));
        }
        let marker = if account.list.error.is_some() {
            " !"
        } else if account.list.loading() {
            " …"
        } else {
            ""
        };
        let label = format!("{} ({}){}", account.name, account.list.issues.len(), marker);
        let style = if index == app.active {
            Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::new()
        };
        spans.push(Span::styled(label, style));
    }
    let style = if app.all_accounts_queue() {
        Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::new()
    };
    spans.push(Span::raw(" | "));
    spans.push(Span::styled(
        format!("All-accounts queue ({}) ", app.queue.len()),
        style,
    ));
    Line::from(spans)
}

/// The fewest title cells a row keeps; below this the assignee column is left out.
const MIN_TITLE_WIDTH: usize = 16;
/// Cells the identifier and Issue state columns are padded to.
const IDENTIFIER_WIDTH: usize = 9;
const STATE_WIDTH: usize = 12;

/// The `[account] ` prefix that All-accounts queue rows start with.
fn account_prefix(name: &str) -> String {
    format!("[{name}] ")
}

/// Row cells an All-accounts queue row needs to keep `MIN_TITLE_WIDTH` title cells
/// after the longest configured Account prefix, the identifier and the Issue state.
fn min_queue_row_width(app: &App) -> usize {
    let prefix = app
        .accounts
        .iter()
        .map(|a| Span::raw(account_prefix(&a.name)).width())
        .max()
        .unwrap_or(0);
    prefix + IDENTIFIER_WIDTH + STATE_WIDTH + MIN_TITLE_WIDTH
}

/// One issue row for a list whose rows are `width` cells wide, after an optional prefix.
/// The detail pane always shows the assignee, so the row may drop it to keep the title readable.
fn list_row(issue: &Issue, prefix: Option<Span<'static>>, width: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = prefix.into_iter().collect();
    spans.extend([
        Span::styled(
            format!("{:<IDENTIFIER_WIDTH$}", issue.identifier),
            Style::new().fg(Color::Cyan),
        ),
        Span::styled(
            format!("{:<STATE_WIDTH$}", issue.state.name),
            Style::new().fg(if issue.is_in_review() {
                Color::Yellow
            } else {
                Color::Gray
            }),
        ),
    ]);
    let used: usize = spans.iter().map(Span::width).sum();
    let assignee = assignee_cell(issue);
    if width.saturating_sub(used + assignee.width()) >= MIN_TITLE_WIDTH {
        spans.push(assignee);
    }
    spans.push(Span::raw(crate::display::safe(&issue.title)));
    Line::from(spans)
}

/// Display columns the row's assignee label may use, before its trailing space.
const ASSIGNEE_WIDTH: usize = "Unassigned".len();

/// `you` highlighted, the display name plain, or `Unassigned` dimmed, padded to a fixed width.
fn assignee_cell(issue: &Issue) -> Span<'static> {
    let (label, style) = match &issue.assignee {
        Some(a) if a.is_me => (
            "you".to_string(),
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Some(a) => (crate::display::safe(&a.display_name), Style::new()),
        None => ("Unassigned".to_string(), Style::new().fg(Color::DarkGray)),
    };
    Span::styled(format!("{} ", fit(&label, ASSIGNEE_WIDTH)), style)
}

/// Pad or truncate `text` to exactly `width` display columns, ending cut text with `…`.
fn fit(text: &str, width: usize) -> String {
    let columns = |s: &str| Span::raw(s).width();
    if columns(text) <= width {
        return format!("{text}{}", " ".repeat(width - columns(text)));
    }
    let mut out = String::new();
    for c in text.chars() {
        let mut next = out.clone();
        next.push(c);
        if columns(&next) + 1 > width {
            break;
        }
        out = next;
    }
    out.push('…');
    let used = columns(&out);
    out + &" ".repeat(width - used)
}

fn detail_lines(issue: &Issue) -> Vec<Line<'static>> {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let mut lines = vec![
        Line::from(Span::styled(
            format!("{}  {}", issue.identifier, issue.title),
            bold,
        )),
        Line::from(""),
        Line::from(format!("State:    {}", issue.state.name)),
        Line::from(format!(
            "Assignee: {}",
            issue
                .assignee
                .as_ref()
                .map_or("Unassigned".to_string(), |a| crate::display::safe(&a.name))
        )),
        Line::from(format!("Priority: {}", issue.priority_label())),
        Line::from(format!("Branch:   {}", issue.branch_name)),
        Line::from(format!("Linear:   {}", issue.url)),
        Line::from(""),
        Line::from(Span::styled("Pull requests", bold)),
    ];
    let prs: Vec<_> = issue.pull_requests().collect();
    if prs.is_empty() {
        lines.push(Line::from("  none linked"));
    }
    for pr in prs {
        lines.push(Line::from(format!(
            "  {}",
            pr.title.as_deref().unwrap_or(&pr.url)
        )));
        lines.push(Line::from(format!("  {}", pr.url)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("All attachments", bold)));
    for a in &issue.attachments.nodes {
        lines.push(Line::from(format!(
            "  [{}] {}",
            a.source_type.as_deref().unwrap_or("?"),
            a.url
        )));
    }
    lines
}

fn draw_review(frame: &mut Frame, review: &mut Review, messages: &str) {
    let [status, body, footer] = Layout::vertical([
        Constraint::Length(if frame.area().width < 100 { 3 } else { 2 }),
        Constraint::Min(0),
        Constraint::Length(5),
    ])
    .areas(frame.area());
    frame.render_widget(
        Paragraph::new(review.readiness_line())
            .wrap(Wrap { trim: false })
            .style(Style::new().fg(Color::Cyan)),
        status,
    );
    let [left, right] = if body.width < 80 {
        Layout::vertical([Constraint::Length(5), Constraint::Min(0)]).areas(body)
    } else {
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).areas(body)
    };
    let items: Vec<_> = review.files.iter().map(file_row).collect();
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::bordered().title(format!(
                " {} ({} files) ",
                review.title,
                review.files.len()
            )))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        left,
        &mut review.list,
    );
    let visible = right.height.saturating_sub(2) as usize;
    let mode = if review.full_file {
        "Full-file view"
    } else {
        "Diff"
    };
    let (title, lines): (_, Vec<Line>) = match review.current() {
        Some(file) => {
            let lines = if review.full_file {
                match &file.full {
                    Some(Ok(lines)) => lines
                        .iter()
                        .skip(review.scroll as usize)
                        .take(visible)
                        .cloned()
                        .collect(),
                    Some(Err(error)) => vec![Line::from(Span::styled(
                        error.clone(),
                        Style::new().fg(Color::Red),
                    ))],
                    None => Vec::new(),
                }
            } else {
                file.lines
                    .iter()
                    .skip(review.scroll as usize)
                    .take(visible)
                    .map(|l| diff_line(l))
                    .collect()
            };
            (format!(" {mode}: {} ", file.path), lines)
        }
        None => (
            " Diff ".into(),
            vec![Line::from("This pull request has no changes.")],
        ),
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(title))
            .scroll((0, review.horizontal)),
        right,
    );
    if let (Some(started), Some(file)) = (review.file_loading_since(), review.current()) {
        let path = file.path.clone();
        draw_loading(
            frame,
            right,
            "Loading full file",
            &path,
            started.elapsed(),
            "j/k other file  f diff",
        );
    }
    frame.render_widget(
        Paragraph::new(format!("{messages}\nj/k file  d/u scroll  h/l horizontal\nf full/diff (toggle to retry)  r status  m merge\nesc/q back  Ctrl+C quit"))
            .style(Style::new().fg(Color::DarkGray)),
        footer,
    );
}

fn draw_merge_picker(frame: &mut Frame, review: &mut Review) {
    let (Some(picker), Some(snapshot)) = (&mut review.merge_picker, &review.snapshot) else {
        return;
    };
    let area = frame.area();
    let width = area.width.min(72);
    let height = area.height.min(16);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let block = Block::bordered().title(if picker.confirming {
        " Confirm merge "
    } else {
        " Choose merge method "
    });
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let [details, body, footer] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(crate::display::safe(&format!(
            "PR #{} · {}\n{}",
            snapshot.number, snapshot.repository, snapshot.title
        )))
        .wrap(Wrap { trim: false }),
        details,
    );
    if picker.confirming {
        let method = picker.list.selected().and_then(|i| picker.options.get(i));
        frame.render_widget(
            Paragraph::new(format!(
                "Method: {}\nMerge the head commit you read?",
                method.map_or("unavailable", |m| m.name())
            ))
            .wrap(Wrap { trim: false }),
            body,
        );
    } else if picker.loading || picker.error.is_some() {
        frame.render_widget(
            Paragraph::new(
                picker
                    .error
                    .as_deref()
                    .unwrap_or("Loading allowed merge methods…"),
            )
            .wrap(Wrap { trim: false }),
            body,
        );
    } else {
        let rows: Vec<_> = picker
            .options
            .iter()
            .map(|m| ListItem::new(m.name()))
            .collect();
        frame.render_stateful_widget(
            List::new(rows).highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
            body,
            &mut picker.list,
        );
    }
    frame.render_widget(
        Paragraph::new(if picker.confirming {
            "y merge · any other key returns to picker"
        } else {
            "j/k move · enter confirm · esc/q cancel"
        })
        .wrap(Wrap { trim: false }),
        footer,
    );
}

fn file_row(file: &FileDiff) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled(
            format!("+{} ", file.additions),
            Style::new().fg(Color::Green),
        ),
        Span::styled(format!("-{} ", file.deletions), Style::new().fg(Color::Red)),
        Span::raw(file.path.clone()),
    ]))
}
fn diff_line(line: &str) -> Line<'static> {
    let style = if line.starts_with("@@") {
        Style::new().fg(Color::Cyan)
    } else if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
        Style::new().add_modifier(Modifier::BOLD)
    } else if line.starts_with('+') {
        Style::new().fg(Color::Green)
    } else if line.starts_with('-') {
        Style::new().fg(Color::Red)
    } else {
        Style::new()
    };
    Line::from(Span::styled(crate::display::safe(line), style))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_bar_moves_in_whole_steps_and_stays_in_bounds() {
        let frames: Vec<_> = (0..40).map(|i| pixel_bar(20, STEP * i)).collect();
        for frame in &frames {
            assert_eq!(frame.chars().count(), 20);
            assert_eq!(frame.chars().filter(|&c| c == '█').count(), 5);
        }
        assert_ne!(frames[0], frames[1], "the bar animates");
        assert_eq!(frames[0], pixel_bar(20, STEP / 2), "no sub-step movement");
        // It bounces: back at the start after a full round trip.
        assert_eq!(frames[0], frames[30]);
        assert!(frames.iter().any(|f| f.ends_with('█')));
    }

    #[test]
    fn pixel_bar_handles_degenerate_widths() {
        assert_eq!(pixel_bar(0, Duration::ZERO), "");
        assert_eq!(pixel_bar(1, Duration::from_secs(9)), "█");
        assert_eq!(pixel_bar(3, Duration::from_secs(9)).chars().count(), 3);
    }

    #[test]
    fn fit_pads_or_truncates_to_exact_display_columns() {
        let columns = |s: &str| Span::raw(s).width();
        assert_eq!(fit("ada", 10), "ada       ");
        assert_eq!(fit("abcdefghij", 10), "abcdefghij");
        assert_eq!(fit("abcdefghijk", 10), "abcdefghi…");
        assert_eq!(fit("김진우김진", 10), "김진우김진");
        // A wide character that would cross the boundary is dropped, then padded.
        assert_eq!(fit("김진우김진우", 10), "김진우김… ");
        assert_eq!(fit("a김진우김진", 10), "a김진우김…");
        for name in [
            "",
            "x",
            "김진우김진우김진우",
            "a김진우김진우",
            "e\u{301}e\u{301}e\u{301}",
        ] {
            assert_eq!(columns(&fit(name, 10)), 10, "{name:?}");
        }
    }

    fn unassigned_issue(identifier: &str) -> Issue {
        serde_json::from_value(serde_json::json!({"id":"i","identifier":identifier,"title":"t","url":"https://linear.app/test","priority":0,"branchName":"b","updatedAt":"2026-01-01T00:00:00Z","team":{"id":"team"},"state":{"id":"s","name":"Todo","type":"started"},"attachments":{"nodes":[]}})).unwrap()
    }

    fn shows_assignee(issue: &Issue, prefix: Option<&str>, width: usize) -> bool {
        let prefix = prefix.map(|p| Span::raw(p.to_string()));
        let row = list_row(issue, prefix, width);
        row.spans.iter().any(|s| s.content.contains("Unassigned"))
    }

    #[test]
    fn assignee_column_shows_only_when_the_title_keeps_min_title_width() {
        // Identifier 9 + state 12 + assignee 11 leaves the title exactly 16 at width 48.
        let short = unassigned_issue("HL-1");
        assert!(shows_assignee(&short, None, 32 + MIN_TITLE_WIDTH));
        assert!(!shows_assignee(&short, None, 32 + MIN_TITLE_WIDTH - 1));
        // The prefix counts against the row.
        assert!(shows_assignee(&short, Some("[work] "), 39 + MIN_TITLE_WIDTH));
        assert!(!shows_assignee(&short, Some("[work] "), 38 + MIN_TITLE_WIDTH));
        // An identifier longer than its 9-cell pad is measured, not assumed.
        let long = unassigned_issue("LONGTEAM-12345");
        assert!(shows_assignee(&long, None, 37 + MIN_TITLE_WIDTH));
        assert!(!shows_assignee(&long, None, 36 + MIN_TITLE_WIDTH));
    }
}
