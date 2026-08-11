use std::process::exit;

use anyhow::Error;
use clap::{Args, Parser, Subcommand};
use nimiq_primitives::{coin::Coin, networks::NetworkId};

mod commands;

#[derive(Parser)]
#[command(name = "mktx", subcommand_required = true, version = nimiq_utils::CARGO_VERSION)]
#[command(about = "CLI tool to create and sign Nimiq transactions offline")]
// `-V` is `--validity-start` here, so the auto-generated version flag (which also claims `-V`)
// is replaced below by a long-only one; otherwise the two collide and debug builds panic.
#[command(disable_version_flag = true)]
pub struct Cli {
    /// Print the version and exit
    #[arg(long, short = 'v', action = clap::ArgAction::Version)]
    version: Option<bool>,

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
    validity_start: Option<u32>,
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

    let validity_start =
        validity_start.ok_or_else(|| anyhow::anyhow!("--validity-start is required"))?;

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

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    /// Optional arguments must be named flags, never positionals.
    ///
    /// Clap binds a lone trailing value to the *first* optional positional, so with more
    /// than one of them there is no way to skip an earlier argument to reach a later one.
    /// Worse, the misbinding is silent whenever the two arguments parse alike: signal data
    /// and an Ed25519 private key are both 32 hex-encoded bytes, so a value meant for one
    /// used to be accepted as the other and produce a valid, signed, wrong transaction.
    #[test]
    fn optional_arguments_are_never_positional() {
        fn check(cmd: &clap::Command, path: &str) {
            for arg in cmd.get_positionals() {
                assert!(
                    arg.is_required_set(),
                    "`{path}`: positional `{}` is optional; make it a flag with #[arg(long)]",
                    arg.get_id()
                );
            }
            for sub in cmd.get_subcommands() {
                check(sub, &format!("{path} {}", sub.get_name()));
            }
        }

        check(&Cli::command(), "mktx");
    }

    /// Clap's own lint; catches duplicate short/long flags across the command tree.
    #[test]
    fn cli_is_well_formed() {
        Cli::command().debug_assert();
    }
}
