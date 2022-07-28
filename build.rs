use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build = tonic_build::configure()
        .type_attribute("protowire.RpcBlockHeader", "#[derive(serde::Serialize)]")
        .type_attribute(
            "protowire.RpcBlockLevelParents",
            "#[derive(serde::Serialize)]",
        );

    let proto_path: &Path = "proto/protowire.proto".as_ref();
    let proto_dir = proto_path.parent().unwrap();
    build.compile(&[proto_path], &[proto_dir])?;
    Ok(())
}
