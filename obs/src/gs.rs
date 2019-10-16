pub struct Texture(*mut obs_sys::gs_texture_t);

#[allow(non_snake_case)]
fn translate_format(fmt: obs_sys::video_format) -> obs_sys::gs_color_format {
    use obs_sys::video_format::*;
    use obs_sys::gs_color_format::*;
    // TODO: complete this
    match fmt {
        VIDEO_FORMAT_BGRA => GS_BGRA,
        VIDEO_FORMAT_RGBA => GS_RGBA,
        _ => panic!("Unknown video format: {:?}", &fmt),
    }
}

impl Texture {
    #[inline(always)]
    pub fn as_raw(&mut self) -> *mut obs_sys::gs_texture_t {
        self.0
    }
}

impl<'a> From<&'a mut obs_sys::obs_source_frame> for Texture {
    fn from(source_frame: &'a mut obs_sys::obs_source_frame) -> Texture {
        let ptr = unsafe {
            obs_sys::gs_texture_create(source_frame.width, source_frame.height, translate_format(source_frame.format), 1, source_frame.data.as_mut_ptr() as *mut *const u8, 0)
        };
        Texture(ptr)
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                obs_sys::gs_texture_destroy(self.0);
            }
        }
    }
}
