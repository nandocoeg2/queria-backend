//! Doctor TUI screen: display and re-run doctor checklist snapshot.

use crate::checks::{self, CheckLevel};
use crate::credentials;
use crate::edge_agent;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use std::time::Duration;

/// Fetch credentials + edge health + MCP tools/list, assemble pure snapshot.
pub async fn collect_doctor_snapshot(
    profile: Option<&str>,
) -> anyhow::Result<checks::DoctorSnapshot> {
    let creds = credentials::resolve(credentials::ResolveOpts {
        profile: profile.map(|s| s.to_owned()),
        require_token: false,
        ..Default::default()
    })?;
    let health = edge_agent::edge_health(&creds.edge_url)
        .await
        .map_err(|e| e.to_string());
    let mcp = match creds.agent_token.as_deref() {
        Some(t) if !t.is_empty() => edge_agent::mcp_tools_list(&creds.mcp_url, t)
            .await
            .map_err(|e| e.to_string()),
        _ => Err("no token".into()),
    };
    Ok(checks::assemble_doctor_snapshot(
        env!("CARGO_PKG_VERSION"),
        creds.profile.as_deref(),
        &creds.edge_url,
        &creds.mcp_url,
        creds.agent_token.as_deref(),
        health,
        mcp,
        None, // permissions from status API: P2
    ))
}

/// Run doctor screen inside an existing alternate-screen terminal session.
/// Keys: `r` re-run checks, `Esc`/`q` return.
pub fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    profile: Option<&str>,
) -> Result<()> {
    let mut snapshot = block_on_compat(collect_doctor_snapshot(profile))?;
    let mut status = String::from("r re-run · Esc/q back");

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

            let profile_label = snapshot
                .profile
                .as_deref()
                .unwrap_or("(none)");
            let title = Paragraph::new(Line::from(vec![
                Span::styled(
                    " doctor ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    " v{} · profile={profile_label} · edge={} · mcp={}",
                    snapshot.version, snapshot.edge_url, snapshot.mcp_url
                )),
            ]))
            .block(Block::default().borders(Borders::ALL).title("QuerIa"));
            f.render_widget(title, chunks[0]);

            let items: Vec<ListItem> = snapshot
                .items
                .iter()
                .map(|item| {
                    let level = match item.level {
                        CheckLevel::Pass => "PASS",
                        CheckLevel::Warn => "WARN",
                        CheckLevel::Fail => "FAIL",
                    };
                    let mut lines = vec![Line::from(format!(
                        "[{level}] {} — {}",
                        item.id, item.detail
                    ))];
                    if !item.hint.is_empty() {
                        lines.push(Line::from(format!("       hint: {}", item.hint)));
                    }
                    ListItem::new(lines)
                })
                .collect();
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Checks"));
            f.render_widget(list, chunks[1]);

            let help = Paragraph::new(status.as_str())
                .block(Block::default().borders(Borders::ALL).title("Status"))
                .wrap(Wrap { trim: true });
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
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => break,
            KeyCode::Char('r') => {
                match block_on_compat(collect_doctor_snapshot(profile)) {
                    Ok(s) => {
                        snapshot = s;
                        status = "re-ran checks".into();
                    }
                    Err(e) => {
                        status = format!("re-run failed: {e:#}");
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Run an async future from the sync TUI without nesting a second Tokio runtime.
fn block_on_compat<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime for doctor TUI");
            rt.block_on(fut)
        }
    }
}
