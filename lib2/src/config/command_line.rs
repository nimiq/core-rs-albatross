use structopt::StructOpt;


// FIXME:
//
// ```
// 4 | #[derive(Debug, StructOpt)]
//   |
// ```                 ^^^^^^^^^ use of undeclared type or module `paw`
//
// We don't want to use `paw` here and don't have the feature enable. Thus, this seems to be a bug.


/// Options that have been passed via command line arguments
// TODO: Add `StructOpt` to derive
#[derive(Debug)]
pub struct CommandLine {
    // TODO
}
