use std::cell::Cell;
use std::collections::BTreeMap;
use std::mem;
use std::sync::{
    Arc,
    RwLock,
    Mutex
};
use std::sync::atomic::{self, AtomicBool, AtomicU32};
use std::thread;
use std::time;
use ::obs::sys as obs_sys;
use wayland_client::{
    Attached,
    Display,
    EventQueue,
    GlobalManager,
    GlobalEvent,
    Interface,
    Main
};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_protocols::unstable::xdg_output::v1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::unstable::xdg_output::v1::client::zxdg_output_v1;
use wayland_protocols::wlr::unstable::screencopy::v1::client::zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1};
use wayland_protocols::wlr::unstable::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;
use crate::shm::{self, ShmFd};

pub struct WlrSource {
    display: Arc<Display>,
    display_events: EventQueue,
    wl_outputs: Arc<RwLock<BTreeMap<u32, Arc<Main<WlOutput>>>>>,
    outputs: BTreeMap<u32, Arc<RwLock<WlrOutput>>>,
    output_manager: Main<ZxdgOutputManagerV1>,
    video_thread: Option<VideoThread>,
    source_handle: obs::source::SourceHandle,
}

impl WlrSource {
    fn update_xdg(&mut self) {
        for (&id, ref wl_output) in self.wl_outputs.read().unwrap().iter() {
            self.outputs.remove(&id);
            self.outputs.insert(id, WlrOutput::new(wl_output, id, &self.output_manager));
        }
        self.display_events.sync_roundtrip(|_, _| {})
            .expect("Error waiting on display events");
    }
}

impl obs::source::Source for WlrSource {
    const ID: &'static [u8] = b"obs_wlroots\0";
    const NAME: &'static [u8] = b"wlroots capture\0";

    fn create(settings: &mut obs_sys::obs_data_t, source: &mut obs_sys::obs_source_t) -> Result<WlrSource, String> {
        use obs::data::ObsData;

        let display = settings.get_str("display")
            .map(|name| name.into_owned())
            .filter(|name| name.len() != 0)
            .map(|display_name| Display::connect_to_name(display_name))
            .unwrap_or_else(Display::connect_to_env)
            .map_err(|e| format!("Error connecting to wayland display: {}", e))?;
        let mut display_events = display.create_event_queue();
        let source_display = (*display).clone().attach(display_events.get_token());

        let outputs: Arc<RwLock<BTreeMap<u32, Arc<Main<WlOutput>>>>> = Arc::new(RwLock::new(BTreeMap::new()));

        let gm_outputs = outputs.clone();
        let global_manager = GlobalManager::new_with_cb(&source_display, move |evt, registry| {
            let mut outputs = gm_outputs.write().unwrap();
            match evt {
                GlobalEvent::New { id, interface, version } => {
                    match interface.as_ref() {
                        <WlOutput as Interface>::NAME => {
                            let output = registry.bind::<WlOutput>(version, id);
                            outputs.insert(id, Arc::new(output));
                        },
                        _ => {},
                    }
                },
                GlobalEvent::Removed { id, .. } => {
                    outputs.remove(&id);
                },
            }
        });
        display_events.sync_roundtrip(|_, _| {})
            .map_err(|e| format!("Error waiting on display events: {}", e))?;
        let output_manager = global_manager.instantiate_exact::<ZxdgOutputManagerV1>(2)
            .map_err(|e| format!("Error instantiating {}: {}", <ZxdgOutputManagerV1 as Interface>::NAME, e))?;
        display_events.sync_roundtrip(|_, _| {})
            .map_err(|e| format!("Error waiting on display events: {}", e))?;
        
        let mut ret = WlrSource {
            display: Arc::new(display),
            display_events: display_events,
            wl_outputs: outputs,
            output_manager: output_manager,
            outputs: BTreeMap::new(),
            video_thread: None,
            source_handle: obs::source::SourceHandle::new(source as *mut obs_sys::obs_source_t),
        };
        ret.update_xdg();
        ret.update(settings);
        Ok(ret)
    }

    fn update(&mut self, settings: &mut obs_sys::obs_data_t) {
        use std::borrow::Cow;
        use obs::data::ObsData;

        let current_output = self.outputs.iter()
            .map(|(_, output)| output.read().unwrap())
            .find(|output| {
                output.name() == settings.get_str("output").unwrap_or(Cow::Borrowed(""))
            })
            .map(|output| output.handle.clone());
        self.video_thread = current_output.map(|handle| VideoThread::new(handle, self.source_handle, self.display.clone(), time::Duration::from_millis(5)));
    }

    fn get_properties(&mut self) -> obs::Properties {
        use obs::properties::PropertyList;

        let mut props = obs::Properties::new();
        let mut output_list = props.add_string_list("output", "Output");

        for (_, ref output) in self.outputs.iter() {
            let output = output.read().unwrap();
            let name = output.name();
            output_list.add_item(name, name);
        }

        props
    }
}

impl obs::source::AsyncVideoSource for WlrSource {}

pub struct VideoThread {
    thread: Option<thread::JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl VideoThread {
    fn new(output: WlOutput, source_handle: obs::source::SourceHandle, display: Arc<Display>, sleep_duration: time::Duration) -> VideoThread {
        let running = Arc::new(AtomicBool::new(true));
        let running_ret = running.clone();
        let t = thread::spawn(move || {
            let mut events = display.create_event_queue();
            let video_display = (**display).clone().attach(events.get_token());
            let global_manager = GlobalManager::new(&video_display);
            events.sync_roundtrip(|_, _| {})
                .expect("Error waiting on display events");
            let screencopy_manager = global_manager.instantiate_exact::<ZwlrScreencopyManagerV1>(1)
                .expect(&format!("Error instantiating {}", <ZwlrScreencopyManagerV1 as Interface>::NAME));
            let shm = global_manager.instantiate_exact::<WlShm>(1)
                .expect(&format!("Error instantiating {}", <WlShm as Interface>::NAME));
            events.sync_roundtrip(|_, _| {})
                .expect("Error waiting on display events");
            let frame = WlrFrame::new((*shm).clone(), source_handle);
            let mut start = time::Instant::now();
            let mut frame_count = 0u64;
            while running.load(atomic::Ordering::Relaxed) {

                if WlrFrame::handle_output(&frame, &screencopy_manager, &output) {
                    frame_count = frame_count + 1;
                }
                events.sync_roundtrip(|_, _| {})
                    .expect("Error waiting on display events");
                if (start.elapsed().as_millis() > 1000) {
                    println!("obs_wlroots: fps = {}", frame_count);
                    start = time::Instant::now();
                    frame_count = 0;
                }
                thread::sleep(sleep_duration);
            }
            println!("obs_wlroots: VideoThread: done");
        });
        VideoThread {
            thread: Some(t),
            running: running_ret,
        }
    }
}

impl Drop for VideoThread {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            self.running.store(false, atomic::Ordering::Relaxed);
            t.join().unwrap();
        }
    }
}

const WLR_FRAME_SHM_PATH: &'static str = "/obs_wlroots";

struct WlrBuffer {
    pool: Main<WlShmPool>,
    buffer: Main<WlBuffer>,
    fd: ShmFd<&'static str>,
    size: usize,
}

impl WlrBuffer {
    fn new(shm: &Attached<WlShm>, frame: &WlrFrame) -> WlrBuffer {
        let size = frame.size();
        let fd = ShmFd::open(WLR_FRAME_SHM_PATH, libc::O_CREAT | libc::O_RDWR, 0).unwrap();
        unsafe {
            shm::unlink(WLR_FRAME_SHM_PATH);
            libc::ftruncate(fd.as_raw(), size as libc::off_t);
        }
        let pool  = shm.create_pool(fd.as_raw(), size as i32);
        let buffer = pool.create_buffer(0, frame.width.get() as i32, frame.height.get() as i32, frame.stride.get() as i32, frame.buffer_format().unwrap());
        WlrBuffer {
            pool: pool,
            buffer: buffer,
            fd: fd,
            size: size,
        }
    }

    #[inline(always)]
    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for WlrBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

struct WlrFrame {
    format: Cell<u32>,
    width: Cell<u32>,
    height: Cell<u32>,
    stride: Cell<u32>,
    buffer: Mutex<Option<WlrBuffer>>,
    shm: Attached<WlShm>,
    waiting: AtomicBool,
    source_handle: obs::source::SourceHandle,
}

impl WlrFrame {
    pub fn new(shm: Attached<WlShm>, source_handle: obs::source::SourceHandle) -> Arc<WlrFrame> {
        Arc::new(WlrFrame {
            format: Cell::new(0),
            width: Cell::new(0),
            height: Cell::new(0),
            stride: Cell::new(0),
            buffer: Mutex::new(None),
            shm: shm,
            waiting: AtomicBool::new(false),
            source_handle: source_handle
        })
    }

    pub fn handle_output(s: &Arc<WlrFrame>, screencopy_manager: &ZwlrScreencopyManagerV1, output: &WlOutput) -> bool {
        if !s.waiting.compare_and_swap(false, true, atomic::Ordering::AcqRel) {
            let handler = s.clone();
            let frame = screencopy_manager.capture_output(1, output);
            frame.assign_mono(move |obj, evt| handler.handle_frame_event(&obj, evt));
            return true;
        }
        false
    }

    fn handle_frame_event(&self, frame: &ZwlrScreencopyFrameV1, event: zwlr_screencopy_frame_v1::Event) {
        use zwlr_screencopy_frame_v1::Event;
        match event {
            Event::Buffer { format, width, height, stride } => {
                self.format.set(format);
                self.width.set(width);
                self.height.set(height);
                self.stride.set(stride);
                let mut buffer = self.buffer.lock().unwrap();
                let buffer_size = buffer.as_ref().map(WlrBuffer::size);
                if buffer_size.is_none() || buffer_size != Some(self.size()) {
                    println!("obs_wlroots: re-creating buffer");
                    *buffer = Some(WlrBuffer::new(&self.shm, self));
                }
                frame.copy(&buffer.as_ref().unwrap().buffer);
            },
            Event::Ready { .. } => {
                use std::ptr;
                use crate::mmap::MappedMemory;
                let buffer = self.buffer.lock().unwrap();
                let buffer = buffer.as_ref().unwrap();
                let buf = unsafe {
                    MappedMemory::new(buffer.size(), libc::PROT_READ, libc::MAP_SHARED, buffer.fd.as_raw(), 0)
                        .unwrap()
                };
                let mut source_frame = obs_sys::obs_source_frame {
                    data: [ptr::null_mut(); 8],
                    linesize: [0; 8],
                    width: self.width.get(),
                    height: self.height.get(),
                    format: obs_sys::video_format_VIDEO_FORMAT_BGRA,
                    flip: true,

                    timestamp: 0,
                    color_matrix: [0f32; 16],
                    full_range: false,
                    color_range_min: [0f32; 3],
                    color_range_max: [0f32; 3],
                    refs: 0,
                    prev_frame: false
                };
                source_frame.data[0] = unsafe { mem::transmute(buf.as_raw()) };
                source_frame.linesize[0] = self.stride.get();

                unsafe {
                    obs_sys::obs_source_output_video(self.source_handle.as_raw(), &source_frame);
                }

                self.waiting.store(false, atomic::Ordering::Relaxed);
                frame.destroy();
            },
            Event::Failed => {
                self.waiting.store(false, atomic::Ordering::Relaxed);
                frame.destroy();
            },
            _ => {},
        }
    }

    fn size(&self) -> usize {
        (self.height.get() as usize) * (self.stride.get() as usize)
    }

    #[inline(always)]
    fn buffer_format(&self) -> Option<wl_shm::Format> {
        wl_shm::Format::from_raw(self.format.get())
    }
}

pub struct WlrOutput {
    handle: WlOutput,
    id: u32,
    name: Option<String>,
}

impl WlrOutput {
    pub fn new(handle: &WlOutput, id: u32, output_manager: &ZxdgOutputManagerV1) -> Arc<RwLock<WlrOutput>> {
        let xdg_output = output_manager.get_xdg_output(&handle);
        let ret = Arc::new(RwLock::new(WlrOutput {
            handle: handle.clone(),
            id: id,
            name: None,
        }));
        let output = ret.clone();
        xdg_output.assign_mono(move |handle, evt| {
            match evt {
                zxdg_output_v1::Event::Name { name } => {
                    let mut output = output.write().unwrap();
                    output.name = Some(name);
                },
                zxdg_output_v1::Event::Done => {
                    handle.destroy();
                },
                _ => {},
            }
        });
        ret
    }

    fn name(&self) -> &str {
        self.name.as_ref()
            .map(|s| s.as_ref())
            .unwrap_or("<unknown>")
    }
}
