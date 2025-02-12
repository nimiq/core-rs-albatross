use std::process::exit;

use anyhow::Error;
use clap::{Args, Parser, Subcommand};
use nimiq_primitives::{coin::Coin, networks::NetworkId};

mod commands;

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(subcommand_required = true)]
pub struct Cli {
    #[clap(flatten)]
    global_opts: CliGlobalOpts,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Args)]
struct CliGlobalOpts {
    #[clap(long, short, global = true, default_value_t = Coin::ZERO)]
    fee: Coin,
    #[clap(long, short, global = true, default_value_t = NetworkId::MainAlbatross)]
    network: NetworkId,
    #[clap(long, short, global = true, default_value_t = 0)]
    validity_start: u32,
}

#[derive(Subcommand)]
enum Commands {
    Basic(BasicArgs),
    Htlc(HtlcArgs),
    Stake(StakeArgs),
    Validator(ValidatorArgs),
    Vesting(VestingArgs),
}

#[derive(Debug, Args)]
struct BasicArgs {
    #[clap(flatten)]
    args: commands::basic::AllArgs,
}

#[derive(Debug, Args)]
#[command(subcommand_required = true)]
struct HtlcArgs {
    #[command(subcommand)]
    command: commands::htlc::HtlcCommands,
}

#[derive(Debug, Args)]
#[command(subcommand_required = true)]
struct StakeArgs {
    #[command(subcommand)]
    command: commands::stake::StakeCommands,
}

#[derive(Debug, Args)]
#[command(subcommand_required = true)]
struct ValidatorArgs {
    #[command(subcommand)]
    command: commands::validator::ValidatorCommands,
}

#[derive(Debug, Args)]
#[command(subcommand_required = true)]
struct VestingArgs {
    #[command(subcommand)]
    command: commands::vesting::VestingCommands,
}

fn run_app() -> Result<(), Error> {
    let cli = Cli::parse();

    let CliGlobalOpts { fee, network, validity_start } = cli.global_opts;

    let result = match cli.command {
        Commands::Basic(args) => {
            commands::basic::get_tx(args.args, fee, validity_start,  network)?
        }
        Commands::Htlc(sub) => {
            commands::htlc::get_tx(sub.command, fee, validity_start, network)?
        }
        Commands::Stake(sub) => {
            commands::stake::get_tx(sub.command, fee, validity_start, network)?
        }
        Commands::Validator(sub) => {
            commands::validator::get_tx(sub.command, fee, validity_start, network)?
        }
        Commands::Vesting(sub) => {
            commands::vesting::get_tx(sub.command, fee, validity_start, network)?
        }
    };

    println!("{}", result.to_hex());
    Ok(())
}

fn main() {
    exit(match run_app() {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    });
}
