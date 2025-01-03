#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]

use std::{env, fs};

use anyhow::Error;
use clap::{crate_authors, crate_description, crate_version, Arg, Command};
use openrpc::InfoObject;
use thiserror::Error;

use nimiq_rpc_interface::types;

mod openrpc;

fn main() -> Result<(), Error> {
    let matches = Command::new("RPC schema generator")
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(
            Arg::new("openrpc_version")
                .short('o')
                .long("openrpc-version")
                .value_name("OPENRPC_VERSION")
                .help("Specify the OpenRPC version of the document. Usually you want to match this with your release version number."),
        )
        .arg(
            Arg::new("openrpc_title")
                .short('t')
                .long("openrpc-title")
                .value_name("OPENRPC_TITLE")
                .default_value("Nimiq JSON-RPC Specification")
                .help("Specify the OpenRPC title of the document."),
        )
        .arg(
            Arg::new("source")
            .short('s')
            .long("source")
            .value_name("SOURCE")
            .default_value("rpc-interface/src")
            .help("The folder that contains the source of the RPC interface traits and structs.")
        )
        .get_matches();

    let doc = openrpc::OpenrpcDocument::default();
    let info = openrpc::InfoObject::default();
    let doc = doc.set_info(info);

    print!(
        "{}",
        serde_json::to_string_pretty(&doc).expect("Failed to serialize OpenRPC spec")
    );

    Ok(())
}

#[derive(Debug, Error)]
enum AppError {
    #[error("Unable to load specified directory: {0}")]
    LoadDirectoryError(#[from] std::io::Error),
    #[error("Failed to parse Rust source file: {0}")]
    OpenRpcArgumentMissing(String),
    #[error("The source argument is missing")]
    SourceCode,
}
