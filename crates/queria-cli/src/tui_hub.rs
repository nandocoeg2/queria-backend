//! Interactive hub TUI: doctor / index / status / config (TTY required).

use crate::config;
use crate::config_tui;
use crate::doctor_tui;
use anyhow::{Result, bail};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use std::io::stdout;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Menu,
    Message,
}

const MENU: &[&str] = &["Doctor", "Index", "Status", "Config", "Quit"];

/// Launch hub TUI. Requires a TTY.
pub fn run_hub(profile: Option<&str>) -> Result<()> {
    if !config::is_tty() {
        bail!("queria-cli tui needs a TTY");
    }

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut screen = Screen::Menu;
    let mut list_state = ListState::default();
    list_state.select(Some(0));
    let mut status =
        String::from("↑↓/jk select · Enter open · d/i/s/c shortcuts · q quit");
    let mut message = String::new();

    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(5),
                        Constraint::Length(3),
                    ])
                    .split(f.area());

                let profile_label = profile.unwrap_or("(active/default)");
                let title = Paragraph::new(Line::from(vec![
                    Span::styled(
                        " queria-cli hub ",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!(" profile={profile_label}")),
                ]))
                .block(Block::default().borders(Borders::ALL).title("QuerIa"));
                f.render_widget(title, chunks[0]);

                match screen {
                    Screen::Menu => {
                        let items: Vec<ListItem> = MENU
                            .iter()
                            .enumerate()
                            .map(|(i, label)| {
                                let key = match i {
                                    0 => "d",
                                    1 => "i",
                                    2 => "s",
                                    3 => "c",
                                    _ => "q",
                                };
                                ListItem::new(format!("[{key}] {label}"))
                            })
                            .collect();
                        let list = List::new(items)
                            .block(Block::default().borders(Borders::ALL).title("Menu"))
                            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
                        f.render_stateful_widget(list, chunks[1], &mut list_state);
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
                Screen::Menu => {
                    let action = match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => Some(4),
                        KeyCode::Down | KeyCode::Char('j') => {
                            let i = list_state.selected().unwrap_or(0);
                            list_state.select(Some((i + 1) % MENU.len()));
                            None
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            let i = list_state.selected().unwrap_or(0);
                            list_state.select(Some((i + MENU.len() - 1) % MENU.len()));
                            None
                        }
                        KeyCode::Char('d') => {
                            list_state.select(Some(0));
                            Some(0)
                        }
                        KeyCode::Char('i') => {
                            list_state.select(Some(1));
                            Some(1)
                        }
                        KeyCode::Char('s') => {
                            list_state.select(Some(2));
                            Some(2)
                        }
                        KeyCode::Char('c') => {
                            list_state.select(Some(3));
                            Some(3)
                        }
                        KeyCode::Enter => Some(list_state.selected().unwrap_or(0)),
                        _ => None,
                    };

                    match action {
                        Some(0) => {
                            doctor_tui::run(&mut terminal, profile)?;
                            status = "returned from doctor".into();
                        }
                        Some(1) => {
                            message = "Index wizard ships in P1".into();
                            screen = Screen::Message;
                            status = "Enter/Esc/q back".into();
                        }
                        Some(2) => {
                            message = "Status ships in P2".into();
                            screen = Screen::Message;
                            status = "Enter/Esc/q back".into();
                        }
                        Some(3) => {
                            // Config owns its own alternate screen — leave hub first.
                            drop(terminal);
                            disable_raw_mode()?;
                            stdout().execute(LeaveAlternateScreen)?;
                            let cfg_res = config_tui::run_tui(profile);
                            enable_raw_mode()?;
                            stdout().execute(EnterAlternateScreen)?;
                            terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
                            status = match cfg_res {
                                Ok(()) => "returned from config".into(),
                                Err(e) => format!("config error: {e:#}"),
                            };
                        }
                        Some(4) | Some(_) => break Ok(()),
                        None => {}
                    }
                }
                Screen::Message => match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                        screen = Screen::Menu;
                        status =
                            "↑↓/jk select · Enter open · d/i/s/c shortcuts · q quit".into();
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
