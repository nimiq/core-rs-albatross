use quote::format_ident;
use std::env;
use std::fs;
use std::path::Path;
use syn::Item;
use syn::ItemEnum;
use syn::ItemStruct;

use quote::quote;

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("gen.rs");
    let mut descriptors = Vec::new();

    let types =
        fs::read_to_string("/Users/stefan/code/nimiq/core-rs-albatross/rpc-interface/src/types.rs")
            .unwrap();
    let ast = syn::parse_file(&types).unwrap();
    let enums = extract_enums(ast.clone());
    let structs = extract_structs(ast);
    enums.iter().for_each(|item_enum| {
        let name = item_enum.ident.to_string();
        let r#type = format_ident!("{}", name);

        descriptors.push(quote! {
            ContentDescriptorOrReference::ContentDescriptorObject(ContentDescriptorObject {
                name: #name.to_string(),
                description: None, // Add descriptions as needed
                summary: None,
                schema: JSONSchema::JsonSchemaObject(schemars::gen::SchemaGenerator::default()
                .into_root_schema_for::<#r#type>()),
                required: Some(true),
                deprecated: None,
            })
        });
    });

    structs.iter().for_each(|item_struct| {
        let name = item_struct.ident.to_string();
        if name == "RPCData" {
            return;
        }
        let r#type = format_ident!("{}", name);

        descriptors.push(quote! {
            ContentDescriptorOrReference::ContentDescriptorObject(ContentDescriptorObject {
                name: #name.to_string(),
                description: None, // Add descriptions as needed
                summary: None,
                schema: JSONSchema::JsonSchemaObject(schemars::gen::SchemaGenerator::default()
                .into_root_schema_for::<#r#type>()),
                required: Some(true),
                deprecated: None,
            })
        });
    });

    let final_quote = quote! {
        use std::{
            collections::BTreeSet,
            fmt::{self, Display, Formatter},
            str::FromStr,
        };

        use clap::ValueEnum;
        use openrpc::{ContentDescriptorObject, ContentDescriptorOrReference};
        use serde::{Deserialize, Serialize};
        use nimiq_collections::BitSet;

        use nimiq_keys::{Address, Ed25519PublicKey, Ed25519Signature, PrivateKey};

        use nimiq_account::{Log, TransactionLog};
        use nimiq_bls::CompressedPublicKey;

        use nimiq_primitives::coin::Coin;
        use nimiq_hash::{Blake2bHash, Blake2sHash};
        use nimiq_block::{MicroJustification, MultiSignature};

        use nimiq_primitives::networks::NetworkId;
        use nimiq_vrf::VrfSeed;
        use nimiq_transaction::account::htlc_contract::AnyHash;

        use nimiq_rpc_interface::serde_helpers::hex::{deserialize as deserialize_hex, serialize as serialize_hex};

        use serde_with::{serde_as, DeserializeFromStr, SerializeDisplay};

        use crate::openrpc::JSONSchema;

        #(#enums)*

        #(#structs)*

        pub fn content_descriptor_components() -> Vec<ContentDescriptorOrReference> {
            vec![#(#descriptors),*]
        }

        impl Display for ValidityStartHeight {
            fn fmt(&self, f: &mut Formatter) -> fmt::Result {
                match self {
                    Self::Absolute(n) => write!(f, "{n}"),
                    Self::Relative(n) => write!(f, "+{n}"),
                }
            }
        }

        impl FromStr for ValidityStartHeight {
            type Err = <u32 as FromStr>::Err;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let s = s.trim();
                if let Some(stripped) = s.strip_prefix('+') {
                    Ok(Self::Relative(stripped.parse()?))
                } else {
                    Ok(Self::Absolute(s.parse()?))
                }
            }
        }
    };

    fs::write(&dest_path, final_quote.to_string()).unwrap();
    // println!("cargo::rerun-if-changed=build.rs");
}

fn extract_enums(ast: syn::File) -> Vec<ItemEnum> {
    ast.items
        .into_iter()
        .filter_map(|item| match item {
            Item::Enum(mut item_enum) => {
                item_enum.attrs.push(syn::parse_quote! {
                    #[derive(schemars::JsonSchema)]
                });

                Some(item_enum)
            }
            _ => None,
        })
        .collect()
}

fn extract_structs(ast: syn::File) -> Vec<ItemStruct> {
    ast.items
        .into_iter()
        .filter_map(|item| match item {
            Item::Struct(mut item_struct) => {
                item_struct.attrs.push(syn::parse_quote! {
                    #[derive(schemars::JsonSchema)]
                });

                Some(item_struct)
            }
            _ => None,
        })
        .collect()
}
