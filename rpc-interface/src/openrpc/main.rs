use openrpc::{Components, OpenrpcDocument};
use std::collections::BTreeMap;

include!(concat!(env!("OUT_DIR"), "/gen.rs"));

mod openrpc;

fn main() {
    let mut descroptors = BTreeMap::new();

    content_descriptor_components()
        .iter()
        .for_each(|desc| match desc {
            ContentDescriptorOrReference::ContentDescriptorObject(content_descriptor_object) => {
                descroptors.insert(
                    content_descriptor_object.name.clone(),
                    Some(serde_json::to_value(content_descriptor_object.clone()).unwrap()),
                );
            }
            ContentDescriptorOrReference::ReferenceObject(reference_object) => todo!(),
        });

    let doc = OpenrpcDocument {
        components: Some(Components {
            schemas: Some(descroptors),
            ..Default::default()
        }),
        ..Default::default()
    };

    let j = serde_json::to_string_pretty(&doc).unwrap();
    print!("{}", j);
}
