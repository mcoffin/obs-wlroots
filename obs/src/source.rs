use ::obs_sys as sys;
use crate::properties;
use crate::gs;
use std::{
    ffi,
    mem,
    ptr,
    io::{
        self,
        Write,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SourceHandle(*mut sys::obs_source_t);

impl SourceHandle {
    #[inline(always)]
    pub fn new(handle: *mut sys::obs_source_t) -> Self {
        SourceHandle(handle)
    }

    #[inline(always)]
    pub fn as_raw(&self) -> *mut sys::obs_source_t {
        self.0
    }
}

unsafe impl Send for SourceHandle {}

#[inline(always)]
pub unsafe fn register_source(info: &'static sys::obs_source_info) {
    obs_register_source(info as *const sys::obs_source_info)
}

#[inline(always)]
pub unsafe fn obs_register_source(info: *const sys::obs_source_info) {
    sys::obs_register_source_s(info, mem::size_of::<sys::obs_source_info>());
}

fn empty_source(t: sys::obs_source_type) -> sys::obs_source_info {
    sys::obs_source_info {
        id: ptr::null(),
        type_: t,
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
        get_properties2: None,
        audio_mix: None,
        icon_type: sys::obs_icon_type::OBS_ICON_TYPE_DESKTOP_CAPTURE,
        media_play_pause: None,
        media_restart: None,
        media_stop: None,
        media_next: None,
        media_previous: None,
        media_get_duration: None,
        media_get_time: None,
        media_set_time: None,
        media_get_state: None,
        version: 0,
        unversioned_id: ptr::null(),
    }
}

pub trait Source: 'static + Sized {
    /// Unique identifier string, *must* include null-terminator
    const ID: &'static [u8];
    /// Source name, *must* include null-terminator
    const NAME: &'static [u8];
    fn create(settings: &mut sys::obs_data_t, source: &mut sys::obs_source_t) -> Result<Self, String>;

    fn update(&mut self, _settings: &mut sys::obs_data_t) {}
    fn get_properties(&mut self) -> properties::Properties {
        properties::Properties::new()
    }
}

unsafe extern "C" fn get_name<S: Source>(_data: *mut ffi::c_void) -> *const i8 {
    ffi::CStr::from_bytes_with_nul_unchecked(S::NAME).as_ptr()
}

unsafe extern "C" fn create<S: Source>(settings: *mut sys::obs_data_t, source: *mut sys::obs_source_t) -> *mut ffi::c_void {
    let settings: &mut sys::obs_data_t = settings.as_mut().unwrap();
    let source: &mut sys::obs_source_t = source.as_mut().unwrap();
    let ret: Result<S, _> = S::create(settings, source);
    if let Err(e) = ret.as_ref() {
        let stderr = io::stderr();
        let mut stderr = stderr.lock();
        let _result = writeln!(&mut stderr, "error: error creaiting source: {}", e);
        ptr::null_mut()
    } else {
        let ret = Box::new(ret);
        mem::transmute(Box::into_raw(ret))
    }
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

unsafe extern "C" fn get_properties<S: Source>(data: *mut ffi::c_void) -> *mut sys::obs_properties_t {
    let data: *mut S = mem::transmute(data);
    let data: &mut S = data.as_mut().unwrap();
    data.get_properties().into_raw()
}

pub trait VideoSource: Source {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn render(&mut self);
}

pub trait AsyncVideoSource: Source {
}

unsafe extern "C" fn video_get_width<S: VideoSource>(data: *mut ffi::c_void) -> u32 {
    let data: &mut S = mem::transmute(data);
    data.width()
}

unsafe extern "C" fn video_get_height<S: VideoSource>(data: *mut ffi::c_void) -> u32 {
    let data: &mut S = mem::transmute(data);
    data.height()
}

unsafe extern "C" fn video_render<S: VideoSource>(data: *mut ffi::c_void, _effect: *mut sys::gs_effect_t) {
    let data: &mut S = mem::transmute(data);
    data.render()
}

pub struct SourceInfo(sys::obs_source_info);

impl SourceInfo {
    #[inline(always)]
    pub fn as_raw<'a>(&'a self) -> &'a sys::obs_source_info {
        &self.0
    }
}

pub fn video_source_info<S: VideoSource>() -> SourceInfo {
    let mut info = empty_source(sys::obs_source_type::OBS_SOURCE_TYPE_INPUT);
    info.id = ffi::CStr::from_bytes_with_nul(S::ID).expect("Invalid ID").as_ptr();
    info.type_ = sys::obs_source_type::OBS_SOURCE_TYPE_INPUT;
    info.output_flags = sys::OBS_SOURCE_VIDEO;
    info.get_name = Some(get_name::<S>);
    info.create = Some(create::<S>);
    info.destroy = Some(destroy::<S>);
    info.get_width = Some(video_get_width::<S>);
    info.get_height = Some(video_get_height::<S>);
    info.update = Some(update::<S>);
    info.get_properties = Some(get_properties::<S>);
    info.video_render = Some(video_render::<S>);
    SourceInfo(info)
}

pub fn async_video_source_info<S: AsyncVideoSource>() -> SourceInfo {
    let mut info = empty_source(sys::obs_source_type::OBS_SOURCE_TYPE_INPUT);
    info.id = ffi::CStr::from_bytes_with_nul(S::ID).expect("Invalid ID").as_ptr();
    info.type_ = sys::obs_source_type::OBS_SOURCE_TYPE_INPUT;
    info.output_flags = sys::OBS_SOURCE_ASYNC_VIDEO;
    info.get_name = Some(get_name::<S>);
    info.create = Some(create::<S>);
    info.destroy = Some(destroy::<S>);
    info.update = Some(update::<S>);
    info.get_properties = Some(get_properties::<S>);
    SourceInfo(info)
}

pub fn obs_source_draw(texture: &mut gs::Texture, x: libc::c_int, y: libc::c_int, cx: u32, cy: u32, flip: bool) {
    unsafe {
        obs_sys::obs_source_draw(texture.as_raw(), x, y, cx, cy, flip);
    }
}
