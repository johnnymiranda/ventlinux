use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor = manifest.join("../../vendor/libventrilo3");
    println!("cargo:rerun-if-changed={}", vendor.display());
    println!("cargo:rerun-if-changed=wrapper.h");

    let speex = pkg_config::probe_library("speex").expect("speex");
    let speexdsp = pkg_config::probe_library("speexdsp").expect("speexdsp");
    let opus = pkg_config::probe_library("opus").expect("opus");

    let mut build = cc::Build::new();
    build
        .files([
            vendor.join("libventrilo3.c"),
            vendor.join("libventrilo3_message.c"),
            vendor.join("ventrilo3_handshake.c"),
            vendor.join("v3shim.c"),
        ])
        .include(&vendor)
        .define("NO_AUTOMAKE", None)
        .define("HAVE_SPEEX", "1")
        .define("HAVE_SPEEX_DSP", "1")
        .define("HAVE_OPUS", "1")
        .flag("-w")
        .flag("-pthread")
        .pic(true)
        .std("gnu11");

    for lib in [&speex, &speexdsp, &opus] {
        for p in &lib.include_paths {
            build.include(p);
        }
    }
    build.compile("ventrilo3");

    println!("cargo:rustc-link-lib=pthread");
    println!("cargo:rustc-link-lib=m");
    for lib in [&speex, &speexdsp, &opus] {
        for name in &lib.libs {
            println!("cargo:rustc-link-lib={name}");
        }
        for p in &lib.link_paths {
            println!("cargo:rustc-link-search=native={}", p.display());
        }
    }

    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}", vendor.display()))
        .allowlist_function("v3_.*")
        .allowlist_function("_v3_recv")
        .allowlist_function("_v3_process_message")
        .allowlist_type("v3_.*")
        .allowlist_type("_v3_event.*")
        .allowlist_type("_v3_events")
        .allowlist_type("_v3_net_message")
        .allowlist_var("V3_.*")
        .blocklist_type("_v3_luser")
        .blocklist_type("__v3_luser")
        .blocklist_type("_v3_server")
        .blocklist_type("__v3_server")
        .blocklist_type("v3_vrf_data")
        .blocklist_function("v3_vrf_.*")
        .layout_tests(false)
        .generate_comments(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    for p in speex
        .include_paths
        .iter()
        .chain(&speexdsp.include_paths)
        .chain(&opus.include_paths)
    {
        builder = builder.clang_arg(format!("-I{}", p.display()));
    }

    let bindings = builder.generate().expect("bindgen");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("write bindings");
}
