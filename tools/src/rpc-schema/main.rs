mod openrpc;
mod parser;

use std::{env, fs};

use anyhow::Error;
use clap::{crate_authors, crate_description, crate_version, Arg, Command};
use syn::parse_file;
use thiserror::Error;

use crate::openrpc::OpenRpcBuilder;

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

    let mut builder = OpenRpcBuilder::builder()
        .version(
            matches
                .get_one::<String>("openrpc_version")
                .ok_or(AppError::OpenRpcArgumentMissing("version".to_string()))?
                .to_string(),
        )
        .title(
            matches
                .get_one::<String>("openrpc_title")
                .ok_or(AppError::OpenRpcArgumentMissing("title".to_string()))?
                .to_string(),
        );

    match fs::read_dir(
        matches
            .get_one::<String>("source")
            .ok_or(AppError::SourceCode)?,
    ) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(contents) = fs::read_to_string(&path) {
                        match parse_file(&contents) {
                            Ok(ast) => {
                                let structs = parser::extract_structs_from_ast(&ast);
                                let fns = parser::extract_fns_from_ast(&ast);
                                for index in 0..structs.len() {
                                    builder = builder.with_schema(structs.get(index).unwrap());
                                }
                                for index in 0..fns.len() {
                                    builder = builder.with_method(
                                        fns.get(index).unwrap(),
                                        path.file_name()
                                            .unwrap()
                                            .to_string_lossy()
                                            .replace(".rs", ""),
                                    );
                                }
                            }
                            Err(err) => {
                                return Err(AppError::ParsingError(err).into());
                            }
                        }
                    }
                }
            }
        }
        Err(err) => return Err(AppError::LoadDirectoryError(err).into()),
    }

    print!(
        "{}",
        serde_json::to_string_pretty(&builder.build()).expect("Failed to serialize OpenRPC spec")
    );

    Ok(())
}

#[derive(Debug, Error)]
enum AppError {
    #[error("Unable to load specified directory: {0}")]
    LoadDirectoryError(#[from] std::io::Error),
    #[error("Failed to parse Rust source file: {0}")]
    ParsingError(#[from] syn::Error),
    #[error("OpenRPC {0} argument is missing")]
    OpenRpcArgumentMissing(String),
    #[error("The source argument is missing")]
    SourceCode,
}
