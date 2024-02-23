mod openrpc;
mod parser;

use std::{
    error::Error,
    fs::{self, File},
    io::Write,
};

use syn::parse_file;

use crate::openrpc::OpenRpcBuilder;

fn main() -> Result<(), Box<dyn Error>> {
    let directory = "./rpc-interface/src";
    let mut builder = OpenRpcBuilder::builder();
    if let Ok(entries) = fs::read_dir(directory) {
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
                                builder = builder.with_method(fns.get(index).unwrap());
                            }
                        }
                        Err(err) => {
                            println!(
                                "Failed to parse file '{:?}': {}",
                                path.file_name().unwrap().to_str(),
                                err
                            );
                        }
                    }
                }
            }
        }
    } else {
        return Err("Unable to load directory".into());
    }

    let json_spec =
        serde_json::to_string_pretty(&builder.build()).expect("Failed to serialize OpenRPC spec");
    let mut file = File::create("tools/src/rpc-schema/schema.json").expect("Failed to create file");
    file.write_all(json_spec.as_bytes())
        .expect("Failed to write to file");

    Ok(())
}
