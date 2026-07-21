//! Interactive config TUI (ratatui). Calls shared config/credentials/mcp_install helpers.

use crate::config::{self, UserConfig};
use crate::credentials::{self, ResolveOpts};
use crate::mcp_install;
use anyhow::{bail, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::{self, stdout};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Edit,
    McpPick,
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditField {
    Edge,
    Token,
    Mcp,
    Slug,
}

pub fn run_tui(profile_override: Option<&str>) -> Result<()> {
    if !config::is_tty() {
        bail!("not a TTY; use: config list|show|set|use|env|mcp …");
    }

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let path = config::config_path()?;
    let mut cfg = UserConfig::load_or_default(&path)?;
    let mut screen = Screen::Home;
    let mut list_state = ListState::default();
    let names: Vec<String> = cfg.profiles.keys().cloned().collect();
    if !names.is_empty() {
        let idx = profile_override
            .and_then(|p| names.iter().position(|n| n == p))
            .or_else(|| {
                cfg.active_profile
                    .as_ref()
                    .and_then(|a| names.iter().position(|n| n == a))
            })
            .unwrap_or(0);
        list_state.select(Some(idx));
    }

    let mut status = String::from(
        "↑↓ select · e edit · n new · u use · d delete · m mcp · p print env · q quit",
    );
    let mut edit_name = String::new();
    let mut edit_field = EditField::Edge;
    let mut edge = String::new();
    let mut token = String::new();
    let mut mcp = String::new();
    let mut slug = String::new();
    let mut message = String::new();
    let mut mcp_idx = 0usize;
    let clients = ["droid", "claude", "cursor", "codex"];

    let result = (|| -> Result<()> {
        loop {
            let names: Vec<String> = cfg.profiles.keys().cloned().collect();
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(5),
                        Constraint::Length(3),
                    ])
                    .split(f.area());

                let title = Paragraph::new(Line::from(vec![
                    Span::styled(
                        " queria-cli config ",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!(" {}", path.display())),
                ]))
                .block(Block::default().borders(Borders::ALL).title("QuerIa"));
                f.render_widget(title, chunks[0]);

                match screen {
                    Screen::Home => {
                        let items: Vec<ListItem> = if names.is_empty() {
                            vec![ListItem::new("(no profiles — press n)")]
                        } else {
                            names
                                .iter()
                                .map(|n| {
                                    let star = if cfg.active_profile.as_deref() == Some(n.as_str())
                                    {
                                        "* "
                                    } else {
                                        "  "
                                    };
                                    ListItem::new(format!("{star}{n}"))
                                })
                                .collect()
                        };
                        let list = List::new(items)
                            .block(Block::default().borders(Borders::ALL).title("Profiles"))
                            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
                        f.render_stateful_widget(list, chunks[1], &mut list_state);
                    }
                    Screen::Edit => {
                        let lines = vec![
                            format!("profile: {edit_name}"),
                            format!(
                                "{} edge_url: {edge}",
                                if edit_field == EditField::Edge {
                                    ">"
                                } else {
                                    " "
                                }
                            ),
                            format!(
                                "{} token: {}",
                                if edit_field == EditField::Token {
                                    ">"
                                } else {
                                    " "
                                },
                                if token.is_empty() {
                                    String::new()
                                } else {
                                    config::redact_token(&token)
                                }
                            ),
                            format!(
                                "{} mcp_url: {mcp}",
                                if edit_field == EditField::Mcp {
                                    ">"
                                } else {
                                    " "
                                }
                            ),
                            format!(
                                "{} project_slug: {slug}",
                                if edit_field == EditField::Slug {
                                    ">"
                                } else {
                                    " "
                                }
                            ),
                            String::from(""),
                            String::from("Tab field · type · Enter save · Esc cancel"),
                        ];
                        let p = Paragraph::new(lines.join("\n"))
                            .block(Block::default().borders(Borders::ALL).title("Edit"))
                            .wrap(Wrap { trim: false });
                        f.render_widget(p, chunks[1]);
                    }
                    Screen::McpPick => {
                        let lines: Vec<String> = clients
                            .iter()
                            .enumerate()
                            .map(|(i, c)| {
                                format!(
                                    "{} {c}",
                                    if i == mcp_idx { ">" } else { " " }
                                )
                            })
                            .collect();
                        let p = Paragraph::new(lines.join("\n")).block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("MCP install — Enter confirm · Esc cancel"),
                        );
                        f.render_widget(p, chunks[1]);
                    }
                    Screen::Message => {
                        let p = Paragraph::new(message.as_str())
                            .block(Block::default().borders(Borders::ALL).title("Message"))
                            .wrap(Wrap { trim: true });
                        f.render_widget(p, chunks[1]);
                    }
                }

                let help = Paragraph::new(status.as_str())
                    .block(Block::default().borders(Borders::ALL).title("Status"));
                f.render_widget(help, chunks[2]);
            })?;

            if !event::poll(Duration::from_millis(200))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match screen {
                Screen::Home => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = list_state.selected().unwrap_or(0);
                        let n = cfg.profiles.len().max(1);
                        list_state.select(Some((i + 1) % n));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = list_state.selected().unwrap_or(0);
                        let n = cfg.profiles.len().max(1);
                        list_state.select(Some((i + n - 1) % n));
                    }
                    KeyCode::Char('n') => {
                        edit_name = "default".into();
                        edge.clear();
                        token.clear();
                        mcp.clear();
                        slug.clear();
                        edit_field = EditField::Edge;
                        screen = Screen::Edit;
                    }
                    KeyCode::Char('e') | KeyCode::Enter => {
                        let names: Vec<String> = cfg.profiles.keys().cloned().collect();
                        if let Some(i) = list_state.selected().filter(|i| *i < names.len()) {
                            edit_name = names[i].clone();
                            let p = cfg.profile(&edit_name).cloned().unwrap_or_default();
                            edge = p.edge_url.unwrap_or_default();
                            token = p.agent_token.unwrap_or_default();
                            mcp = p.mcp_url.unwrap_or_default();
                            slug = p.project_slug.unwrap_or_default();
                            edit_field = EditField::Edge;
                            screen = Screen::Edit;
                        }
                    }
                    KeyCode::Char('u') => {
                        let names: Vec<String> = cfg.profiles.keys().cloned().collect();
                        if let Some(i) = list_state.selected().filter(|i| *i < names.len()) {
                            cfg.active_profile = Some(names[i].clone());
                            cfg.save(&path)?;
                            status = format!("active_profile = {}", names[i]);
                        }
                    }
                    KeyCode::Char('d') => {
                        let names: Vec<String> = cfg.profiles.keys().cloned().collect();
                        if let Some(i) = list_state.selected().filter(|i| *i < names.len()) {
                            let n = names[i].clone();
                            cfg.profiles.remove(&n);
                            if cfg.active_profile.as_deref() == Some(n.as_str()) {
                                cfg.active_profile = cfg.profiles.keys().next().cloned();
                            }
                            cfg.save(&path)?;
                            list_state.select(if cfg.profiles.is_empty() {
                                None
                            } else {
                                Some(0)
                            });
                            status = format!("deleted {n}");
                        }
                    }
                    KeyCode::Char('m') => {
                        mcp_idx = 0;
                        screen = Screen::McpPick;
                    }
                    KeyCode::Char('p') => {
                        status = "use: queria-cli config env  (after quit) to print exports"
                            .into();
                    }
                    _ => {}
                },
                Screen::Edit => match key.code {
                    KeyCode::Esc => {
                        screen = Screen::Home;
                        status = "edit cancelled".into();
                    }
                    KeyCode::Tab | KeyCode::Down => {
                        edit_field = match edit_field {
                            EditField::Edge => EditField::Token,
                            EditField::Token => EditField::Mcp,
                            EditField::Mcp => EditField::Slug,
                            EditField::Slug => EditField::Edge,
                        };
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        edit_field = match edit_field {
                            EditField::Edge => EditField::Slug,
                            EditField::Token => EditField::Edge,
                            EditField::Mcp => EditField::Token,
                            EditField::Slug => EditField::Mcp,
                        };
                    }
                    KeyCode::Enter => {
                        if let Err(e) = config::validate_profile_name(&edit_name) {
                            status = e.to_string();
                            continue;
                        }
                        let p = cfg.profile_mut(&edit_name);
                        p.edge_url = nonempty(edge.clone());
                        p.agent_token = nonempty(token.clone());
                        p.mcp_url = nonempty(mcp.clone());
                        p.project_slug = nonempty(slug.clone());
                        if cfg.active_profile.is_none() {
                            cfg.active_profile = Some(edit_name.clone());
                        }
                        cfg.save(&path)?;
                        screen = Screen::Home;
                        status = format!("saved profile {edit_name}");
                        let names: Vec<String> = cfg.profiles.keys().cloned().collect();
                        if let Some(i) = names.iter().position(|n| n == &edit_name) {
                            list_state.select(Some(i));
                        }
                    }
                    KeyCode::Backspace => {
                        let t = active_edit_buf(
                            edit_field, &mut edge, &mut token, &mut mcp, &mut slug,
                        );
                        t.pop();
                    }
                    KeyCode::Char(c) => {
                        let t = active_edit_buf(
                            edit_field, &mut edge, &mut token, &mut mcp, &mut slug,
                        );
                        t.push(c);
                    }
                    _ => {}
                },
                Screen::McpPick => match key.code {
                    KeyCode::Esc => screen = Screen::Home,
                    KeyCode::Down | KeyCode::Char('j') => {
                        mcp_idx = (mcp_idx + 1) % clients.len();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        mcp_idx = (mcp_idx + clients.len() - 1) % clients.len();
                    }
                    KeyCode::Enter => {
                        let client = clients[mcp_idx];
                        // must leave raw mode for async + http, then show result
                        drop(terminal);
                        disable_raw_mode()?;
                        stdout().execute(LeaveAlternateScreen)?;
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()?;
                        let res = rt.block_on(async {
                            let creds = credentials::resolve(ResolveOpts {
                                profile: cfg.active_profile.clone(),
                                require_token: false,
                                ..Default::default()
                            })?;
                            mcp_install::install(&creds, client, false, true).await
                        });
                        message = match res {
                            Ok(()) => format!("MCP install ({client}) finished OK"),
                            Err(e) => format!("MCP install failed: {e:#}"),
                        };
                        enable_raw_mode()?;
                        stdout().execute(EnterAlternateScreen)?;
                        terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
                        screen = Screen::Message;
                    }
                    _ => {}
                },
                Screen::Message => match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                        screen = Screen::Home;
                    }
                    _ => {}
                },
            }
        }
    })();

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    result
}

fn nonempty(s: String) -> Option<String> {
    let s = s.trim().to_owned();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn active_edit_buf<'a>(
    field: EditField,
    edge: &'a mut String,
    token: &'a mut String,
    mcp: &'a mut String,
    slug: &'a mut String,
) -> &'a mut String {
    match field {
        EditField::Edge => edge,
        EditField::Token => token,
        EditField::Mcp => mcp,
        EditField::Slug => slug,
    }
}
