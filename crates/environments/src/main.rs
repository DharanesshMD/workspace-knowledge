use clap::{Parser, Subcommand};
use std::path::PathBuf;
use wk_environments::{openbao::OpenBao, profiles::Profile, Error, Result};
#[derive(Parser)]
#[command(about = "Separate OpenBao environment broker; never returns values through MCP")]
struct Args {
    #[arg(long)]
    config: PathBuf,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Schema,
    Import {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        approve_file_import: bool,
    },
    Run {
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    Export {
        #[arg(long)]
        destination: PathBuf,
    },
}
fn run(args: Args) -> Result<i32> {
    let bytes = std::fs::read(args.config).map_err(|_| Error::Local)?;
    let profile: Profile = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    profile.validate()?;
    if matches!(&args.command, Action::Schema) {
        println!("{}", profile.schema()?);
        return Ok(0);
    }
    let token = std::env::var("WK_BAO_TOKEN").map_err(|_| Error::Denied)?;
    let vault = OpenBao::new(profile.clone(), &token)?;
    match args.command {
        Action::Import {
            file,
            approve_file_import,
        } => {
            let values =
                wk_environments::import::read(&profile.workspace, &file, approve_file_import)?;
            vault.write(&values)?;
            println!("Imported approved profile (values omitted)");
        }
        Action::Run { command } => {
            return wk_environments::process::run(
                &profile.workspace,
                &command[0],
                &command[1..],
                &vault.read()?,
            )
        }
        Action::Export { destination } => {
            wk_environments::import::export(&profile.workspace, &destination, &vault.read()?)?
        }
        Action::Schema => unreachable!(),
    }
    Ok(0)
}
fn main() {
    match run(Args::parse()) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
