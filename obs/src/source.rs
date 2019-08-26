use ::obs_sys as sys;
use std::ffi;
use std::mem;
use std::ptr;

pub unsafe fn register_source(info: &'static sys::obs_source_info) {
    obs_register_source(info as *const sys::obs_source_info)
}

pub unsafe fn obs_register_source(info: *const sys::obs_source_info) {
    sys::obs_register_source_s(info, mem::size_of::<sys::obs_source_info>());
}

pub fn empty_source() -> sys::obs_source_info {
    sys::obs_source_info {
        id: ptr::null(),
        type_: 0,
        output_flags: 0,
        get_name: None,
        create: None,
        destroy: None,
        get_width: None,
        get_height: None,
        get_defaults: None,
        get_properties: None,
        update: None,
        activate: None,
        deactivate: None,
        show: None,
        hide: None,
        video_tick: None,
        video_render: None,
        filter_video: None,
        filter_audio: None,
        enum_active_sources: None,
        save: None,
        load: None,
        mouse_click: None,
        mouse_move: None,
        mouse_wheel: None,
        focus: None,
        key_click: None,
        filter_remove: None,
        type_data: ptr::null_mut(),
        free_type_data: None,
        audio_render: None,
        enum_all_sources: None,
        transition_start: None,
        transition_stop: None,
        get_defaults2: None,
        get_properties2: None
    }
}

pub trait Source: 'static + Sized {
    const ID: &'static [u8];
    const NAME: &'static [u8];
    fn create(settings: &mut sys::obs_data_t, source: &mut sys::obs_source_t) -> Result<Self, String>;

    fn update(&mut self, _settings: &mut sys::obs_data_t) {}
}

unsafe extern "C" fn get_name<S: Source>(_data: *mut ffi::c_void) -> *const i8 {
    ffi::CStr::from_bytes_with_nul_unchecked(S::NAME).as_ptr()
}

unsafe extern "C" fn create<S: Source>(settings: *mut sys::obs_data_t, source: *mut sys::obs_source_t) -> *mut ffi::c_void {
    let settings: &mut sys::obs_data_t = settings.as_mut().unwrap();
    let source: &mut sys::obs_source_t = source.as_mut().unwrap();
    let ret: S = S::create(settings, source)
        .expect("Error creating source");
    let ret = Box::new(ret);
    mem::transmute(Box::into_raw(ret))
}

unsafe extern "C" fn destroy<S: Source>(data: *mut ffi::c_void) {
    let _b: Box<S> = Box::from_raw(mem::transmute(data));
}

unsafe extern "C" fn update<S: Source>(data: *mut ffi::c_void, settings: *mut sys::obs_data_t) {
    let data: *mut S = mem::transmute(data);
    let data: &mut S = data.as_mut().unwrap();
    let settings: &mut sys::obs_data_t = settings.as_mut().unwrap();
    data.update(settings);
}

pub trait VideoSource: Source {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
}

unsafe extern "C" fn video_get_width<S: VideoSource>(data: *mut ffi::c_void) -> u32 {
    let data: &mut S = mem::transmute(data);
    data.width()
}

unsafe extern "C" fn video_get_height<S: VideoSource>(data: *mut ffi::c_void) -> u32 {
    let data: &mut S = mem::transmute(data);
    data.height()
}

pub struct SourceInfo(sys::obs_source_info);

impl SourceInfo {
    #[inline(always)]
    pub fn as_raw<'a>(&'a self) -> &'a sys::obs_source_info {
        &self.0
    }
}

pub fn video_source_info<S: VideoSource>() -> SourceInfo {
    let mut info = empty_source();
    info.id = ffi::CStr::from_bytes_with_nul(S::ID).expect("Invalid ID").as_ptr();
    info.type_ = sys::obs_source_type_OBS_SOURCE_TYPE_INPUT;
    info.output_flags = sys::OBS_SOURCE_VIDEO;
    info.get_name = Some(get_name::<S>);
    info.create = Some(create::<S>);
    info.destroy = Some(destroy::<S>);
    info.get_width = Some(video_get_width::<S>);
    info.get_height = Some(video_get_height::<S>);
    info.update = Some(update::<S>);
    SourceInfo(info)
}
