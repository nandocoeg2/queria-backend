mod bootstrap;
mod doctor_mcp;

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
}

#[derive(Debug, Subcommand)]
enum DoctorCommand {
    Mcp {
        #[arg(long, default_value = "http://127.0.0.1:17672/mcp")]
        url: String,
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
    }
}
