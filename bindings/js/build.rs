fn main() {
  println!("cargo:rerun-if-changed=__test__/native/raw_union_oracle.c");
  if std::env::var_os("CARGO_FEATURE_TEST_HOOKS").is_none() {
    return;
  }
  cc::Build::new()
    .file("__test__/native/raw_union_oracle.c")
    .warnings(true)
    .compile("dynwinrt_raw_union_oracle");
}
