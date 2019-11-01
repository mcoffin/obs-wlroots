use std::cell::Cell;
use std::collections::BTreeMap;
use std::mem;
use std::sync::{
    Arc,
    RwLock,
    Mutex,
    mpsc,
};
use std::sync::atomic::{self, AtomicBool};
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
use crate::shm::ShmFd;
use crate::mmap::MappedMemory;

pub struct WlrSource {
    outputs: Arc<RwLock<BTreeMap<String, WlrOutput>>>,
    source_handle: obs::source::SourceHandle,
    output_thread: Option<thread::JoinHandle<()>>,
    output_running: Arc<AtomicBool>,
    update_thread: Option<thread::JoinHandle<()>>,
    update_sender: mpsc::SyncSender<Option<obs::data::Data>>,
    update_running: Arc<AtomicBool>,
    video_receiver: Option<mpsc::Receiver<FrameData>>,
    should_render: Arc<AtomicBool>,
    last_width: u32,
    last_height: u32,
}

impl Drop for WlrSource {
    fn drop(&mut self) {
        println!("obs_wlroots: WlrSource::drop");
        self.output_running.store(false, atomic::Ordering::Relaxed);
        self.output_thread.take().map(|t| t.join());
        println!("obs_wlroots: WlrSource::drop: done with output thread!");
        self.update_running.store(false, atomic::Ordering::Relaxed);
        self.update_sender.send(None).unwrap();
        mem::drop(self.video_receiver.take());
        self.update_thread.take().map(|t| t.join());
        println!("obs_wlroots: WlrSource::drop: done with update thread!");
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
        let display = Arc::new(display);
        let xdg_outputs: Arc<RwLock<BTreeMap<String, WlrOutput>>> = Arc::new(RwLock::new(BTreeMap::new()));
        let output_running = Arc::new(AtomicBool::new(true));
        let (update_sender, update_receiver) = mpsc::sync_channel::<Option<obs::data::Data>>(1);
        let source_handle = obs::source::SourceHandle::new(source as *mut obs_sys::obs_source_t);
        let (video_sender, video_receiver) = mpsc::sync_channel(1);
        let should_render = Arc::new(AtomicBool::new(false));
        let update_running = Arc::new(AtomicBool::new(true));
        let update_thread = {
            let display = display.clone();
            let xdg_outputs = xdg_outputs.clone();
            let should_render = should_render.clone();
            let builder = thread::Builder::new()
                .name("obs-wlroots:update".into());
            let update_running = update_running.clone();
            builder.spawn(move || {
                use std::borrow::Cow;

                let display = display;
                let mut video_thread: Option<VideoThread> = None;
                let mut last_settings: Option<obs::data::Data> = None;
                while let Ok(settings) = update_receiver.recv() {
                    if !update_running.load(atomic::Ordering::Relaxed) {
                        break;
                    }
                    println!("obs_wlroots: update");
                    if settings.is_some() {
                        last_settings = settings;
                    }
                    if let Some(settings) = last_settings.as_ref() {
                        let current_output = {
                            let xdg_outputs = xdg_outputs.read().unwrap();
                            xdg_outputs.get(settings.get_str("output").unwrap_or(Cow::Borrowed("")).as_ref())
                                .map(|output| output.handle.clone())
                        };
                        mem::drop(video_thread.take());
                        video_thread = current_output.map(|handle| {
                            VideoThread::new(handle, source_handle, display.clone(), video_sender.clone())
                        });
                    }
                    should_render.store(video_thread.is_some(), atomic::Ordering::Relaxed);
                }
                println!("obs_wlroots: update_thread done! (dropping VideoThread)");
                mem::drop(video_thread.take());
            }).unwrap()
        };
        let output_thread = {
            let display = display.clone();
            let xdg_outputs = xdg_outputs.clone();
            let output_running = output_running.clone();
            let update_sender = update_sender.clone();
            let builder = thread::Builder::new()
                .name("obs-wlroots:output".into());
            let obs_thread = thread::current();
            builder.spawn(move || {
                use std::rc::Rc;
                use std::cell::RefCell;
                let update_sender = update_sender;
                let mut output_events = display.create_event_queue();
                let output_display = (**display).clone().attach(output_events.get_token());
                let output_manager: Rc<RefCell<Option<Attached<ZxdgOutputManagerV1>>>> = Rc::new(RefCell::new(None));
                let gm_output_manager = output_manager.clone();
                let tmp_xdg_outputs = xdg_outputs.clone();
                let gm_update_sender = update_sender.clone();
                let global_manager = GlobalManager::new_with_cb(&output_display, move |evt, registry| {
                    let output_manager = gm_output_manager.borrow();
                    if let Some(output_manager) = output_manager.as_ref() {
                        match evt {
                            GlobalEvent::New { id, interface, version } => {
                                if &interface == <WlOutput as Interface>::NAME {
                                    let output = registry.bind::<WlOutput>(version, id);
                                    WlrOutput::new(id, &output, output_manager, &xdg_outputs, gm_update_sender.clone());
                                }
                            },
                            GlobalEvent::Removed { id, interface } => {
                                if &interface == <WlOutput as Interface>::NAME {
                                    let mut outputs = xdg_outputs.write().unwrap();
                                    let maybe_name = outputs.iter()
                                        .find(|&(_name, output)| output.id() == id)
                                        .map(|(name, _)| name.clone());
                                    if let Some(name) = maybe_name {
                                        outputs.remove(&name);
                                        mem::drop(outputs);
                                        gm_update_sender.send(None).unwrap();
                                    }
                                }
                            },
                        }
                    }
                });
                output_events.sync_roundtrip(|_, _| {})
                    .expect("Error waiting on events");
                {
                    let mut output_manager = output_manager.borrow_mut();
                    *output_manager = global_manager.instantiate_exact::<ZxdgOutputManagerV1>(2)
                        .map(|gm| (*gm).clone())
                        .ok();
                    for (id, interface, version) in global_manager.list() {
                        if &interface == <WlOutput as Interface>::NAME {
                            let output = output_display.get_registry().bind::<WlOutput>(version, id);
                            WlrOutput::new(id, &output, output_manager.as_ref().unwrap(), &tmp_xdg_outputs, update_sender.clone());
                        }
                    }
                }
                mem::drop(tmp_xdg_outputs);
                obs_thread.unpark();
                while output_running.load(atomic::Ordering::Relaxed) {
                    output_events.sync_roundtrip(|_, _| {})
                        .map(|_| {})
                        .unwrap_or_else(|_| output_running.store(false, atomic::Ordering::Relaxed));
                }
                println!("obs_wlroots: output_thread done!");
            }).unwrap()
        };
        thread::park();
        let mut ret = WlrSource {
            outputs: xdg_outputs,
            source_handle: source_handle,
            output_thread: Some(output_thread),
            output_running: output_running,
            update_thread: Some(update_thread),
            update_sender: update_sender,
            update_running: update_running,
            video_receiver: Some(video_receiver),
            should_render: should_render,
            last_width: 0,
            last_height: 0,
        };
        ret.update(settings);
        Ok(ret)
    }

    fn update(&mut self, settings: &mut obs_sys::obs_data_t) {
        self.update_sender.send(Some(obs::data::Data::new(settings))).unwrap();
    }

    fn get_properties(&mut self) -> obs::Properties {
        use obs::properties::PropertyList;

        let mut props = obs::Properties::new();
        let mut output_list = props.add_string_list("output", "Output");

        {
            let outputs = self.outputs.read().unwrap();
            for name in outputs.keys() {
                output_list.add_item(name, name);
            }
        }

        props
    }
}

impl obs::source::VideoSource for WlrSource {
    fn width(&self) -> u32 {
        self.last_width
    }

    fn height(&self) -> u32 {
        self.last_height
    }

    fn render(&mut self) {
        if self.should_render.load(atomic::Ordering::Relaxed) {
            if let Some(video_receiver) = self.video_receiver.as_mut() {
                let mut data = video_receiver.recv().unwrap();
                let source_frame = data.as_source_frame();
                self.last_width = source_frame.width;
                self.last_height = source_frame.height;
                let flip = source_frame.flip;
                let mut texture = obs::gs::Texture::from(source_frame);
                obs::source::obs_source_draw(&mut texture, 0, 0, 0, 0, flip);
            }
        }
    }
}

pub struct VideoThread {
    thread: Option<thread::JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

#[no_mangle]
fn obs_wlroots_video_thread_done(frame: &WlrFrame) {
    println!("obs_wlroots: VideoThread: frame status = {}", frame.waiting.load(atomic::Ordering::Relaxed));
    println!("obs_wlroots: VideoThread: done");
}

#[no_mangle]
fn obs_wlroots_create_event_queue(display: &Display) -> EventQueue {
    display.create_event_queue()
}

impl VideoThread {
    fn new(output: WlOutput, source_handle: obs::source::SourceHandle, display: Arc<Display>, sender: mpsc::SyncSender<FrameData>) -> VideoThread {
        let running = Arc::new(AtomicBool::new(true));
        let running_ret = running.clone();
        let builder = thread::Builder::new()
            .name("obs-wlroots".into());
        let t = builder.spawn(move || {
            println!("obs_wlroots: VideoThread: start");
            let mut events = obs_wlroots_create_event_queue(display.as_ref());
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
            let frame = WlrFrame::new((*shm).clone(), source_handle, sender);
            let mut start = time::Instant::now();
            let mut frame_count = 0u64;

            while running.load(atomic::Ordering::Relaxed) || frame.waiting.load(atomic::Ordering::Relaxed) {
                if WlrFrame::handle_output(&frame, &screencopy_manager, &output, &running) {
                    frame_count = frame_count + 1;
                }
                events.sync_roundtrip(|_, _| {})
                    .expect("Error waiting on display events");
                if start.elapsed().as_millis() > 1000 {
                    println!("obs_wlroots: fps = {}", frame_count);
                    start = time::Instant::now();
                    frame_count = 0;
                }
            }
            events.sync_roundtrip(|_, _| {})
                .expect("Error waiting on disply events");
            obs_wlroots_video_thread_done(&frame);
            mem::drop(frame);
            mem::drop(events);
            mem::drop(running);
        }).unwrap();
        VideoThread {
            thread: Some(t),
            running: running_ret,
        }
    }
}

impl Drop for VideoThread {
    fn drop(&mut self) {
        println!("obs_wlroots: VideoThread::drop");
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
        let mut fd = ShmFd::open(WLR_FRAME_SHM_PATH, libc::O_CREAT | libc::O_RDWR, 0).unwrap();
        fd.unlink()
            .expect("error unlinking ShmFd");
        fd.truncate(size as libc::off_t)
            .expect("error truncating ShmFd");
        let pool  = shm.create_pool(fd.as_raw(), size as i32);
        let frame_meta = frame.metadata.get();
        let buffer = pool.create_buffer(0, frame_meta.width as i32, frame_meta.height as i32, frame_meta.stride as i32, frame.buffer_format().unwrap());
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
        println!("obs_wlroots: WlrBuffer::drop");
        self.buffer.destroy();
        self.pool.destroy();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FrameMetadata {
    format: u32,
    width: u32,
    height: u32,
    stride: u32,
}

impl FrameMetadata {
    pub fn new(format: u32, width: u32, height: u32, stride: u32) -> FrameMetadata {
        FrameMetadata {
            format: format,
            width: width,
            height: height,
            stride: stride,
        }
    }

    #[inline(always)]
    pub fn size(&self) -> usize {
        (self.height as usize) * (self.stride as usize)
    }
}

impl Default for FrameMetadata {
    fn default() -> Self {
        FrameMetadata {
            format: 0,
            width: 0,
            height: 0,
            stride: 0,
        }
    }
}

struct WlrFrame {
    sender: mpsc::SyncSender<FrameData>,
    metadata: Cell<FrameMetadata>,
    buffer: Mutex<Option<WlrBuffer>>,
    shm: Attached<WlShm>,
    waiting: AtomicBool,
    _source_handle: obs::source::SourceHandle,
}

impl WlrFrame {
    pub fn new(shm: Attached<WlShm>, source_handle: obs::source::SourceHandle, sender: mpsc::SyncSender<FrameData>) -> Arc<WlrFrame> {
        Arc::new(WlrFrame {
            sender: sender,
            metadata: Cell::new(FrameMetadata::default()),
            buffer: Mutex::new(None),
            shm: shm,
            waiting: AtomicBool::new(false),
            _source_handle: source_handle
        })
    }

    pub fn handle_output(s: &Arc<WlrFrame>, screencopy_manager: &ZwlrScreencopyManagerV1, output: &WlOutput, running: &Arc<AtomicBool>) -> bool {
        if !s.waiting.compare_and_swap(false, true, atomic::Ordering::AcqRel) {
            let handler = s.clone();
            let running = running.clone();
            let frame = screencopy_manager.capture_output(1, output);
            frame.assign_mono(move |obj, evt| handler.handle_frame_event(&obj, evt, running.clone()));
            return true;
        }
        false
    }

    fn handle_frame_event(&self, frame: &ZwlrScreencopyFrameV1, event: zwlr_screencopy_frame_v1::Event, running: Arc<AtomicBool>) {
        use zwlr_screencopy_frame_v1::Event;
        match event {
            Event::Buffer { format, width, height, stride } => {
                self.metadata.set(FrameMetadata::new(format, width, height, stride));
                let mut buffer = self.buffer.lock().unwrap();
                let buffer_size = buffer.as_ref().map(WlrBuffer::size);
                if buffer_size.is_none() || buffer_size != Some(self.size()) {
                    println!("obs_wlroots: re-creating buffer: had_previous = {}", !buffer_size.is_none());
                    *buffer = Some(WlrBuffer::new(&self.shm, self));
                }
                frame.copy(&buffer.as_ref().unwrap().buffer);
            },
            Event::Ready { .. } => {
                let buffer = self.buffer.lock().unwrap();
                let buffer = buffer.as_ref().unwrap();
                let buf = unsafe {
                    MappedMemory::new(buffer.size(), libc::PROT_READ, libc::MAP_SHARED, buffer.fd.as_raw(), 0)
                        .unwrap()
                };
                let meta = self.metadata.get();
                // unsafe {
                //     obs_sys::obs_source_output_video(self.source_handle.as_raw(), &source_frame);
                // }
                self.sender.send(unsafe { FrameData::new(buf, &meta) })
                    .unwrap_or_else(|_| {
                        println!("obs_wlroots: obs is probably shutting down.");
                        running.store(false, atomic::Ordering::Relaxed);
                    });

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
        let meta = self.metadata.get();
        meta.size()
    }

    #[inline(always)]
    fn buffer_format(&self) -> Option<wl_shm::Format> {
        wl_shm::Format::from_raw(self.metadata.get().format)
    }
}

impl Drop for WlrFrame {
    fn drop(&mut self) {
        println!("obs_wlroots: WlrFrame::drop");
    }
}

#[derive(Clone)]
pub struct WlrOutput {
    id: u32,
    handle: WlOutput,
    name: Option<String>,
}

impl WlrOutput {
    pub fn new(id: u32, handle: &WlOutput, output_manager: &ZxdgOutputManagerV1, outputs: &Arc<RwLock<BTreeMap<String, WlrOutput>>>, sender: mpsc::SyncSender<Option<obs::data::Data>>) {
        let xdg_output = output_manager.get_xdg_output(&handle);
        let mut obj = WlrOutput {
            id: id,
            handle: handle.clone(),
            name: None,
        };
        let outputs = outputs.clone();
        xdg_output.assign_mono(move |handle, evt| {
            match evt {
                zxdg_output_v1::Event::Name { name } => {
                    obj.name = Some(name);
                },
                zxdg_output_v1::Event::Done => {
                    {
                        let mut outputs = outputs.write().unwrap();
                        outputs.insert(obj.name().to_string(), obj.clone());
                        sender.send(None).unwrap();
                    }
                    handle.destroy();
                },
                _ => {},
            }
        });
    }

    #[inline(always)]
    pub fn id(&self) -> u32 {
        self.id
    }

    fn name(&self) -> &str {
        self.name.as_ref()
            .map(|s| s.as_ref())
            .unwrap_or("<unknown>")
    }
}

struct FrameData {
    data: Vec<u8>,
    meta: FrameMetadata,
    source_frame: obs_sys::obs_source_frame,
}

impl FrameData {
    unsafe fn new(buf: MappedMemory, meta: &FrameMetadata) -> FrameData {
        use std::ptr;
        use std::slice;
        let mut source_frame = obs_sys::obs_source_frame {
            data: [ptr::null_mut(); 8],
            linesize: [0; 8],
            width: meta.width,
            height: meta.height,
            format: obs_sys::video_format::VIDEO_FORMAT_BGRA, // TODO: don't hard-code this
            flip: true,

            timestamp: 0,
            color_matrix: [0f32; 16],
            full_range: false,
            color_range_min: [0f32; 3],
            color_range_max: [0f32; 3],
            refs: 0,
            prev_frame: false
        };
        source_frame.linesize[0] = meta.stride;
        let buf = slice::from_raw_parts(mem::transmute(buf.as_raw()), meta.size());
        let mut buf = Vec::from(buf);
        source_frame.data[0] = buf.as_mut_ptr();
        FrameData {
            data: buf,
            meta: *meta,
            source_frame: source_frame
        }
    }

    #[inline(always)]
    fn as_source_frame(&mut self) -> &mut obs_sys::obs_source_frame {
        &mut self.source_frame
    }
}

unsafe impl Send for FrameData {}
