mod bootstrap;
mod database;
mod doctor_mcp;
mod embeddings;
mod evaluation;
mod retrieval;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "queria-cli")]
#[command(about = "Queria operational CLI")]
struct Cli {
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
}

#[derive(Debug, Subcommand)]
enum DoctorCommand {
    Mcp {
        #[arg(long, default_value = "http://127.0.0.1:17672/mcp")]
        url: String,
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
    },
}

#[derive(Debug, Subcommand)]
enum EvalCommand {
    Run {
        #[arg(long)]
        project: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Doctor {
            command: DoctorCommand::Mcp { url },
        } => doctor_mcp::run(&url).await,
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
                },
        } => retrieval::probe(&project, &query, include_global, limit).await,
        Command::Eval {
            command: EvalCommand::Run { project },
        } => evaluation::run(&project).await,
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
}
