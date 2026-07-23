//! Status TUI: agent projects-status (embed + needs_review) without SETUP_TOKEN.

use crate::credentials;
use crate::edge_agent::{self, ProjectsStatusResponse};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use std::time::Duration;

/// Load credentials and fetch projects-status for the Status screen.
/// Credential/resolve/missing-token failures soft-degrade to `StatusView::Error`
/// (same as network/404) so the hub TUI session is never ejected.
async fn load_status(profile: Option<&str>) -> StatusView {
    let creds = match credentials::resolve(credentials::ResolveOpts {
        profile: profile.map(|s| s.to_owned()),
        require_token: false,
        ..Default::default()
    }) {
        Ok(c) => c,
        Err(e) => {
            return StatusView::Error {
                edge_url: credentials::DEFAULT_EDGE_URL.to_owned(),
                profile: profile.map(|s| s.to_owned()),
                detail: format!("credentials resolve failed: {e:#}"),
            };
        }
    };
    let Some(token) = creds.agent_token.as_deref().filter(|t| !t.is_empty()) else {
        return StatusView::Error {
            edge_url: creds.edge_url,
            profile: creds.profile,
            detail: "no agent token — open Config".into(),
        };
    };
    match edge_agent::fetch_projects_status(&creds.edge_url, token).await {
        Ok((_status, body)) => StatusView::Ok {
            edge_url: creds.edge_url,
            profile: creds.profile,
            body,
        },
        Err(e) if edge_agent::is_projects_status_404(&e) => StatusView::NotFound {
            edge_url: creds.edge_url,
            profile: creds.profile,
            detail: e.to_string(),
        },
        Err(e) => StatusView::Error {
            edge_url: creds.edge_url,
            profile: creds.profile,
            detail: e.to_string(),
        },
    }
}

#[derive(Debug)]
enum StatusView {
    Ok {
        edge_url: String,
        profile: Option<String>,
        body: ProjectsStatusResponse,
    },
    NotFound {
        edge_url: String,
        profile: Option<String>,
        detail: String,
    },
    Error {
        edge_url: String,
        profile: Option<String>,
        detail: String,
    },
}

impl StatusView {
    fn edge_url(&self) -> &str {
        match self {
            StatusView::Ok { edge_url, .. }
            | StatusView::NotFound { edge_url, .. }
            | StatusView::Error { edge_url, .. } => edge_url,
        }
    }

    fn profile(&self) -> Option<&str> {
        match self {
            StatusView::Ok { profile, .. }
            | StatusView::NotFound { profile, .. }
            | StatusView::Error { profile, .. } => profile.as_deref(),
        }
    }
}

/// Run status screen inside an existing alternate-screen terminal session.
/// Keys: `r` refresh, `Esc`/`q` return.
pub fn run<B: Backend>(terminal: &mut Terminal<B>, profile: Option<&str>) -> Result<()> {
    // Soft-degrade: credential errors become StatusView::Error, never eject hub.
    let mut view = block_on_compat(load_status(profile));
    let mut status = String::from("r refresh · Esc/q back");

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

            let profile_label = view.profile().unwrap_or("(none)");
            let title = Paragraph::new(Line::from(vec![
                Span::styled(" status ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(
                    " profile={profile_label} · edge={}",
                    view.edge_url()
                )),
            ]))
            .block(Block::default().borders(Borders::ALL).title("QuerIa"));
            f.render_widget(title, chunks[0]);

            match &view {
                StatusView::Ok { body, .. } => {
                    let mut items: Vec<ListItem> = Vec::new();
                    items.push(ListItem::new(format!(
                        "embed_profile={} · perms={}",
                        body.embedding_profile_version,
                        if body.permissions.is_empty() {
                            "(none)".to_string()
                        } else {
                            body.permissions.join(",")
                        }
                    )));
                    if body.projects.is_empty() {
                        items.push(ListItem::new("(no projects in token scope)"));
                    } else {
                        for row in &body.projects {
                            items.push(ListItem::new(format!(
                                "{}  {}/{}/{}  NR={}",
                                row.slug,
                                row.embed.ready,
                                row.embed.pending,
                                row.embed.failed,
                                row.needs_review_count
                            )));
                        }
                    }
                    let list = List::new(items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Projects (slug ready/pending/failed NR)"),
                    );
                    f.render_widget(list, chunks[1]);
                }
                StatusView::NotFound { detail, .. } => {
                    let msg = format!(
                        "projects-status not found (HTTP 404).\n\
                         \n\
                         This edge is older than the Status API — redeploy edge/API\n\
                         to pick up GET /api/v1/agent/projects-status.\n\
                         Doctor still works on this build.\n\
                         \n\
                         ({detail})"
                    );
                    let p = Paragraph::new(msg)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("Status (degraded)"),
                        )
                        .wrap(Wrap { trim: false });
                    f.render_widget(p, chunks[1]);
                }
                StatusView::Error { detail, .. } => {
                    let p = Paragraph::new(format!("Could not load status:\n{detail}"))
                        .block(Block::default().borders(Borders::ALL).title("Status error"))
                        .wrap(Wrap { trim: true });
                    f.render_widget(p, chunks[1]);
                }
            }

            let help = Paragraph::new(status.as_str())
                .block(Block::default().borders(Borders::ALL).title("Keys"))
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
                view = block_on_compat(load_status(profile));
                status = "refreshed".into();
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
                .expect("tokio runtime for status TUI");
            rt.block_on(fut)
        }
    }
}
