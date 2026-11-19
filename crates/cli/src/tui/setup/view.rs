//! How the setup looks. It draws the state as it is and decides nothing:
//!
//! ```text
//!  mccboard setup
//! ┌ Steps ───────────────┐┌ 3. Accounts ─────────────────────────────────────────┐
//! │ ✓ 1 Claude Code      ││Each account logs in to an instance of its own: …     │
//! │ ✓ 2 Instances        ││                                                      │
//! │ · 3 Accounts         ││  ~/.claude    A           a@example.com  logged in   │
//! │ ✓ 4 Sharing          ││› ~/.claude-c  no account                 not logged …│
//! │ · 5 Drift            ││                                                      │
//! │ ✓ 6 Launchers        ││                                                      │
//! │ ✓ 7 Preferences      ││                                                      │
//! │   8 Summary          ││                                                      │
//! └──────────────────────┘└──────────────────────────────────────────────────────┘
//!  ~/.claude-c is still not logged in: Enter on it opens the login again.
//!  ↑↓ select  a add an account  Enter log in  ←→ step  q quit
//! ```
//!
//! The list of steps marks with ✓ the ones whose work is done and with · the others, and
//! shows the current one in reverse video. The installer and a repair ask in a box over the
//! screen.

use std::path::Path;

use mccboard_core::{
    CLAUDE_CODE_INSTALL_COMMAND, CLAUDE_CODE_WINDOWS_INSTALL_COMMAND, ClaudeCode, InstanceKind,
    LauncherPlace, LauncherState, LoginState, Observation,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use super::state::{Setup, StepId};
use super::steps::{AddForm, Preference, needs_login};
use super::texts::Hint;
use crate::tui::Show;
use crate::tui::texts::Texts;
use crate::tui::widgets::{self, Instances, dim, key_hints, login_cell, rgb};

/// The width of the list of steps.
const STEPS_WIDTH: u16 = 24;

/// Below this width the list of steps becomes one line over the step (80 columns, the usual
/// SSH terminal, leave the step too little room beside it).
const SIDE_LIST_FROM: u16 = 100;

/// The drift items a repair question lists before "… and N more".
const ITEMS_SHOWN: usize = 8;

pub fn draw(setup: &Setup, frame: &mut Frame) {
    let texts = setup.texts();
    let area = frame.area();
    let [header, body, notice, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(format!(" {}", texts.setup_title())).bold(),
        header,
    );
    if area.width >= SIDE_LIST_FROM {
        let [steps_area, step_area] =
            Layout::horizontal([Constraint::Length(STEPS_WIDTH), Constraint::Min(20)]).areas(body);
        draw_steps(setup, texts, frame, steps_area);
        draw_step(setup, texts, frame, step_area);
    } else {
        let [steps_area, step_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(4)]).areas(body);
        frame.render_widget(Paragraph::new(steps_line(setup)), steps_area);
        draw_step(setup, texts, frame, step_area);
    }
    if let (Some(text), Some(kind)) = (setup.notice_text(), &setup.notice) {
        let style = if kind.is_error() {
            Style::new().fg(Color::Red)
        } else {
            Style::new().fg(Color::Cyan)
        };
        frame.render_widget(
            Paragraph::new(format!(" {text}"))
                .style(style)
                .wrap(Wrap { trim: false }),
            notice,
        );
    }
    frame.render_widget(Paragraph::new(key_hints(&hints(setup, texts))), keys);
    draw_question(setup, texts, frame, area);
}

/// The mark of `step`: ✓ when its work is done, · when not, none for the summary.
fn mark(setup: &Setup, step: StepId) -> Span<'static> {
    match setup.done(step) {
        Some(true) => Span::raw("✓ ").green(),
        Some(false) => Span::raw("· ").yellow(),
        None => Span::raw("  "),
    }
}

/// The steps on one line, each by its number with its mark, the one shown highlighted: on a
/// narrow terminal, where its name is in the title of the step.
fn steps_line(setup: &Setup) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for step in StepId::ALL {
        let item = Line::from(vec![
            mark(setup, step),
            Span::raw(step.number().to_string()),
        ]);
        let style = if step == setup.step() {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        spans.extend(item.spans.into_iter().map(|span| span.patch_style(style)));
        spans.push(Span::raw("  "));
    }
    Line::from(spans)
}

/// The steps, each marked done or not, the one shown highlighted.
fn draw_steps(setup: &Setup, texts: Texts, frame: &mut Frame, area: Rect) {
    let lines: Vec<Line> = StepId::ALL
        .iter()
        .map(|step| {
            let mark = mark(setup, *step);
            let name = Span::raw(format!("{} {}", step.number(), texts.step_name(*step)));
            let line = Line::from(vec![Span::raw(" "), mark, name]);
            if *step == setup.step() {
                line.add_modifier(Modifier::REVERSED)
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(format!(" {} ", texts.steps()))),
        area,
    );
}

fn draw_step(setup: &Setup, texts: Texts, frame: &mut Frame, area: Rect) {
    let step = setup.step();
    let block = Block::bordered().title(format!(" {}. {} ", step.number(), texts.step_name(step)));
    if step == StepId::Summary {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        draw_summary(setup, texts, frame, inner);
        return;
    }
    let lines = match step {
        StepId::ClaudeCode => claude_code_lines(setup, texts),
        StepId::Instances => instances_lines(setup, texts),
        StepId::Accounts => accounts_lines(setup, texts),
        StepId::Sharing => sharing_lines(setup, texts),
        StepId::Drift => drift_lines(setup, texts),
        StepId::Launchers => launchers_lines(setup, texts),
        StepId::Preferences => preferences_lines(setup, texts),
        StepId::Summary => Vec::new(),
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

/// A command to type, set apart.
fn command(text: &str) -> Line<'static> {
    Line::from(format!("  {text}")).bold()
}

/// `rows` of cells as lines, every column as wide as its widest cell. The selected row starts
/// with ›.
fn rows(rows: Vec<Vec<(String, Style)>>, selected: Option<usize>) -> Vec<Line<'static>> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|row| row.get(i))
                .map(|(text, _)| text.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.into_iter()
        .enumerate()
        .map(|(index, cells)| {
            let mark = if Some(index) == selected {
                "› "
            } else {
                "  "
            };
            let mut spans = vec![Span::raw(mark)];
            let last = cells.len().saturating_sub(1);
            for (i, (text, style)) in cells.into_iter().enumerate() {
                let text = if i == last {
                    text
                } else {
                    format!("{text:<width$}  ", width = widths[i])
                };
                spans.push(Span::styled(text, style));
            }
            let line = Line::from(spans);
            if Some(index) == selected {
                line.bold()
            } else {
                line
            }
        })
        .collect()
}

fn claude_code_lines(setup: &Setup, texts: Texts) -> Vec<Line<'static>> {
    let step = &setup.claude_code;
    let Some(claude_code) = &step.claude_code else {
        return vec![Line::from(texts.looking_for_claude_code()).dim()];
    };
    match claude_code {
        ClaudeCode::Found {
            program,
            on_path,
            version,
        } => {
            let mut lines = vec![Line::from(
                texts.claude_found(&setup.shown_path(program), version.as_deref()),
            )];
            if !on_path && let Some(dir) = program.parent() {
                lines.push(Line::default());
                lines.push(Line::from(texts.claude_not_on_path(&setup.shown_path(dir))).yellow());
            }
            lines
        }
        ClaudeCode::Missing if step.can_install() => vec![
            Line::from(texts.claude_missing()),
            Line::default(),
            command(CLAUDE_CODE_INSTALL_COMMAND),
            Line::default(),
            Line::from(texts.install_after_asking()),
        ],
        ClaudeCode::Missing => vec![
            Line::from(texts.claude_missing_windows()),
            Line::default(),
            command(CLAUDE_CODE_WINDOWS_INSTALL_COMMAND),
        ],
    }
}

/// An observed account and login in words: the email, and the login state.
fn observed(texts: Texts, observation: &Observation) -> [(String, Style); 2] {
    let account = match &observation.account {
        Some(account) => (
            account
                .email
                .clone()
                .unwrap_or_else(|| account.uuid.to_string()),
            Style::new(),
        ),
        None => (texts.no_account().to_owned(), dim()),
    };
    let login = match observation.login {
        LoginState::LoggedIn { .. } => (texts.logged_in().to_owned(), Style::new()),
        LoginState::Expired { .. } => (texts.token_expired().to_owned(), Style::new()),
        LoginState::NotLoggedIn => (texts.not_logged_in().to_owned(), Style::new().yellow()),
        LoginState::Unknown => (texts.login_unknown().to_owned(), Style::new().yellow()),
    };
    [account, login]
}

fn kind(texts: Texts, kind: InstanceKind) -> &'static str {
    match kind {
        InstanceKind::Primary => texts.primary(),
        InstanceKind::Linked => texts.linked(),
    }
}

fn instances_lines(setup: &Setup, texts: Texts) -> Vec<Line<'static>> {
    let step = &setup.instances;
    let mut lines = vec![Line::from(texts.instances_intro()), Line::default()];
    if let Err(failure) = &step.candidates {
        lines.push(Line::from(texts.failure(failure)).red());
        return lines;
    }
    let table = step
        .list()
        .iter()
        .map(|candidate| {
            let mark = if candidate.registered {
                ("✓".to_owned(), Style::new().green())
            } else if step.is_checked(candidate) {
                ("[x]".to_owned(), Style::new())
            } else {
                ("[ ]".to_owned(), Style::new())
            };
            let [account, login] = observed(texts, &candidate.observation);
            let state = if candidate.registered {
                (format!("{}, {}", login.0, texts.registered_word()), login.1)
            } else {
                login
            };
            vec![
                mark,
                (setup.shown_path(&candidate.config_dir), Style::new()),
                (kind(texts, candidate.kind).to_owned(), Style::new()),
                account,
                state,
            ]
        })
        .collect();
    lines.extend(rows(table, Some(step.selected)));
    if step.primary().is_none() {
        lines.push(Line::default());
        lines.push(Line::from(texts.no_primary()).yellow());
    }
    lines
}

fn accounts_lines(setup: &Setup, texts: Texts) -> Vec<Line<'static>> {
    let step = &setup.accounts;
    let mut lines = vec![Line::from(texts.accounts_intro()), Line::default()];
    match &step.registry {
        Err(failure) => lines.push(Line::from(texts.failure(failure)).red()),
        Ok(registry) if registry.instances.is_empty() => {
            lines.push(Line::from(texts.no_registered_instance()));
        }
        Ok(registry) => {
            let table = registry
                .instances
                .iter()
                .map(|instance| {
                    let account = instance
                        .registered_account
                        .as_ref()
                        .and_then(|uuid| registry.accounts.iter().find(|a| &a.uuid == uuid));
                    let (label, email) = match account {
                        Some(account) => (
                            (account.label.clone(), Style::new().fg(rgb(&account.color))),
                            (account.email.clone().unwrap_or_default(), Style::new()),
                        ),
                        None => (
                            (texts.no_account().to_owned(), dim()),
                            (String::new(), Style::new()),
                        ),
                    };
                    vec![
                        (setup.shown_path(&instance.config_dir), Style::new()),
                        label,
                        email,
                        login_cell(texts, instance),
                    ]
                })
                .collect();
            // The selection matters only while no form is typed.
            let selected = step.form.is_none().then_some(step.selected);
            lines.extend(rows(table, selected));
        }
    }
    match &step.form {
        None => {}
        Some(AddForm::Label { text }) => {
            lines.push(Line::default());
            lines.push(Line::from(texts.label_prompt()).bold());
            lines.push(field(text));
        }
        Some(AddForm::Folder { label, text, .. }) => {
            lines.push(Line::default());
            lines.push(Line::from(texts.folder_prompt(label)).bold());
            lines.push(field(text));
        }
    }
    lines
}

/// A text field with its cursor at the end.
fn field(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  > "),
        Span::raw(text.to_owned()).bold(),
        Span::raw("█").add_modifier(Modifier::SLOW_BLINK),
    ])
}

fn sharing_lines(setup: &Setup, texts: Texts) -> Vec<Line<'static>> {
    let step = &setup.sharing;
    if let Err(failure) = &step.questions {
        return vec![Line::from(texts.failure(failure)).red()];
    }
    let questions = step.questions();
    if questions.is_empty() {
        return vec![Line::from(texts.nothing_to_ask())];
    }
    let mut lines = vec![Line::from(texts.sharing_intro()), Line::default()];
    if let Some(asked) = step.asked() {
        let number = questions
            .iter()
            .position(|q| q.item == asked.item)
            .unwrap_or(0)
            + 1;
        lines.push(
            Line::from(texts.share_question(&asked.item, asked.is_dir, number, questions.len()))
                .bold(),
        );
    }
    let skipped: Vec<&str> = questions
        .iter()
        .filter(|q| step.skipped.contains(&q.item))
        .map(|q| q.item.as_str())
        .collect();
    if !skipped.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(texts.skipped(&skipped.join(", "))).dim());
    }
    lines
}

fn drift_lines(setup: &Setup, texts: Texts) -> Vec<Line<'static>> {
    let step = &setup.drift;
    let doctor = match &step.doctor {
        Err(failure) => return vec![Line::from(texts.failure(failure)).red()],
        Ok(doctor) if doctor.instances.is_empty() => {
            return vec![Line::from(texts.no_registered_instance())];
        }
        Ok(doctor) => doctor,
    };
    let mut lines = vec![Line::from(texts.drift_intro()), Line::default()];
    let table = doctor
        .instances
        .iter()
        .map(|instance| {
            let style = if instance.drift_count > 0 {
                Style::new().yellow()
            } else {
                dim()
            };
            vec![
                (setup.shown_path(&instance.config_dir), Style::new()),
                (kind(texts, instance.kind).to_owned(), Style::new()),
                (texts.drift(instance.drift_count), style),
            ]
        })
        .collect();
    lines.extend(rows(table, Some(step.selected)));
    if let Some(instance) = step.selected_instance()
        && !instance.items.is_empty()
    {
        lines.push(Line::default());
        let path = |p: &Path| setup.shown_path(p);
        for item in &instance.items {
            lines.push(Line::from(format!("  {}", item.id)).bold());
            lines.push(Line::from(format!(
                "    {}",
                texts.repair_action(&item.repair, &item.item, &path)
            )));
        }
    }
    lines
}

fn launchers_lines(setup: &Setup, texts: Texts) -> Vec<Line<'static>> {
    let status = match &setup.launchers.status {
        Err(failure) => return vec![Line::from(texts.failure(failure)).red()],
        Ok(status) => status,
    };
    let path = |p: &Path| setup.shown_path(p);
    let mut lines = vec![Line::from(texts.launchers_place(&status.place, &path))];
    lines.push(Line::default());
    if status.instances.is_empty() && status.orphans.is_empty() {
        lines.push(Line::from(texts.no_launchers_needed()));
        return lines;
    }
    let mut table: Vec<Vec<(String, Style)>> = status
        .instances
        .iter()
        .map(|launcher| {
            let state = match &launcher.state {
                LauncherState::Ok => (texts.launcher_ok().to_owned(), Style::new().green()),
                LauncherState::Missing => (
                    texts.launcher_missing(&launcher.name),
                    Style::new().yellow(),
                ),
                LauncherState::Stale { .. } => {
                    (texts.launcher_stale(&launcher.name), Style::new().yellow())
                }
            };
            vec![
                (launcher.name.clone(), Style::new().bold()),
                (setup.shown_path(&launcher.config_dir), Style::new()),
                state,
            ]
        })
        .collect();
    table.extend(status.orphans.iter().map(|orphan| {
        vec![
            (orphan.name.clone(), Style::new().bold()),
            (
                orphan
                    .config_dir
                    .as_deref()
                    .map_or_else(|| "?".to_owned(), |dir| setup.shown_path(dir)),
                Style::new(),
            ),
            (texts.launcher_orphan(&orphan.name), Style::new().yellow()),
        ]
    }));
    lines.extend(rows(table, None));
    if !status.instances.is_empty() {
        lines.push(Line::default());
        match &status.place {
            LauncherPlace::ScriptFolder { path: scripts } => {
                lines.push(Line::from(texts.use_here_scripts()));
                lines.push(command(&format!(
                    "export PATH=\"{}:$PATH\"",
                    scripts.display()
                )));
            }
            LauncherPlace::BinFolder { path: bin, .. } => {
                lines.push(Line::from(texts.use_here_windows()));
                lines.push(command(&format!("$env:Path += \";{}\"", bin.display())));
            }
        }
    }
    lines
}

fn preferences_lines(setup: &Setup, texts: Texts) -> Vec<Line<'static>> {
    let step = &setup.preferences;
    let renew = step.renew.as_ref().ok().copied();
    let table = vec![
        vec![(texts.language_set(), Style::new())],
        vec![(texts.renewal_row(renew), Style::new())],
    ];
    let selected = match step.selected {
        Preference::Language => 0,
        Preference::Renewal => 1,
    };
    let mut lines = rows(table, Some(selected));
    lines.push(Line::default());
    lines.push(Line::from(texts.renewal_hint()).dim());
    if let Err(failure) = &step.renew {
        lines.push(Line::default());
        lines.push(Line::from(texts.failure(failure)).red());
    }
    lines
}

fn draw_summary(setup: &Setup, texts: Texts, frame: &mut Frame, area: Rect) {
    let step = &setup.summary;
    let registry = match &step.registry {
        Err(failure) => {
            frame.render_widget(
                Paragraph::new(texts.failure(failure))
                    .red()
                    .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }
        Ok(registry) => registry,
    };
    let mut accounts = vec![Line::from(format!("{}:", texts.accounts())).bold()];
    if registry.accounts.is_empty() {
        accounts.push(Line::from(format!("  {}", texts.no_accounts_yet())));
    }
    for account in &registry.accounts {
        accounts.push(Line::from(vec![
            Span::raw("  ■ ").fg(rgb(&account.color)),
            Span::raw(account.label.clone())
                .fg(rgb(&account.color))
                .bold(),
            Span::raw(format!(
                "  {}",
                account.email.as_deref().unwrap_or_default()
            ))
            .dim(),
        ]));
    }
    let mut end = Vec::new();
    let left: Vec<&str> = StepId::ALL
        .iter()
        .filter(|step| setup.done(**step) == Some(false))
        .map(|step| texts.step_name(*step))
        .collect();
    if !left.is_empty() {
        end.push(Line::from(texts.still_to_do(&left.join(", "))).yellow());
    }
    end.push(Line::from(texts.summary_end()));
    let table_height = if registry.instances.is_empty() {
        0
    } else {
        registry.instances.len() as u16 + 2
    };
    let [accounts_area, instances_area, end_area] = Layout::vertical([
        Constraint::Length(accounts.len() as u16 + 1),
        Constraint::Length(table_height),
        Constraint::Min(2),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(accounts), accounts_area);
    if !registry.instances.is_empty() {
        let instances = Instances {
            home: setup.core.home(),
            registry,
            doctor: step.doctor.as_ref(),
            launchers: step.launchers.as_ref(),
        };
        let [title, table] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(instances_area);
        frame.render_widget(
            Paragraph::new(format!("{}:", texts.instances())).bold(),
            title,
        );
        frame.render_widget(
            widgets::instances_table(texts, &instances, table.width),
            table,
        );
    }
    frame.render_widget(Paragraph::new(end).wrap(Wrap { trim: false }), end_area);
}

/// The keys the setup answers now, with what they do.
fn hints(setup: &Setup, texts: Texts) -> Vec<(&'static str, &'static str)> {
    if setup.leaving.is_some() || setup.claude_code.confirm || setup.drift.confirm {
        return Vec::new();
    }
    let quit = match setup.back {
        Show::Dashboard => Hint::ToDashboard,
        Show::Setup | Show::Quit => Hint::Quit,
    };
    // While a repair runs only `q` answers, and it waits for the repair to end.
    if setup.busy() {
        return vec![texts.hint(quit)];
    }
    let mut hints = match setup.step() {
        StepId::ClaudeCode => match &setup.claude_code.claude_code {
            None => Vec::new(),
            Some(ClaudeCode::Missing) if setup.claude_code.can_install() => vec![Hint::Install],
            Some(_) => vec![Hint::Continue],
        },
        StepId::Instances if !setup.instances.checked.is_empty() => {
            vec![Hint::Select, Hint::Check, Hint::Register]
        }
        StepId::Instances if setup.instances.offers_first_login() => {
            vec![Hint::Select, Hint::Check, Hint::LogIn]
        }
        StepId::Instances => vec![Hint::Select, Hint::Check, Hint::Continue],
        StepId::Accounts => match &setup.accounts.form {
            Some(AddForm::Label { .. }) => {
                return [Hint::NextField, Hint::Close]
                    .map(|hint| texts.hint(hint))
                    .to_vec();
            }
            Some(AddForm::Folder { .. }) => {
                return [Hint::AddNow, Hint::Back]
                    .map(|hint| texts.hint(hint))
                    .to_vec();
            }
            None => {
                let login = setup.accounts.selected_instance().is_some_and(needs_login);
                vec![
                    Hint::Select,
                    Hint::Add,
                    if login { Hint::LogIn } else { Hint::Continue },
                ]
            }
        },
        StepId::Sharing if setup.sharing.asked().is_some() => {
            vec![Hint::Link, Hint::Private, Hint::Skip]
        }
        StepId::Sharing => vec![Hint::Continue],
        StepId::Drift => {
            let repair = setup
                .drift
                .selected_instance()
                .is_some_and(|instance| instance.drift_count > 0);
            vec![
                Hint::Select,
                if repair { Hint::Repair } else { Hint::Continue },
            ]
        }
        StepId::Launchers if setup.launchers.done() => vec![Hint::Continue],
        StepId::Launchers => vec![Hint::Fix],
        StepId::Preferences => vec![Hint::Select, Hint::Change, Hint::Continue],
        StepId::Summary => vec![Hint::Dashboard],
    };
    hints.extend([Hint::Steps, quit]);
    hints.into_iter().map(|hint| texts.hint(hint)).collect()
}

/// The question waiting for an answer, in a box over the screen: running the installer, or
/// repairing the selected instance.
fn draw_question(setup: &Setup, texts: Texts, frame: &mut Frame, area: Rect) {
    if setup.claude_code.confirm {
        let lines = vec![
            Line::from(texts.confirm_install()).bold(),
            Line::default(),
            command(CLAUDE_CODE_INSTALL_COMMAND),
            Line::default(),
            key_hints(&texts.install_keys()),
        ];
        widgets::question(frame, area, lines);
        return;
    }
    let Some(instance) = setup
        .drift
        .selected_instance()
        .filter(|_| setup.drift.confirm)
    else {
        return;
    };
    let mut lines = vec![
        Line::from(texts.confirm_repair(&setup.shown_path(&instance.config_dir))).bold(),
        Line::default(),
        Line::from(texts.repair_items(instance.items.len())),
    ];
    for item in instance.items.iter().take(ITEMS_SHOWN) {
        lines.push(Line::from(format!("  {}", item.id)));
    }
    if instance.items.len() > ITEMS_SHOWN {
        lines.push(Line::from(texts.more_items(instance.items.len() - ITEMS_SHOWN)).dim());
    }
    lines.push(Line::default());
    lines.push(key_hints(&texts.confirm_keys()));
    widgets::question(frame, area, lines);
}
