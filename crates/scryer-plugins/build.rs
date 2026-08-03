use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    for (source, output) in [
        ("probes/simd128.wat", "simd128_probe.wasm"),
        ("probes/relaxed_simd.wat", "relaxed_simd_probe.wasm"),
    ] {
        println!("cargo:rerun-if-changed={source}");
        let wat = fs::read_to_string(source).unwrap_or_else(|error| {
            panic!("failed to read Wasmtime capability probe {source}: {error}")
        });
        let wasm = wat::parse_str(&wat).unwrap_or_else(|error| {
            panic!("failed to compile Wasmtime capability probe {source}: {error}")
        });
        fs::write(out_dir.join(output), wasm).unwrap_or_else(|error| {
            panic!("failed to write Wasmtime capability probe {output}: {error}")
        });
    }
}
