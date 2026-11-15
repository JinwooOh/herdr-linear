use std::time::Duration;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{App, View},
    linear::Issue,
    review::{FileDiff, Review},
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let messages = messages(app);
    match &mut app.view {
        View::Review(review) => {
            draw_review(frame, review, &messages);
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
    let [left, right] = if body.width < 80 {
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body)
    } else {
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(body)
    };
    let title = account_tabs(app);
    let all = app.all_accounts_queue();
    let items: Vec<ListItem> = if all {
        app.queue
            .iter()
            .map(|&(a, i)| {
                let issue = &app.accounts[a].issues[i];
                let mut row = list_row(issue);
                row.spans.insert(
                    0,
                    Span::styled(
                        format!("[{}] ", app.accounts[a].name),
                        Style::new().fg(Color::Magenta),
                    ),
                );
                ListItem::new(row)
            })
            .collect()
    } else {
        app.accounts[app.active]
            .issues
            .iter()
            .map(|i| ListItem::new(list_row(i)))
            .collect()
    };
    let list = List::new(items)
        .block(Block::bordered().title(title))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    if all {
        frame.render_stateful_widget(list, left, &mut app.queue_list);
    } else {
        frame.render_stateful_widget(list, left, &mut app.accounts[app.active].list);
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
        None => vec![Line::from(
            "No open issues assigned to you (or still loading).",
        )],
    };
    frame.render_widget(
        Paragraph::new(detail)
            .block(Block::bordered().title(" Detail "))
            .wrap(Wrap { trim: false }),
        right,
    );
    let activity = if app.accounts.iter().any(|a| a.loading()) {
        "refreshing…"
    } else {
        ""
    };
    let hint = format!("{activity}\n{messages}\nj/k move  g/G ends  tab/1-9 Account  a all\nenter PR  s state  r refresh  q/Esc quit  Ctrl+C quit");
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
    let errors = app
        .accounts
        .iter()
        .filter_map(|a| {
            a.error
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
    let mut spans = vec![Span::raw(" ")];
    for (index, account) in app.accounts.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" | "));
        }
        let marker = if account.error.is_some() {
            " !"
        } else if account.loading() {
            " …"
        } else {
            ""
        };
        let label = format!("{} ({}){}", account.name, account.issues.len(), marker);
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
    spans.push(Span::styled(format!("All-accounts queue ({}) ", app.queue.len()), style));
    Line::from(spans)
}

fn list_row(issue: &Issue) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<9}", issue.identifier),
            Style::new().fg(Color::Cyan),
        ),
        Span::styled(
            format!("{:<12}", issue.state.name),
            Style::new().fg(if issue.is_in_review() {
                Color::Yellow
            } else {
                Color::Gray
            }),
        ),
        Span::raw(crate::display::safe(&issue.title)),
    ])
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
    let [body, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(4)]).areas(frame.area());
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
        Paragraph::new(format!("{messages}\nj/k file  d/u scroll  h/l horizontal\nf full/diff (toggle to retry)  esc/q back  Ctrl+C quit"))
            .style(Style::new().fg(Color::DarkGray)),
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
}
