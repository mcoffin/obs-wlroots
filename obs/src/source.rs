use ::obs_sys as sys;
use std::mem;
use std::ptr;

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
