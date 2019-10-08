use std::borrow::Cow;
use std::collections::{BTreeMap, LinkedList};
use std::sync::{atomic, Arc, RwLock, Mutex};
use std::thread;
use ::obs::sys as obs_sys;
use crate::shm;
use wayland_client::protocol::wl_output;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_shm_pool;
use wayland_protocols::unstable::xdg_output::v1::client::zxdg_output_manager_v1;
use wayland_protocols::unstable::xdg_output::v1::client::zxdg_output_v1;
use wayland_protocols::wlr::unstable::screencopy::v1::client::zwlr_screencopy_manager_v1;
use wayland_protocols::wlr::unstable::screencopy::v1::client::zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1};

const SOURCE_INFO_ID: &'static [u8] = b"obs_wlroots\0";
const SOURCE_NAME: &'static [u8] = b"wlroots output\0";

struct VideoThread {
    thread: Option<thread::JoinHandle<()>>,
    running: Arc<atomic::AtomicBool>,
}

impl VideoThread {
    pub fn new(
        source: obs::source::SourceHandle,
        output_id: u32,
        overlay_cursor: bool,
        display: Arc<wayland_client::Display>,
    ) -> VideoThread {
        use wayland_client::NewProxy;

        let running = Arc::new(atomic::AtomicBool::new(true));
        let stored_running = running.clone();
        let t = thread::spawn(move || {
            let mut display_events = display.create_event_queue();
            let output: Arc<RwLock<Option<wl_output::WlOutput>>> = Arc::new(RwLock::new(None));
            let gm_output = output.clone();
            let gm_running = running.clone();
            let global_manager = wayland_client::GlobalManager::new(&display);
            display_events.sync_roundtrip()
                .expec
            if output.read().unwrap().is_none() {
                running.store(false, atomic::Ordering::Relaxed);
                return;
            }
            let shm = global_manager.instantiate_exact::<wl_shm::WlShm, _>(1, |shm| shm.implement_dummy())
                .expect("Error creating wl_shm");
            let screencopy_manager = global_manager.instantiate_exact::<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, _>(1, |mgr| mgr.implement_dummy())
                .expect("Error creating wlr_screencopy_manager_v1");
            display_events.sync_roundtrip();
            let frame = WlrFrame::new(source, shm);
            let waiting = frame.waiting.clone();
            let mut frame = Container::new(frame);
            while running.load(atomic::Ordering::Relaxed) {
                let output = output.read().unwrap();
                if output.is_none() {
                    return;
                }
                let output = output.as_ref().unwrap();
                if !waiting.compare_and_swap(false, true, atomic::Ordering::SeqCst) {
                    let frame = frame.clone();
                    // let wlr_frame = screencopy_manager.capture_output(overlay_cursor as i32, output, move |f: NewProxy<_>| {
                    //     f.implement(frame, ())
                    // }).expect("error creating frame");
                }
                display_events.sync_roundtrip()
                    .expect("error waiting on display events");
            }
        });
        VideoThread {
            thread: Some(t),
            running: stored_running,
        }
    }
}

impl Drop for VideoThread {
    fn drop(&mut self) {
        self.running.store(false, atomic::Ordering::Relaxed);
        self.thread.take().map(thread::JoinHandle::join);
    }
}

struct OutputMetadata {
    name: String,
}

impl OutputMetadata {
    fn new() -> OutputMetadata {
        OutputMetadata {
            name: "<unknown>".to_string(),
        }
    }
}

struct WlrFrame {
    fd: Option<shm::ShmFd<&'static str>>,
    shm: wl_shm::WlShm,
    pool: Option<wl_shm_pool::WlShmPool>,
    buffer: Option<wayland_client::protocol::wl_buffer::WlBuffer>,
    format: u32,
    width: u32,
    height: u32,
    stride: u32,
    current_size: usize,
    source_handle: obs::source::SourceHandle,
    waiting: Arc<atomic::AtomicBool>,
}

const WLR_FRAME_SHM_PATH: &'static str = "/obs_wlroots";

impl WlrFrame {
    fn new(source: obs::source::SourceHandle, shm: wl_shm::WlShm) -> WlrFrame {
        WlrFrame {
            fd: None,
            shm: shm,
            pool: None,
            buffer: None,
            format: 0,
            width: 0,
            height: 0,
            stride: 0,
            current_size: 0,
            source_handle: source,
            waiting: Arc::new(atomic::AtomicBool::new(false)),
        }
    }

    pub fn size(&self) -> usize {
        (self.height as usize) * (self.stride as usize)
    }

    fn ensure_size(&mut self) {
        use wayland_client::NewProxy;
        if self.fd.is_none() || self.current_size < self.size() {
            self.fd = shm::ShmFd::open(WLR_FRAME_SHM_PATH, libc::O_CREAT | libc::O_RDWR, 0).ok();
            unsafe {
                shm::unlink(WLR_FRAME_SHM_PATH);
                libc::ftruncate(self.fd.as_ref().unwrap().as_raw(), self.size() as libc::off_t);
            }
            let new_pool = self.shm.create_pool(self.fd.as_ref().map(shm::ShmFd::as_raw).unwrap(), self.size() as i32, |p: NewProxy<_>| p.implement_dummy()).unwrap();
            let new_buffer = new_pool.create_buffer(0, self.width as i32, self.height as i32, self.stride as i32, self.buffer_format().unwrap(), |b: NewProxy<_>| b.implement_dummy()).unwrap();
            self.buffer = Some(new_buffer);
            self.pool = Some(new_pool);
            self.current_size = self.size();
        }
    }

    #[inline(always)]
    fn buffer_format(&self) -> Option<wl_shm::Format> {
        wl_shm::Format::from_raw(self.format)
    }
}

impl Drop for WlrFrame {
    fn drop(&mut self) {
        println!("obs_wlroots: WlrFrame::drop");
    }
}

impl zwlr_screencopy_frame_v1::EventHandler for WlrFrame {
    fn buffer(
        &mut self,
        object: ZwlrScreencopyFrameV1,
        format: u32,
        width: u32,
        height: u32,
        stride: u32
    ) {
        use wayland_client::NewProxy;
        println!("obs_wlroots: WlrFrame: buffer");

        self.format = format;
        self.width = width;
        self.height = height;
        self.stride = stride;
        self.ensure_size();
        object.copy(self.buffer.as_ref().unwrap());
    }

    fn ready(
        &mut self,
        object: ZwlrScreencopyFrameV1,
        tv_sec_hi: u32,
        tv_sec_lo: u32,
        tv_nsec: u32
    ) {
        println!("obs_wlroots: WlrFrame: ready");
        self.waiting.store(false, atomic::Ordering::Relaxed);
        object.destroy();
    }

    fn failed(
        &mut self,
        object: ZwlrScreencopyFrameV1
    ) {
        println!("obs_wlroots: WlrFrame: failed");
        self.waiting.store(false, atomic::Ordering::Relaxed);
        object.destroy();
    }
}

struct Container<T>(Arc<Mutex<T>>);

impl<T> Clone for Container<T> {
    fn clone(&self) -> Self {
        Container(self.0.clone())
    }
}

impl<T: Sized> Container<T> {
    fn new(value: T) -> Container<T> {
        Container(Arc::new(Mutex::new(value)))
    }
}

impl<T: zwlr_screencopy_frame_v1::EventHandler> zwlr_screencopy_frame_v1::EventHandler for Container<T> {
    fn buffer(
        &mut self,
        object: ZwlrScreencopyFrameV1,
        format: u32,
        width: u32,
        height: u32,
        stride: u32
    ) {
        let mut content = self.0.lock().unwrap();
        content.buffer(object, format, width, height, stride);
    }

    fn ready(
        &mut self,
        object: ZwlrScreencopyFrameV1,
        tv_sec_hi: u32,
        tv_sec_lo: u32,
        tv_nsec: u32
    ) {
        let mut content = self.0.lock().unwrap();
        content.ready(object, tv_sec_hi, tv_sec_lo, tv_nsec);
    }

    fn failed(
        &mut self,
        object: ZwlrScreencopyFrameV1
    ) {
        let mut content = self.0.lock().unwrap();
        content.failed(object);
    }
}

pub struct WlrSource {
    display: Arc<wayland_client::Display>,
    display_events: wayland_client::EventQueue,
    shm: wl_shm::WlShm,
    output_manager: zxdg_output_manager_v1::ZxdgOutputManagerV1,
    screencopy_manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    outputs: Arc<RwLock<BTreeMap<u32, wl_output::WlOutput>>>,
    xdg_outputs: LinkedList<zxdg_output_v1::ZxdgOutputV1>,
    output_metadata: Arc<RwLock<BTreeMap<u32, OutputMetadata>>>,
    current_output: Option<u32>,
    source_handle: obs::source::SourceHandle,
    video_thread: Option<VideoThread>,
}

impl WlrSource {
    fn update_xdg_outputs(&mut self) {
        use wayland_client::NewProxy;

        let outputs = self.outputs.read().unwrap();
        self.xdg_outputs = LinkedList::new();
        for (&id, output) in outputs.iter() {
            let output_metadata = self.output_metadata.clone();
            let xdg_output = self.output_manager.get_xdg_output(output, move |output: NewProxy<_>| {
                let output_metadata = output_metadata.clone();
                output.implement_closure(move |event, _proxy| {
                    let output_metadata = output_metadata.clone();
                    let mut output_metadata = output_metadata.write().unwrap();
                    match event {
                        zxdg_output_v1::Event::Name { name } => {
                            output_metadata.insert(id, OutputMetadata {
                                name: name,
                            });
                        },
                        _ => {},
                    }
                }, ())
            }).expect("Error creating xdg output interface");
            self.xdg_outputs.push_back(xdg_output);
        }
        self.display_events.sync_roundtrip()
            .expect("Error waiting on display events");
    }

    pub fn get_current_output(&self) -> Option<wl_output::WlOutput> {
        let outputs = self.outputs.read().unwrap();
        self.current_output
            .as_ref()
            .and_then(|id| outputs.get(id))
            .map(Clone::clone)
    }
}

impl obs::source::Source for WlrSource {
    const ID: &'static [u8] = SOURCE_INFO_ID;
    const NAME: &'static [u8] = SOURCE_NAME;
    fn create(settings: &mut obs_sys::obs_data_t, source: &mut obs_sys::obs_source_t) -> Result<WlrSource, String> {
        use obs::data::ObsData;
        use wayland_client::{Display, GlobalManager};

        let (display, mut display_events) = settings.get_str("display")
            .map(|name| name.into_owned())
            .filter(|name| name.len() != 0)
            .map(|display_name| Display::connect_to_name(display_name))
            .unwrap_or_else(|| Display::connect_to_env())
            .map_err(|e| format!("Error connecting to wayland display: {}", e))?;

        let outputs = Arc::new(RwLock::new(BTreeMap::new()));
        let outputs_clone = outputs.clone();

        let mut _status = display_events.sync_roundtrip()
            .map_err(|e| format!("Error waiting on display events: {}", e))?;

        let global_manager = GlobalManager::new_with_cb(&display, move |event, registry| {
            use wayland_client::{GlobalEvent, Interface, NewProxy};
            let mut outputs = outputs.write().unwrap();
            match event {
                GlobalEvent::New { id, interface, version } => {
                    match interface.as_ref() {
                        <wl_output::WlOutput as Interface>::NAME => {
                            let output = registry.bind::<wl_output::WlOutput, _>(version, id, |output: NewProxy<_>| output.implement_dummy())
                                .expect("Error binding output interface");
                            outputs.insert(id, output);
                        },
                        _ => {},
                    }
                },
                GlobalEvent::Removed { id, .. } => {
                    outputs.remove(&id);
                },
            }
        });

        let _status = display_events.sync_roundtrip()
            .map_err(|e| format!("Error waiting on display events: {}", e))?;

        let shm = global_manager.instantiate_exact::<wl_shm::WlShm, _>(1, |shm| shm.implement_dummy())
            .map_err(|e| format!("Error creating shm interface: {}", e))?;
        let output_manager = global_manager.instantiate_exact::<zxdg_output_manager_v1::ZxdgOutputManagerV1, _>(2, |mgr| mgr.implement_dummy())
            .map_err(|e| format!("Error creating output manager interface: {}", e))?;
        let screencopy_manager = global_manager.instantiate_exact::<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, _>(1, |mgr| mgr.implement_dummy())
            .map_err(|e| format!("Error creating screencopy manager interface: {}", e))?;

        for (id, interface, version) in global_manager.list() {
            println!("{}: {} (version {})", id, interface, version);
        }

        let mut ret = WlrSource {
            display: Arc::new(display),
            display_events: display_events,
            shm: shm,
            output_manager: output_manager,
            screencopy_manager: screencopy_manager,
            outputs: outputs_clone,
            xdg_outputs: LinkedList::new(),
            output_metadata: Arc::new(RwLock::new(BTreeMap::new())),
            current_output: None,
            source_handle: obs::source::SourceHandle::new(source as *mut obs_sys::obs_source_t),
            video_thread: None,
        };

        ret.update_xdg_outputs();

        {
            let output_metadata = ret.output_metadata.read().unwrap();
            for (&id, metadata) in output_metadata.iter() {
                println!("output {}: {}", id, &metadata.name);
            }
        }
        ret.update(settings);

        Ok(ret)
    }

    fn update(&mut self, settings: &mut obs_sys::obs_data_t) {
        use obs::data::ObsData;

        println!("obs_wlroots: update(output = {:?})", settings.get_string("output"));

        let output_metadata = self.output_metadata.read().unwrap();
        let outputs = self.outputs.read().unwrap();

        self.current_output = output_metadata.iter()
            .find(|&(_id, meta)| meta.name == settings.get_str("output").unwrap_or(Cow::Borrowed(meta.name.as_ref())))
            .map(|(&id, _meta)| id)
            .and_then(move |id| outputs.get(&id).map(|_| id));

        println!("obs_wlroots: update(current_output = {:?})", &self.current_output);

        self.video_thread = None;
        self.video_thread = self.current_output
            .map(|id| VideoThread::new(self.source_handle, id, true, self.display.clone()));
    }

    fn get_properties(&mut self) -> obs::Properties {
        use obs::Properties;
        use obs::properties::PropertyList;

        let mut props = obs::Properties::new();
        let mut output_list = props.add_string_list("output", "Output");

        let output_metadata = self.output_metadata.read().unwrap();
        for (&id, ref metadata) in output_metadata.iter() {
            output_list.add_item(&metadata.name, &metadata.name);
        }
        props
    }
}

impl obs::source::AsyncVideoSource for WlrSource {}

// impl obs::source::VideoSource for WlrSource {
//     fn width(&self) -> u32 {
//         self.current_frame.as_ref()
//             .map(|f| f.0.lock().unwrap())
//             .map(|f| f.width)
//             .unwrap_or(0)
//     }
// 
//     fn height(&self) -> u32 {
//         self.current_frame.as_ref()
//             .map(|f| f.0.lock().unwrap())
//             .map(|f| f.height)
//             .unwrap_or(0)
//     }
// 
//     fn render(&mut self) {
//         use wayland_client::NewProxy;
// 
//         let current_output = self.get_current_output();
// 
//         if current_output.is_none() {
//             return;
//         }
//         let current_output = current_output.unwrap();
// 
//         println!("obs_wlroots: render");
// 
//         let c = Container::new(WlrFrame::new(self.shm.clone()));
//         self.current_frame = Some(c.clone());
//         let frame = self.screencopy_manager.capture_output(1, &current_output, move |f: NewProxy<_>| {
//             f.implement_threadsafe(c, ())
//         }).expect("error creating frame");
//         let mut waiting = true;
//         while waiting {
//             println!("obs_wlroots: render(waiting)");
//             self.display_events.sync_roundtrip()
//                 .expect("Error waiting on display events");
//             {
//                 let current_frame = self.current_frame.as_ref().unwrap().0.lock().unwrap();
//                 waiting = current_frame.waiting;
//             }
//         }
//     }
// }
