use std::process::exit;

use anyhow::Error;
use clap::{Args, Parser, Subcommand};
use nimiq_primitives::{coin::Coin, networks::NetworkId};

mod commands;

#[derive(Parser)]
#[command(name = "mktx", subcommand_required = true, disable_help_flag = true, version = nimiq_utils::CARGO_VERSION)]
#[command(about = "CLI tool to create and sign Nimiq transactions offline")]
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
    #[clap(long, short = 'V', global = true)]
    validity_start: u32,
}

#[derive(Subcommand)]
enum Commands {
    /// Create and sign basic transactions with or without data
    Basic(BasicArgs),
    /// Manage HTLC transactions
    Htlc(HtlcArgs),
    /// Build staking transactions
    Stake(StakeArgs),
    /// Create and manage validator transactions
    Validator(ValidatorArgs),
    /// Create or redeem vesting contracts
    Vesting(VestingArgs),
}

#[derive(Debug, Args)]
struct BasicArgs {
    #[clap(flatten)]
    args: commands::basic::AllArgs,
}

#[derive(Debug, Args)]
#[command(subcommand_required = true, disable_help_subcommand = true)]
struct HtlcArgs {
    #[command(subcommand)]
    command: commands::htlc::HtlcCommands,
}

#[derive(Debug, Args)]
#[command(subcommand_required = true, disable_help_subcommand = true)]
struct StakeArgs {
    #[command(subcommand)]
    command: commands::stake::StakeCommands,
}

#[derive(Debug, Args)]
#[command(subcommand_required = true, disable_help_subcommand = true)]
struct ValidatorArgs {
    #[command(subcommand)]
    command: commands::validator::ValidatorCommands,
}

#[derive(Debug, Args)]
#[command(subcommand_required = true, disable_help_subcommand = true)]
struct VestingArgs {
    #[command(subcommand)]
    command: commands::vesting::VestingCommands,
}

fn run_app() -> Result<(), Error> {
    let cli = Cli::parse();

    let CliGlobalOpts {
        fee,
        network,
        validity_start,
    } = cli.global_opts;

    let result = match cli.command {
        Commands::Basic(args) => commands::basic::get_tx(args.args, fee, validity_start, network)?,
        Commands::Htlc(sub) => commands::htlc::get_tx(sub.command, fee, validity_start, network)?,
        Commands::Stake(sub) => commands::stake::get_tx(sub.command, fee, validity_start, network)?,
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
