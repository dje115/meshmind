fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protos = &[
        "../../proto/common.proto",
        "../../proto/cas.proto",
        "../../proto/events.proto",
        "../../proto/snapshot.proto",
        "../../proto/replication.proto",
        "../../proto/mesh.proto",
        "../../proto/research.proto",
        "../../proto/training.proto",
    ];

    prost_build::Config::new()
        .compile_protos(protos, &["../../proto/"])?;

    for proto in protos {
        println!("cargo:rerun-if-changed={proto}");
    }

    Ok(())
}
