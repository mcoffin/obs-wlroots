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
        .rustified_enum("*")
        .blacklist_item("true_")
        .blacklist_item("false_")
        .blacklist_item(".+_defined")
        .blacklist_item("__have_.+")
        .blacklist_item("__glibc_c[0-9]+_.+")
        .blacklist_item("math_errhandling")
        // .rustified_enum("obs_combo_format")
        // .rustified_enum("video_format")
        // .rustified_enum("obs_scene_duplicate_type")
        // .rustified_enum("obs_transition_mode")
        // .rustified_enum("obs_transition_target")
        // .rustified_enum("obs_transition_scale_type")
        // .rustified_enum("obs_monitoring_type")
        // .rustified_enum("obs_deinterlace_field_order")
        // .rustified_enum("obs_deinterlace_mode")
        // .rustified_enum("obs_obj_type")
        // .rustified_enum("obs_base_effect")
        .generate()
        .expect("Error generating libobs bindings");

    let out_path: PathBuf = env::var("OUT_DIR").unwrap().into();

    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Error writing bindings to file");
}
