use std::{env, fs, path::PathBuf};

fn main() {
  println!("cargo:rerun-if-changed=src/proto/mexc");

  let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

  let mut cfg = prost_build::Config::new();
  cfg.protoc_arg("--experimental_allow_proto3_optional");
  cfg.out_dir(&out_dir);

  let protos = fs::read_dir("src/proto/mexc")
    .unwrap()
    .filter_map(|e| {
      let p = e.unwrap().path();
      if p.extension().and_then(|s| s.to_str()) == Some("proto") {
        Some(p.to_string_lossy().to_string())
      } else {
        None
      }
    })
    .collect::<Vec<_>>();

  cfg
    .compile_protos(&protos, &["src/proto/mexc"])
    .expect("failed to compile MEXC .proto files");

  println!("cargo:rustc-env=PROTO_OUT={}", out_dir.display());
}
