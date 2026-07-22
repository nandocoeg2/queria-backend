mod backup;
mod bootstrap;
mod checks;
mod config;
mod config_tui;
mod credentials;
mod database;
mod doctor_mcp;
mod doctor_tui;
mod edge_agent;
mod embeddings;
mod evaluation;
mod index_here;
mod mcp_install;
mod restore_drill;
mod retrieval;
mod tui_hub;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "queria-cli")]
#[command(about = "Queria operational CLI")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Profile from ~/.config/queria/config.toml (overrides active_profile).
    #[arg(long, global = true, env = "QUERIA_PROFILE")]
    profile: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Doctor {
        #[command(subcommand)]
        command: DoctorCommand,
    },
    Bootstrap,
    Database {
        #[command(subcommand)]
        command: DatabaseCommand,
    },
    Embeddings {
        #[command(subcommand)]
        command: EmbeddingsCommand,
    },
    Retrieval {
        #[command(subcommand)]
        command: RetrievalCommand,
    },
    Eval {
        #[command(subcommand)]
        command: EvalCommand,
    },
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },
    /// Interactive TUI: profiles / token / edge / MCP install (~/.config/queria/config.toml).
    Config,
    /// Interactive hub TUI: doctor / index / status / config (TTY required).
    Tui,
    /// Discover local git roots under cwd and upload tracked files for needs_review indexing.
    IndexHere {
        /// Env var name holding the raw agent token (never print the token).
        #[arg(long, default_value = "QUERIA_AGENT_TOKEN")]
        token_env: String,
        /// Env var name for edge base URL (default env QUERIA_EDGE_URL → http://127.0.0.1:17674).
        #[arg(long, default_value = "QUERIA_EDGE_URL")]
        edge_url_env: String,
        /// Nested git scan depth under cwd.
        #[arg(long, default_value_t = index_here::DEFAULT_DEPTH)]
        depth: u32,
        /// Non-interactive: required when multiple git roots are discovered.
        #[arg(long)]
        yes: bool,
        /// Discover + gate counts only; no HTTP upload.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
enum DoctorCommand {
    /// Probe MCP tools/list. Default URL: config/env mcp_url (or {edge}/mcp), not localhost hardcode.
    Mcp {
        /// Override MCP endpoint. When omitted, resolve from profile/env.
        #[arg(long)]
        url: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum DatabaseCommand {
    Migrate,
}

#[derive(Debug, Subcommand)]
enum EmbeddingsCommand {
    Backfill {
        #[arg(long)]
        project: String,
    },
    Status {
        #[arg(long)]
        project: String,
    },
}

#[derive(Debug, Subcommand)]
enum RetrievalCommand {
    Probe {
        #[arg(long)]
        project: String,
        #[arg(long)]
        query: String,
        #[arg(long, default_value_t = true)]
        include_global: bool,
        #[arg(long, default_value_t = 5)]
        limit: u32,
        /// When set, overrides server QUERIA_RERANK_ENABLED default.
        #[arg(long)]
        rerank: Option<bool>,
        /// When set, overrides server QUERIA_COMPRESS_ENABLED default.
        #[arg(long)]
        compress: Option<bool>,
    },
}

#[derive(Debug, Subcommand)]
enum EvalCommand {
    Run {
        #[arg(long)]
        project: String,
    },
}

#[derive(Debug, Subcommand)]
enum BackupCommand {
    RestoreDrill {
        #[arg(long)]
        org: String,
        #[arg(long)]
        target_database_url: Option<String>,
        #[arg(long)]
        target_qdrant_url: Option<String>,
        #[arg(long)]
        target_qdrant_collection: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let profile = cli.profile.clone();

    match cli.command {
        Command::Doctor {
            command: DoctorCommand::Mcp { url },
        } => {
            let creds = credentials::resolve(credentials::ResolveOpts {
                profile: profile.clone(),
                require_token: true,
                ..Default::default()
            })?;
            let url = match url {
                Some(u) if !u.trim().is_empty() => u,
                _ => creds.mcp_url.clone(),
            };
            let token = creds
                .agent_token
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("agent token required for doctor mcp"))?;
            doctor_mcp::run(&url, token).await
        }
        Command::Bootstrap => bootstrap::run().await,
        Command::Database {
            command: DatabaseCommand::Migrate,
        } => database::migrate().await,
        Command::Embeddings {
            command: EmbeddingsCommand::Backfill { project },
        } => embeddings::backfill(&project).await,
        Command::Embeddings {
            command: EmbeddingsCommand::Status { project },
        } => embeddings::status(&project).await,
        Command::Retrieval {
            command:
                RetrievalCommand::Probe {
                    project,
                    query,
                    include_global,
                    limit,
                    rerank,
                    compress,
                },
        } => retrieval::probe(&project, &query, include_global, limit, rerank, compress).await,
        Command::Eval {
            command: EvalCommand::Run { project },
        } => evaluation::run(&project).await,
        Command::Backup {
            command:
                BackupCommand::RestoreDrill {
                    org,
                    target_database_url,
                    target_qdrant_url,
                    target_qdrant_collection,
                },
        } => {
            backup::restore_drill(
                &org,
                target_database_url,
                target_qdrant_url,
                target_qdrant_collection,
            )
            .await
        }
        Command::Config => {
            config_tui::run_tui(profile.as_deref())?;
            Ok(())
        }
        Command::Tui => {
            tui_hub::run_hub(profile.as_deref())?;
            Ok(())
        }
        Command::IndexHere {
            token_env,
            edge_url_env,
            depth,
            yes,
            dry_run,
        } => {
            index_here::run(
                &token_env,
                &edge_url_env,
                depth,
                yes,
                dry_run,
                profile.as_deref(),
            )
            .await
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_embedding_backfill_command() {
        let cli = Cli::try_parse_from([
            "queria-cli",
            "embeddings",
            "backfill",
            "--project",
            "fjulian-me",
        ])
        .expect("embedding command should parse");

        assert!(matches!(
            cli.command,
            Command::Embeddings {
                command: EmbeddingsCommand::Backfill { project }
            } if project == "fjulian-me"
        ));
    }

    #[test]
    fn parses_eval_run_command() {
        let cli = Cli::try_parse_from(["queria-cli", "eval", "run", "--project", "fjulian-me"])
            .expect("eval command should parse");

        assert!(matches!(
            cli.command,
            Command::Eval {
                command: EvalCommand::Run { project }
            } if project == "fjulian-me"
        ));
    }

    #[test]
    fn parses_backup_restore_drill_command() {
        let cli = Cli::try_parse_from([
            "queria-cli",
            "backup",
            "restore-drill",
            "--org",
            "fjulian",
            "--target-database-url",
            "postgres://localhost/queria_restore",
            "--target-qdrant-url",
            "http://localhost:6333",
            "--target-qdrant-collection",
            "queria_restore",
        ])
        .expect("restore drill command should parse");

        assert!(matches!(
            cli.command,
            Command::Backup {
                command: BackupCommand::RestoreDrill { org, .. }
            } if org == "fjulian"
        ));
    }

    /// VAL-CROSS-001: probe flags optional; omit → None (server defaults).
    #[test]
    fn parses_retrieval_probe_without_quality_flags() {
        let cli = Cli::try_parse_from([
            "queria-cli",
            "retrieval",
            "probe",
            "--project",
            "fjulian-me",
            "--query",
            "hello",
        ])
        .expect("probe should parse");
        match cli.command {
            Command::Retrieval {
                command:
                    RetrievalCommand::Probe {
                        project,
                        query,
                        include_global,
                        limit,
                        rerank,
                        compress,
                    },
            } => {
                assert_eq!(project, "fjulian-me");
                assert_eq!(query, "hello");
                assert!(include_global);
                assert_eq!(limit, 5);
                assert!(rerank.is_none());
                assert!(compress.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    /// VAL-CROSS-002: CLI accepts explicit rerank/compress overrides.
    #[test]
    fn parses_retrieval_probe_with_quality_flag_overrides() {
        let cli = Cli::try_parse_from([
            "queria-cli",
            "retrieval",
            "probe",
            "--project",
            "fjulian-me",
            "--query",
            "hello",
            "--rerank=false",
            "--compress=true",
            "--limit",
            "3",
        ])
        .expect("probe with flags should parse");
        match cli.command {
            Command::Retrieval {
                command:
                    RetrievalCommand::Probe {
                        rerank,
                        compress,
                        limit,
                        ..
                    },
            } => {
                assert_eq!(rerank, Some(false));
                assert_eq!(compress, Some(true));
                assert_eq!(limit, 3);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_tui_command() {
        let cli = Cli::try_parse_from(["queria-cli", "tui"]).expect("tui should parse");
        assert!(matches!(cli.command, Command::Tui));
    }

    #[test]
    fn parses_index_here_defaults() {
        let cli =
            Cli::try_parse_from(["queria-cli", "index-here"]).expect("index-here should parse");
        match cli.command {
            Command::IndexHere {
                token_env,
                edge_url_env,
                depth,
                yes,
                dry_run,
            } => {
                assert_eq!(token_env, "QUERIA_AGENT_TOKEN");
                assert_eq!(edge_url_env, "QUERIA_EDGE_URL");
                assert_eq!(depth, 4);
                assert!(!yes);
                assert!(!dry_run);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_index_here_flags() {
        let cli = Cli::try_parse_from([
            "queria-cli",
            "index-here",
            "--token-env",
            "MY_TOKEN",
            "--edge-url-env",
            "MY_EDGE",
            "--depth",
            "2",
            "--yes",
            "--dry-run",
        ])
        .expect("index-here flags should parse");
        match cli.command {
            Command::IndexHere {
                token_env,
                edge_url_env,
                depth,
                yes,
                dry_run,
            } => {
                assert_eq!(token_env, "MY_TOKEN");
                assert_eq!(edge_url_env, "MY_EDGE");
                assert_eq!(depth, 2);
                assert!(yes);
                assert!(dry_run);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
