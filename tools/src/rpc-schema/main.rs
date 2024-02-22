mod openrpc;
mod parser;

use std::{
    error::Error,
    fs::File,
    io::{Read, Write},
    path::Path,
};

use crate::openrpc::OpenRpcBuilder;

fn main() -> Result<(), Box<dyn Error>> {
    let type_defs =
        Path::new("/Users/stefan/code/nimiq/core-rs-albatross/rpc-interface/src/types.rs");
    let mut file = File::open(type_defs)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    let ast = syn::parse_file(&content)?;
    let structs = parser::extract_structs_from_ast(&ast);
    content.clear();

    // blockchain/consensus/mempool/network/policy/validator/wallet/zkp_component
    let methods =
        Path::new("/Users/stefan/code/nimiq/core-rs-albatross/rpc-interface/src/blockchain.rs");
    let mut file = File::open(methods)?;
    file.read_to_string(&mut content)?;
    let ast = syn::parse_file(&content)?;
    let fns = parser::extract_fns_from_ast(&ast);

    let mut builder = OpenRpcBuilder::builder();
    for index in 0..structs.len() {
        builder = builder.with_schema(structs.get(index).unwrap());
    }

    for index in 0..fns.len() {
        builder = builder.with_method(fns.get(index).unwrap());
    }

    let definition = builder.build();
    let json_spec =
        serde_json::to_string_pretty(&definition).expect("Failed to serialize OpenRPC spec");
    let mut file = File::create("tools/src/rpc-schema/schema.json").expect("Failed to create file");
    file.write_all(json_spec.as_bytes())
        .expect("Failed to write to file");

    Ok(())
}
