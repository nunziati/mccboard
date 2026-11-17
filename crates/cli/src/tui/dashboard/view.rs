//! How the dashboard looks. It draws the state as it is and decides nothing:
//!
//! ```text
//!  mccboard                                                   Refreshing usage…
//! ┌ Accounts ──────────────────────────────────────────────────────────────────┐
//! │■ A · 35%  a@example.com                                                    │
//! │  Session  ████░░░░░░   35%      3h  0m                                     │
//! │  Weekly   █░░░░░░░░░   14%  4d  0h  0m                                     │
//! │  Fable    ██████░░░░   58%  4d  0h  0m                                     │
//! │■ B · token expired  b@example.com                                          │
//! │  ···                                                                       │
//! │  Token renewal: next try at the next refresh                               │
//! └────────────────────────────────────────────────────────────────────────────┘
//! ┌ Instances ─────────────────────────────────────────────────────────────────┐
//! │  Folder       Type     Account  Login          Drift          Launcher     │
//! │› ~/.claude    primary  A        logged in      no drift       claude       │
//! │  ~/.claude-b  linked   B        token expired  1 item to fix  claude-b     │
//! └────────────────────────────────────────────────────────────────────────────┘
//!  Nothing to repair in ~/.claude.
//!  r refresh  p repair  l language  q quit  ↑↓ select
//! ```
//!
//! Under each account: its session, weekly and model windows, or the widget's placeholder
//! when it has no reading, then the tray's lines on a stale reading and on the token's
//! renewal. `p` asks in a box over the screen, listing the drift items it would fix.
//!
//! Each account is drawn in its color as the taskbar widget adjusts it for a dark taskbar,
//! the usual terminal. A stale reading is drawn dim, as the widget fades it.

use mccboard_core::{SEGMENTS, TaskbarTheme, TimeToReset, WidgetColumn, WidgetRow};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, TableState};

use super::state::{Confirm, Dashboard};
use crate::tui::texts::Texts;
use crate::tui::widgets::{self, Instances, dim, key_hints, rgb};

/// The widest a window's name is shown (model names can be long).
const NAME_WIDTH: usize = 16;

/// The drift items a repair question lists before "… and N more".
const ITEMS_SHOWN: usize = 8;

/// The mark before the selected instance.
const SELECTED: &str = "› ";

pub fn draw(dashboard: &Dashboard, frame: &mut Frame) {
    let texts = dashboard.texts();
    let accounts = account_lines(dashboard, texts);
    let area = frame.area();
    // The accounts take the room they need, leaving the instances a few rows.
    let accounts_height = (accounts.len() as u16 + 2).min(area.height.saturating_sub(8).max(3));
    let [header, accounts_area, instances_area, notice, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(accounts_height),
        Constraint::Min(4),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(dashboard, texts, frame, header);
    frame.render_widget(
        Paragraph::new(accounts).block(Block::bordered().title(format!(" {} ", texts.accounts()))),
        accounts_area,
    );
    draw_instances(dashboard, texts, frame, instances_area);
    if let (Some(text), Some(notice_kind)) = (dashboard.notice_text(), &dashboard.notice) {
        let style = if notice_kind.is_error() {
            Style::new().fg(Color::Red)
        } else {
            Style::new().fg(Color::Cyan)
        };
        frame.render_widget(Paragraph::new(format!(" {text}")).style(style), notice);
    }
    frame.render_widget(Paragraph::new(key_hints(&texts.dashboard_keys())), keys);
    if let Some(confirm) = &dashboard.confirm {
        draw_confirm(dashboard, texts, confirm, frame, area);
    }
}

fn draw_header(dashboard: &Dashboard, texts: Texts, frame: &mut Frame, area: Rect) {
    frame.render_widget(Paragraph::new(" mccboard").bold(), area);
    if dashboard.refreshing {
        frame.render_widget(
            Paragraph::new(format!("{} ", texts.refreshing()))
                .alignment(Alignment::Right)
                .dim(),
            area,
        );
    }
}

/// Every account with its windows, in the taskbar widget's order (`config.toml`'s).
fn account_lines(dashboard: &Dashboard, texts: Texts) -> Vec<Line<'static>> {
    let tray = &dashboard.tray;
    if tray.no_instances {
        return vec![Line::from(texts.no_instances())];
    }
    // Why usage can't be shown at all.
    if let Some(notice) = &tray.notice {
        return vec![Line::from(notice.clone()).red()];
    }
    let columns = &dashboard.widget.columns;
    if columns.is_empty() {
        return vec![Line::from(texts.no_accounts())];
    }
    let width = columns
        .iter()
        .flat_map(|column| column.models.iter().map(|m| m.model.chars().count()))
        .chain([
            texts.session().chars().count(),
            texts.weekly().chars().count(),
        ])
        .max()
        .unwrap_or(0)
        .min(NAME_WIDTH);
    let mut lines = Vec::new();
    for column in columns {
        let Some(account) = tray
            .accounts
            .iter()
            .find(|a| a.account.uuid == column.account)
        else {
            continue;
        };
        let ink = column.ink(TaskbarTheme::Dark);
        let mut head = vec![
            Span::raw("■ ").fg(rgb(&ink.bar)),
            Span::raw(account.title.clone()).fg(rgb(&ink.text)).bold(),
        ];
        if let Some(email) = &account.account.email {
            head.push(Span::raw(format!("  {email}")).dim());
        }
        lines.push(Line::from(head));
        match column.state.placeholder() {
            Some(placeholder) => lines.push(Line::from(format!("  {placeholder}")).dim()),
            None => lines.extend(window_lines(column, texts, width)),
        }
        // After its windows, the tray tells when a stale reading was read and how the token's
        // renewal went.
        for line in account.lines.iter().skip(account.windows.len()) {
            lines.push(Line::from(format!("  {line}")).yellow());
        }
    }
    lines
}

fn window_lines(column: &WidgetColumn, texts: Texts, width: usize) -> Vec<Line<'static>> {
    let color = rgb(&column.ink(TaskbarTheme::Dark).bar);
    let base = if column.state.faded() {
        dim()
    } else {
        Style::new()
    };
    let main = [
        (texts.session().to_owned(), &column.session),
        (texts.weekly().to_owned(), &column.weekly),
    ];
    let models = column.models.iter().map(|m| (m.model.clone(), &m.row));
    main.into_iter()
        .chain(models)
        .map(|(name, row)| window_line(&name, row, width, color, base))
        .collect()
}

/// "  Session   ████░░░░░░  35%      2h 14m".
fn window_line(
    name: &str,
    row: &WidgetRow,
    width: usize,
    color: Color,
    base: Style,
) -> Line<'static> {
    let name: String = name.chars().take(width).collect();
    let filled = usize::from(row.segments.min(SEGMENTS));
    let empty = usize::from(SEGMENTS) - filled;
    Line::from(vec![
        Span::styled(format!("  {name:<width$}  "), base),
        Span::styled("█".repeat(filled), base.fg(color)),
        Span::styled("░".repeat(empty), base.fg(Color::DarkGray)),
        Span::styled(format!("  {:>3}%  ", row.percent), base),
        Span::styled(format!("{:>10}", reset_text(&row.reset)), base),
    ])
}

/// The time to reset with every number in its slot, for units to line up from row to row:
/// "4d  2h  0m", "    2h 14m".
fn reset_text(reset: &TimeToReset) -> String {
    if reset.parts.is_empty() {
        return TimeToReset::IDLE.to_owned();
    }
    reset
        .parts
        .iter()
        .map(|part| {
            format!(
                "{:>slot$}{}",
                part.value,
                part.unit,
                slot = usize::from(part.slot)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn draw_instances(dashboard: &Dashboard, texts: Texts, frame: &mut Frame, area: Rect) {
    let block = Block::bordered().title(format!(" {} ", texts.instances()));
    let registry = match &dashboard.registry {
        Err(failure) => {
            let text = texts.instances_unreadable(failure);
            frame.render_widget(Paragraph::new(text).red().block(block), area);
            return;
        }
        Ok(registry) if registry.instances.is_empty() => {
            frame.render_widget(Paragraph::new(texts.no_instances()).block(block), area);
            return;
        }
        Ok(registry) => registry,
    };
    let instances = Instances {
        home: dashboard.core.home(),
        registry,
        doctor: dashboard.doctor.as_ref(),
        launchers: dashboard.launchers.as_ref(),
    };
    // Inside the borders, after the column of the selection mark.
    let width = area
        .width
        .saturating_sub(2 + SELECTED.chars().count() as u16);
    let table = widgets::instances_table(texts, &instances, width)
        .block(block)
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol(SELECTED);
    let mut state = TableState::default().with_selected(Some(dashboard.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

/// The repair question over the screen: the instance, the items it would fix, the answers.
fn draw_confirm(
    dashboard: &Dashboard,
    texts: Texts,
    confirm: &Confirm,
    frame: &mut Frame,
    area: Rect,
) {
    let mut lines = vec![
        Line::from(texts.confirm_repair(&dashboard.shown_path(&confirm.config_dir))).bold(),
        Line::default(),
    ];
    if !confirm.items.is_empty() {
        lines.push(Line::from(texts.repair_items(confirm.items.len())));
        for id in confirm.items.iter().take(ITEMS_SHOWN) {
            lines.push(Line::from(format!("  {id}")));
        }
        if confirm.items.len() > ITEMS_SHOWN {
            lines.push(Line::from(texts.more_items(confirm.items.len() - ITEMS_SHOWN)).dim());
        }
        lines.push(Line::default());
    }
    lines.push(key_hints(&texts.confirm_keys()));
    widgets::question(frame, area, lines);
}
