use obs::sys as obs_sys;

#[test]
fn api_ver_correct() {
    let version = obs::libobs_api_ver();
    assert_eq!(version.major() as u32, obs_sys::LIBOBS_API_MAJOR_VER);
    assert_eq!(version.minor() as u32, obs_sys::LIBOBS_API_MINOR_VER);
    assert_eq!(version.patch() as u32, obs_sys::LIBOBS_API_PATCH_VER);
}
