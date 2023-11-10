use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nimiq_primitives::{key_nibbles::KeyNibbles, trie::trie_node::TrieNode};
use postcard;

fn serialize_trie_node(c: &mut Criterion, node: TrieNode, name: &str) {
    let mut buf = [0; 150000];
    let mut group = c.benchmark_group(name);
    group.bench_function("serde", |b| {
        b.iter(|| {
            let _ = black_box(postcard::to_slice(black_box(&node), &mut buf).unwrap());
        })
    });
    group.bench_function("plain", |b| {
        b.iter(|| {
            let _ = black_box(node.serialize(&mut buf.as_mut_slice()).unwrap());
        })
    });
    group.finish();
}

fn serialize_empty_node(c: &mut Criterion) {
    let node = TrieNode::new_empty(KeyNibbles::default());
    serialize_trie_node(c, node, "empty")
}
fn serialize_root_node(c: &mut Criterion) {
    let node = TrieNode::new_root();
    serialize_trie_node(c, node, "root")
}
fn serialize_incomplete_root_node(c: &mut Criterion) {
    let node = TrieNode::new_root_incomplete();
    serialize_trie_node(c, node, "root_incomplete")
}
fn serialize_leaf_node32(c: &mut Criterion) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 32]);
    serialize_trie_node(c, node, "leaf32")
}
fn serialize_leaf_node128(c: &mut Criterion) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 128]);
    serialize_trie_node(c, node, "leaf128")
}
fn serialize_leaf_node1024(c: &mut Criterion) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 1024]);
    serialize_trie_node(c, node, "leaf1024")
}
fn serialize_leaf_node16384(c: &mut Criterion) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 16384]);
    serialize_trie_node(c, node, "leaf16834")
}
fn serialize_leaf_node32768(c: &mut Criterion) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 32768]);
    serialize_trie_node(c, node, "leaf32768")
}
fn serialize_leaf_node131072(c: &mut Criterion) {
    let node = TrieNode::new_leaf(KeyNibbles::default(), vec![0x42; 131072]);
    serialize_trie_node(c, node, "leaf131072")
}

criterion_group!(
    trie_node,
    serialize_empty_node,
    serialize_root_node,
    serialize_incomplete_root_node,
    serialize_leaf_node32,
    serialize_leaf_node128,
    serialize_leaf_node1024,
    serialize_leaf_node16384,
    serialize_leaf_node32768,
    serialize_leaf_node131072,
);
criterion_main!(trie_node);
