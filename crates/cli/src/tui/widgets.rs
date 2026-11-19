//! Pieces of drawing that the dashboard and the setup share: the key hints, the colors of
//! accounts, the table of registered instances and the box that asks a question.

use std::path::Path;

use mccboard_core::{
    Doctor, InstanceKind, InstanceStatus, LauncherState, LauncherStatus, LoginState, Registry,
    Warning,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table};

use super::texts::Texts;

/// "r refresh  p repair …", keys bold and actions dim.
pub fn key_hints(keys: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (index, (key, action)) in keys.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::raw((*key).to_owned()).bold());
        spans.push(Span::raw(format!(" {action}")).dim());
    }
    Line::from(spans)
}

pub fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

/// An account's color on the terminal.
pub fn rgb(color: &mccboard_core::Color) -> Color {
    let [r, g, b] = color.rgb();
    Color::Rgb(r, g, b)
}

/// What the table of instances shows: the registered instances, with their drift and their
/// launchers when these could be read.
pub struct Instances<'a> {
    pub home: &'a Path,
    pub registry: &'a Registry,
    pub doctor: Option<&'a Doctor>,
    pub launchers: Option<&'a LauncherStatus>,
}

/// The space between two columns of the table of instances.
const COLUMN_SPACING: u16 = 2;

/// The instances as a table, a header row first: folder, kind, account, login, drift and
/// launcher of each, every column as wide as its widest cell. When the table has less than
/// `width` for its cells, the widest of the other columns gives up room first: the folder
/// tells the rows apart.
pub fn instances_table(texts: Texts, instances: &Instances, width: u16) -> Table<'static> {
    let header = texts.instance_columns().map(str::to_owned);
    let rows: Vec<[(String, Style); 6]> = instances
        .registry
        .instances
        .iter()
        .map(|instance| instance_cells(texts, instances, instance))
        .collect();
    let mut widths: Vec<u16> = (0..header.len())
        .map(|i| {
            rows.iter()
                .map(|row| row[i].0.chars().count())
                .chain([header[i].chars().count()])
                .max()
                .unwrap_or(0) as u16
        })
        .collect();
    let room = width.saturating_sub(COLUMN_SPACING * (widths.len() as u16 - 1));
    while widths.iter().sum::<u16>() > room {
        let Some(widest) = (1..widths.len())
            .filter(|i| widths[*i] > 1)
            .max_by_key(|i| widths[*i])
        else {
            break;
        };
        widths[widest] -= 1;
    }
    let rows = rows.into_iter().map(|cells| {
        Row::new(
            cells
                .into_iter()
                .map(|(text, style)| Cell::from(text).style(style)),
        )
    });
    Table::new(rows, widths.into_iter().map(Constraint::Length))
        .header(Row::new(header).bold())
        .column_spacing(COLUMN_SPACING)
}

/// Folder, kind, account, login, drift and launcher of an instance, each with its style.
fn instance_cells(
    texts: Texts,
    instances: &Instances,
    instance: &InstanceStatus,
) -> [(String, Style); 6] {
    let plain = Style::new();
    let warn = Style::new().fg(Color::Yellow);
    let path = super::shown_path(instances.home, &instance.config_dir);
    let kind = match instance.kind {
        InstanceKind::Primary => texts.primary(),
        InstanceKind::Linked => texts.linked(),
    };
    let account = instance.registered_account.as_ref().map(|uuid| {
        match instances.registry.accounts.iter().find(|a| &a.uuid == uuid) {
            Some(account) => (account.label.clone(), Style::new().fg(rgb(&account.color))),
            // Registered with an account no longer in config.toml.
            None => (uuid.as_str().chars().take(8).collect(), plain),
        }
    });
    let account = account.unwrap_or_else(|| (texts.no_account().to_owned(), dim()));
    let login = login_cell(texts, instance);
    let drift = instances
        .doctor
        .and_then(|doctor| {
            doctor
                .instances
                .iter()
                .find(|d| d.config_dir == instance.config_dir)
        })
        .map_or(("?".to_owned(), dim()), |drift| match drift.drift_count {
            0 => (texts.drift(0), dim()),
            count => (texts.drift(count), warn),
        });
    let launcher = match instance.kind {
        // The primary instance keeps plain `claude`.
        InstanceKind::Primary => ("claude".to_owned(), plain),
        InstanceKind::Linked => match instances
            .launchers
            .and_then(|launchers| launchers.instance(&instance.config_dir))
        {
            Some(launcher) => match &launcher.state {
                LauncherState::Ok => (launcher.name.clone(), plain),
                LauncherState::Missing => (texts.launcher_missing(&launcher.name), warn),
                LauncherState::Stale { .. } => (texts.launcher_stale(&launcher.name), warn),
            },
            None => ("?".to_owned(), dim()),
        },
    };
    [
        (path, plain),
        (kind.to_owned(), plain),
        account,
        login,
        drift,
        launcher,
    ]
}

/// The login of an instance in a word or two, yellow when it needs attention and red when its
/// folder is gone.
pub fn login_cell(texts: Texts, instance: &InstanceStatus) -> (String, Style) {
    let warn = Style::new().fg(Color::Yellow);
    if instance
        .observation
        .warnings
        .contains(&Warning::ConfigDirMissing)
    {
        return (
            texts.folder_missing().to_owned(),
            Style::new().fg(Color::Red),
        );
    }
    match &instance.observation.login {
        LoginState::LoggedIn { .. } => (texts.logged_in().to_owned(), Style::new()),
        LoginState::Expired { .. } => (texts.token_expired().to_owned(), warn),
        LoginState::NotLoggedIn => (texts.not_logged_in().to_owned(), warn),
        LoginState::Unknown => (texts.login_unknown().to_owned(), warn),
    }
}

/// A question in a yellow box over the middle of `area`, as wide and high as its lines.
pub fn question(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let width = (lines.iter().map(Line::width).max().unwrap_or(0) as u16 + 4).min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = area.centered(Constraint::Length(width), Constraint::Length(height));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().border_style(Style::new().fg(Color::Yellow))),
        popup,
    );
}
