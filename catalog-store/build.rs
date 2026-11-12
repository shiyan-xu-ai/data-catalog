//! Compiles the vendored Lance table protos into a descriptor set, so manifest and transaction
//! files can be decoded field by field. `protos/` is copied from `lance-table` at the pinned
//! `lance` version; refresh it whenever that pin changes.

use prost::Message;

fn main() {
    println!("cargo:rerun-if-changed=protos");
    let descriptors = protox::compile(["table.proto", "transaction.proto"], ["protos"])
        .expect("compile vendored Lance protos");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("lance_table.bin"), descriptors.encode_to_vec())
        .expect("write descriptor set");
}
