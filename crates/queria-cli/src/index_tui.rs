//! Index-here wizard TUI: discover → checklist → preflight → dry-run → upload → job_ids.

use crate::config;
use crate::credentials::{self, ResolveOpts};
use crate::edge_agent;
use crate::index_here::{
    self, DEFAULT_DEPTH, RootFilePlan, filter_plans_by_paths, upload_selected_plans,
};
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
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Checklist,
    Preflight,
    DryRun,
    Uploading,
    Result,
}

/// Launch index-here wizard (own alternate-screen session). Requires a TTY.
pub fn run_index_wizard(profile: Option<&str>) -> Result<()> {
    if !config::is_tty() {
        bail!("queria-cli index wizard needs a TTY");
    }

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            let _ = disable_raw_mode();
            let _ = stdout().execute(LeaveAlternateScreen);
            return Err(e.into());
        }
    };

    let result = (|| -> Result<()> {
        // Draw scanning frame before blocking discovery/plan work.
        terminal.draw(|f| {
            let chunks = layout_chunks(f.area());
            render_title(f, chunks[0], profile);
            let body = Paragraph::new("scanning git roots under cwd…")
                .block(Block::default().borders(Borders::ALL).title("Discover"));
            f.render_widget(body, chunks[1]);
            let help = Paragraph::new("please wait")
                .block(Block::default().borders(Borders::ALL).title("Status"));
            f.render_widget(help, chunks[2]);
        })?;

        let plans = discover_and_plan()?;
        if plans.is_empty() {
            show_message_then_wait(
                &mut terminal,
                profile,
                "No git roots found under cwd.",
                "Esc/Enter/q back",
            )?;
            return Ok(());
        }

        let mut selected = vec![true; plans.len()];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let mut screen = Screen::Checklist;
        let mut status = String::from("↑↓/jk move · Space toggle · Enter next · Esc cancel");
        let mut result_message = String::new();
        let mut preflight = PreflightState::Unknown;

        loop {
            terminal.draw(|f| {
                let chunks = layout_chunks(f.area());
                render_title(f, chunks[0], profile);

                match screen {
                    Screen::Checklist => {
                        let items: Vec<ListItem> = plans
                            .iter()
                            .enumerate()
                            .map(|(i, plan)| {
                                let mark = if selected.get(i).copied().unwrap_or(false) {
                                    "[x]"
                                } else {
                                    "[ ]"
                                };
                                ListItem::new(format!("{mark} {}", root_line(plan)))
                            })
                            .collect();
                        let list = List::new(items)
                            .block(
                                Block::default()
                                    .borders(Borders::ALL)
                                    .title("Select repos (Space toggle)"),
                            )
                            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
                        f.render_stateful_widget(list, chunks[1], &mut list_state);
                    }
                    Screen::Preflight => {
                        let body = Paragraph::new(preflight_text(&preflight))
                            .block(Block::default().borders(Borders::ALL).title("Preflight"))
                            .wrap(Wrap { trim: false });
                        f.render_widget(body, chunks[1]);
                    }
                    Screen::DryRun => {
                        let chosen = selected_plans(&plans, &selected);
                        let body = Paragraph::new(dry_run_text(&chosen))
                            .block(
                                Block::default()
                                    .borders(Borders::ALL)
                                    .title("Dry-run summary"),
                            )
                            .wrap(Wrap { trim: false });
                        f.render_widget(body, chunks[1]);
                    }
                    Screen::Uploading => {
                        let body = Paragraph::new("uploading selected roots…")
                            .block(Block::default().borders(Borders::ALL).title("Upload"));
                        f.render_widget(body, chunks[1]);
                    }
                    Screen::Result => {
                        let body = Paragraph::new(result_message.as_str())
                            .block(Block::default().borders(Borders::ALL).title("Result"))
                            .wrap(Wrap { trim: false });
                        f.render_widget(body, chunks[1]);
                    }
                }

                let help = Paragraph::new(status.as_str())
                    .block(Block::default().borders(Borders::ALL).title("Status"))
                    .wrap(Wrap { trim: true });
                f.render_widget(help, chunks[2]);
            })?;

            // Upload runs after the Uploading screen is painted once.
            if screen == Screen::Uploading {
                let chosen = selected_plans(&plans, &selected);
                result_message = match do_upload(profile, &chosen) {
                    Ok(job_ids) => format_success(&job_ids),
                    Err(e) => format!("Upload failed:\n{e:#}"),
                };
                screen = Screen::Result;
                status = "Enter/Esc/q back".into();
                continue;
            }

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
                Screen::Checklist => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => break Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = list_state.selected().unwrap_or(0);
                        list_state.select(Some((i + 1) % plans.len()));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = list_state.selected().unwrap_or(0);
                        list_state.select(Some((i + plans.len() - 1) % plans.len()));
                    }
                    KeyCode::Char(' ') => {
                        if let Some(i) = list_state.selected() {
                            toggle_selected(&mut selected, i);
                        }
                    }
                    KeyCode::Enter => {
                        if !selected.iter().any(|s| *s) {
                            status = "select at least one repo (Space)".into();
                            continue;
                        }
                        preflight = check_index_local_preflight(profile);
                        screen = Screen::Preflight;
                        status = if preflight.blocks_upload() {
                            "Esc/q back (upload blocked)".into()
                        } else {
                            "Enter continue · Esc cancel".into()
                        };
                    }
                    _ => {}
                },
                Screen::Preflight => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        screen = Screen::Checklist;
                        status = "↑↓/jk move · Space toggle · Enter next · Esc cancel".into();
                    }
                    KeyCode::Enter => {
                        if preflight.blocks_upload() {
                            status =
                                "blocked: need Custom token with index_local · Esc back".into();
                            continue;
                        }
                        screen = Screen::DryRun;
                        status = "u or Enter upload · Esc back".into();
                    }
                    _ => {}
                },
                Screen::DryRun => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        screen = Screen::Preflight;
                        status = if preflight.blocks_upload() {
                            "Esc/q back (upload blocked)".into()
                        } else {
                            "Enter continue · Esc cancel".into()
                        };
                    }
                    KeyCode::Enter | KeyCode::Char('u') => {
                        if preflight.blocks_upload() {
                            // Should be unreachable if Preflight gate works; re-check.
                            screen = Screen::Preflight;
                            status =
                                "blocked: need Custom token with index_local · Esc back".into();
                            continue;
                        }
                        screen = Screen::Uploading;
                        status = "uploading…".into();
                    }
                    _ => {}
                },
                Screen::Uploading => {
                    // Keys ignored while upload is in progress.
                }
                Screen::Result => match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => break Ok(()),
                    _ => {}
                },
            }
        }
    })();

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    result
}

fn discover_and_plan() -> Result<Vec<RootFilePlan>> {
    let cwd = std::env::current_dir()?;
    let roots = index_here::discover_git_roots(&cwd, DEFAULT_DEPTH)?;
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    let allow_owned = {
        let path = config::config_path().ok();
        let cfg = path
            .as_ref()
            .and_then(|p| config::UserConfig::load_or_default(p).ok())
            .unwrap_or_default();
        config::effective_index_extensions(&cfg)
    };
    let allow_refs: Vec<&str> = allow_owned.iter().map(String::as_str).collect();
    let all_paths: Vec<PathBuf> = roots.iter().map(|r| r.path.clone()).collect();
    roots
        .into_iter()
        .map(|root| index_here::plan_root_files_with_extensions(root, &all_paths, &allow_refs))
        .collect()
}

fn do_upload(profile: Option<&str>, plans: &[RootFilePlan]) -> Result<Vec<String>> {
    if plans.is_empty() {
        bail!("no roots selected");
    }
    let total_accept: usize = plans.iter().map(|p| p.accepted.len()).sum();
    if total_accept == 0 {
        return Ok(Vec::new());
    }

    let creds = credentials::resolve(ResolveOpts {
        profile: profile.map(|s| s.to_owned()),
        require_token: true,
        ..Default::default()
    })?;
    let token = creds
        .agent_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing agent token after resolve"))?;
    let endpoint = format!(
        "{}/api/v1/agent/index-local",
        creds.edge_url.trim_end_matches('/')
    );

    // Quiet: suppress stderr progress under alt-screen so TUI is not corrupted.
    block_on_compat(async { upload_selected_plans(&endpoint, &token, plans, true).await })
}

fn format_success(job_ids: &[String]) -> String {
    if job_ids.is_empty() {
        return String::from(
            "No accepted files to upload (nothing sent).\n\
             Admin → Needs review → Promote",
        );
    }
    format!(
        "job_ids:\n  {}\n\nAdmin → Needs review → Promote",
        job_ids.join("\n  ")
    )
}

/// Outcome of IndexLocal permission preflight via projects-status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightState {
    /// Status not checked yet (or credentials incomplete). Soft Daily warn; allow attempt.
    Unknown,
    /// Token has `index_local`.
    Allowed,
    /// Permissions list present and missing `index_local` — hard-block upload.
    BlockedMissingIndexLocal,
    /// projects-status 404 (old edge) — soft Daily warn; allow attempt (403 still handled).
    StatusNotFound,
    /// Fetch failed for a non-404 reason — soft warn; allow attempt.
    FetchFailed(String),
}

impl PreflightState {
    pub fn blocks_upload(&self) -> bool {
        matches!(self, PreflightState::BlockedMissingIndexLocal)
    }
}

/// Fetch projects-status and classify IndexLocal readiness for the wizard preflight.
pub fn check_index_local_preflight(profile: Option<&str>) -> PreflightState {
    let Ok(creds) = credentials::resolve(ResolveOpts {
        profile: profile.map(|s| s.to_owned()),
        require_token: false,
        ..Default::default()
    }) else {
        return PreflightState::Unknown;
    };
    let Some(token) = creds.agent_token.as_deref().filter(|t| !t.is_empty()) else {
        return PreflightState::Unknown;
    };
    match block_on_compat(edge_agent::fetch_projects_status(&creds.edge_url, token)) {
        Ok((_status, body)) => {
            if body.permissions.iter().any(|p| p == "index_local") {
                PreflightState::Allowed
            } else {
                PreflightState::BlockedMissingIndexLocal
            }
        }
        Err(e) if edge_agent::is_projects_status_404(&e) => PreflightState::StatusNotFound,
        Err(e) => PreflightState::FetchFailed(e.to_string()),
    }
}

fn preflight_text(state: &PreflightState) -> String {
    // Always surface Daily vs Custom IndexLocal copy first.
    let header = "IndexLocal permission preflight\n\
                  \n\
                  Daily agent tokens cannot upload local repos.\n\
                  Use a Custom token with index_local (Admin → Tokens).\n";
    let tail = match state {
        PreflightState::Allowed => {
            "\n\
             Permissions check: PASS — token has index_local.\n\
             \n\
             Press Enter to continue · Esc cancel"
        }
        PreflightState::BlockedMissingIndexLocal => {
            "\n\
             Permissions check: BLOCKED — token lacks index_local.\n\
             Mint a Custom agent token with index_local (Admin → Tokens).\n\
             Upload is not allowed with this token.\n\
             \n\
             Esc/q back"
        }
        PreflightState::StatusNotFound => {
            "\n\
             Permissions API unavailable (HTTP 404 — older edge).\n\
             Soft warn only — upload will still attempt; 403 means mint Custom index_local.\n\
             Redeploy edge/API for live permission checks.\n\
             \n\
             Press Enter to continue · Esc cancel"
        }
        PreflightState::FetchFailed(detail) => {
            return format!(
                "{header}\n\
                 Permissions check skipped ({detail}).\n\
                 Soft warn only — upload will still attempt; 403 means mint Custom index_local.\n\
                 \n\
                 Press Enter to continue · Esc cancel"
            );
        }
        PreflightState::Unknown => {
            "\n\
             (Permissions not checked — no token/credentials.)\n\
             Upload will hard-fail on 403 if the token lacks index_local.\n\
             \n\
             Press Enter to continue · Esc cancel"
        }
    };
    format!("{header}{tail}")
}

fn dry_run_text(plans: &[RootFilePlan]) -> String {
    let (n_repos, n_accept, n_skip) = summary_totals(plans);
    let mut lines = vec![format!(
        "Selected {n_repos} repo(s) · will index {n_accept} file(s) · skip {n_skip}"
    )];
    for plan in plans {
        lines.push(format!("  • {}", root_line(plan)));
    }
    lines.push(String::new());
    lines.push(String::from("Press u or Enter to upload · Esc back"));
    lines.join("\n")
}

fn root_line(plan: &RootFilePlan) -> String {
    let name = plan
        .root
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(".");
    let branch = plan.root.branch.as_deref().unwrap_or("?");
    format!(
        "{name} ({branch}) +{} −{}",
        plan.accepted.len(),
        plan.skipped
    )
}

/// Toggle selection at `idx` (no-op if out of range). Pure helper for tests + TUI.
pub fn toggle_selected(selected: &mut [bool], idx: usize) {
    if let Some(s) = selected.get_mut(idx) {
        *s = !*s;
    }
}

/// Paths of selected plans (same order as `plans`).
pub fn selected_paths(plans: &[RootFilePlan], selected: &[bool]) -> Vec<PathBuf> {
    plans
        .iter()
        .zip(selected.iter())
        .filter(|(_, s)| **s)
        .map(|(p, _)| p.root.path.clone())
        .collect()
}

/// Filter plans by the bool selection mask.
pub fn selected_plans(plans: &[RootFilePlan], selected: &[bool]) -> Vec<RootFilePlan> {
    let paths = selected_paths(plans, selected);
    filter_plans_by_paths(plans, &paths)
}

/// (repo_count, accepted_files, skipped_files) for a plan slice.
pub fn summary_totals(plans: &[RootFilePlan]) -> (usize, usize, u32) {
    let n_repos = plans.len();
    let n_accept: usize = plans.iter().map(|p| p.accepted.len()).sum();
    let n_skip: u32 = plans.iter().map(|p| p.skipped).sum();
    (n_repos, n_accept, n_skip)
}

fn show_message_then_wait<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    profile: Option<&str>,
    message: &str,
    status: &str,
) -> Result<()> {
    loop {
        terminal.draw(|f| {
            let chunks = layout_chunks(f.area());
            render_title(f, chunks[0], profile);
            let body = Paragraph::new(message)
                .block(Block::default().borders(Borders::ALL).title("Index"))
                .wrap(Wrap { trim: true });
            f.render_widget(body, chunks[1]);
            let help = Paragraph::new(status)
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
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => break,
            _ => {}
        }
    }
    Ok(())
}

fn layout_chunks(area: ratatui::layout::Rect) -> std::rc::Rc<[ratatui::layout::Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area)
}

fn render_title(f: &mut ratatui::Frame<'_>, area: ratatui::layout::Rect, profile: Option<&str>) {
    let profile_label = profile.unwrap_or("(active/default)");
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " index-here wizard ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" profile={profile_label}")),
    ]))
    .block(Block::default().borders(Borders::ALL).title("QuerIa"));
    f.render_widget(title, area);
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
                .expect("tokio runtime for index TUI");
            rt.block_on(fut)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_here::{DiscoveredRoot, IndexableFile};
    use queria_ingestion::local_index_gates::content_hash;

    fn sample_plan(path: &str, accept: usize, skip: u32) -> RootFilePlan {
        let accepted = (0..accept)
            .map(|i| IndexableFile {
                path: format!("f{i}.md"),
                content: "x".into(),
                content_hash: content_hash("x"),
            })
            .collect();
        RootFilePlan {
            root: DiscoveredRoot {
                path: PathBuf::from(path),
                origin_url: None,
                commit_sha: Some("abc".into()),
                branch: Some("main".into()),
            },
            accepted,
            skipped: skip,
        }
    }

    #[test]
    fn toggle_selected_flips_in_range() {
        let mut sel = vec![true, false, true];
        toggle_selected(&mut sel, 1);
        assert_eq!(sel, vec![true, true, true]);
        toggle_selected(&mut sel, 0);
        assert_eq!(sel, vec![false, true, true]);
        toggle_selected(&mut sel, 99); // no-op
        assert_eq!(sel, vec![false, true, true]);
    }

    #[test]
    fn selected_plans_respects_mask() {
        let plans = vec![sample_plan("/tmp/a", 2, 1), sample_plan("/tmp/b", 1, 0)];
        let selected = vec![false, true];
        let chosen = selected_plans(&plans, &selected);
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].root.path, PathBuf::from("/tmp/b"));
        let (n, a, s) = summary_totals(&chosen);
        assert_eq!((n, a, s), (1, 1, 0));
    }

    #[test]
    fn format_success_lists_job_ids() {
        let msg = format_success(&["job-1".into(), "job-2".into()]);
        assert!(msg.contains("job-1"));
        assert!(msg.contains("job-2"));
        assert!(msg.contains("Admin → Needs review → Promote"));
    }

    #[test]
    fn preflight_blocked_copy_mentions_custom_token() {
        let text = preflight_text(&PreflightState::BlockedMissingIndexLocal);
        assert!(text.contains("BLOCKED"));
        assert!(text.contains("index_local"));
        assert!(text.contains("Custom"));
        assert!(PreflightState::BlockedMissingIndexLocal.blocks_upload());
        assert!(!PreflightState::Allowed.blocks_upload());
        assert!(!PreflightState::StatusNotFound.blocks_upload());
        assert!(!PreflightState::Unknown.blocks_upload());
    }

    #[test]
    fn preflight_404_allows_attempt() {
        let text = preflight_text(&PreflightState::StatusNotFound);
        assert!(text.contains("404"));
        assert!(text.contains("continue"));
        assert!(!PreflightState::StatusNotFound.blocks_upload());
    }
}
