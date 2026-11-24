fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    #[cfg(feature = "gpu-viewport")]
    build_usd_shaders();
}

#[cfg(feature = "gpu-viewport")]
fn build_usd_shaders() {
    use wesl::{Compiler, Feature};

    let mut compiler = Compiler::default();
    compiler.options.features.default = Feature::Error;

    // The mesh pipeline selects its variant using the adapter's depth sampling support.
    for (artifact, shadows) in [("usd_shadows", true), ("usd_no_shadows", false)] {
        compiler.options.features.set("shadow_maps", shadows);
        let shader = compiler
            .compile("src/gpu/usd/viewport.wesl")
            .unwrap_or_else(|error| panic!("could not compile {artifact}:\n{error}"));
        shader.emit_rerun_if_changed();

        validate_wgsl(artifact, &shader.to_string());
        shader.write_artifact(artifact);
    }
    println!("cargo::rerun-if-changed=src/gpu/usd/deform.wgsl");
    validate_wgsl("usd_deformation", include_str!("src/gpu/usd/deform.wgsl"));
}

#[cfg(feature = "gpu-viewport")]
fn validate_wgsl(artifact: &str, wgsl: &str) {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    let module = naga::front::wgsl::parse_str(wgsl)
        .unwrap_or_else(|error| panic!("invalid {artifact} WGSL:\n{}", error.emit_to_string(wgsl)));
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|error| panic!("invalid {artifact} WGSL:\n{}", error.emit_to_string(wgsl)));
}
