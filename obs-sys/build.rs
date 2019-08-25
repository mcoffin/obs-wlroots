extern crate bindgen;
extern crate pkg_config;

use std::env;
use std::path::PathBuf;

fn main() {
    let _obs = pkg_config::probe_library("libobs")
        .expect("Error finding libobs with pkg-config");

    let bindings = bindgen::builder()
        .header("wrapper.h")
        .blacklist_type("_bindgen_ty_2")
        .generate()
        .expect("Error generating libobs bindings");

    let out_path: PathBuf = env::var("OUT_DIR").unwrap().into();

    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Error writing bindings to file");
}
