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
    display: Arc<Display>,
    display_events: EventQueue,
    wl_outputs: Arc<RwLock<BTreeMap<u32, Arc<Main<WlOutput>>>>>,
    outputs: BTreeMap<u32, Arc<RwLock<WlrOutput>>>,
    output_manager: Main<ZxdgOutputManagerV1>,
    video_thread: Option<VideoThread>,
    source_handle: obs::source::SourceHandle,
    last_width: u32,
    last_height: u32,
}

impl WlrSource {
    fn update_xdg(&mut self) {
        for (&id, ref wl_output) in self.wl_outputs.read().unwrap().iter() {
            self.outputs.remove(&id);
            self.outputs.insert(id, WlrOutput::new(wl_output, &self.output_manager));
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
            last_width: 0,
            last_height: 0,
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
        mem::drop(self.video_thread.take());
        self.video_thread = current_output.map(|handle| VideoThread::new(handle, self.source_handle, self.display.clone()));
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

impl obs::source::VideoSource for WlrSource {
    fn width(&self) -> u32 {
        self.last_width
    }

    fn height(&self) -> u32 {
        self.last_height
    }

    fn render(&mut self) {
        if let Some(video_thread) = self.video_thread.as_mut() {
            let unparked = video_thread.thread.as_ref().map(|t| t.thread().unpark());
            if unparked.is_some() {
                let FrameData(_mem, mut source_frame) = video_thread.receiver.recv().unwrap();
                self.last_width = source_frame.width;
                self.last_height = source_frame.height;
                let mut texture = obs::gs::Texture::from(&mut source_frame);
                obs::source::obs_source_draw(&mut texture, 0, 0, 0, 0, source_frame.flip);
            }
        }
    }
}

pub struct VideoThread {
    thread: Option<thread::JoinHandle<()>>,
    running: Arc<AtomicBool>,
    receiver: mpsc::Receiver<FrameData>,
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
    fn new(output: WlOutput, source_handle: obs::source::SourceHandle, display: Arc<Display>) -> VideoThread {
        let running = Arc::new(AtomicBool::new(true));
        let running_ret = running.clone();
        let builder = thread::Builder::new()
            .name("obs-wlroots".into());
        let (sender, receiver) = mpsc::sync_channel(1);
        let t = builder.spawn(move || {
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
                if !frame.waiting.load(atomic::Ordering::Relaxed) {
                    thread::park();
                }
                if WlrFrame::handle_output(&frame, &screencopy_manager, &output) {
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
            receiver: receiver,
        }
    }
}

impl Drop for VideoThread {
    fn drop(&mut self) {
        println!("obs_wlroots: VideoThread::drop");
        if let Some(t) = self.thread.take() {
            self.running.store(false, atomic::Ordering::Relaxed);
            t.thread().unpark();
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
    source_handle: obs::source::SourceHandle,
}

impl WlrFrame {
    pub fn new(shm: Attached<WlShm>, source_handle: obs::source::SourceHandle, sender: mpsc::SyncSender<FrameData>) -> Arc<WlrFrame> {
        Arc::new(WlrFrame {
            sender: sender,
            metadata: Cell::new(FrameMetadata::default()),
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
                self.sender.send(unsafe { FrameData::new(buf, &meta) }).unwrap();

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

pub struct WlrOutput {
    handle: WlOutput,
    name: Option<String>,
}

impl WlrOutput {
    pub fn new(handle: &WlOutput, output_manager: &ZxdgOutputManagerV1) -> Arc<RwLock<WlrOutput>> {
        let xdg_output = output_manager.get_xdg_output(&handle);
        let ret = Arc::new(RwLock::new(WlrOutput {
            handle: handle.clone(),
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

struct FrameData(MappedMemory, obs_sys::obs_source_frame);

impl FrameData {
    unsafe fn new(buf: MappedMemory, meta: &FrameMetadata) -> FrameData {
        use std::ptr;
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
        source_frame.data[0] = mem::transmute(buf.as_raw());
        source_frame.linesize[0] = meta.stride;
        FrameData(buf, source_frame)
    }
}

unsafe impl Send for FrameData {}
