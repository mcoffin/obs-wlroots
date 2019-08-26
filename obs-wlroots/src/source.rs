use std::borrow::Cow;
use std::collections::{BTreeMap, LinkedList};
use std::sync::{Arc, RwLock};
use ::obs::sys as obs_sys;
use wayland_client::protocol::wl_output;
use wayland_client::protocol::wl_shm;
use wayland_protocols::unstable::xdg_output::v1::client::zxdg_output_manager_v1;
use wayland_protocols::unstable::xdg_output::v1::client::zxdg_output_v1;
use wayland_protocols::wlr::unstable::screencopy::v1::client::zwlr_screencopy_manager_v1;

const SOURCE_INFO_ID: &'static [u8] = b"obs_wlroots\0";
const SOURCE_NAME: &'static [u8] = b"wlroots output\0";

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

pub struct WlrSource {
    display: wayland_client::Display,
    display_events: wayland_client::EventQueue,
    shm: wl_shm::WlShm,
    output_manager: zxdg_output_manager_v1::ZxdgOutputManagerV1,
    screencopy_manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    outputs: Arc<RwLock<BTreeMap<u32, wl_output::WlOutput>>>,
    xdg_outputs: LinkedList<zxdg_output_v1::ZxdgOutputV1>,
    output_metadata: Arc<RwLock<BTreeMap<u32, OutputMetadata>>>,
    current_output: Option<u32>,
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
    fn create(settings: &mut obs_sys::obs_data_t, _source: &mut obs_sys::obs_source_t) -> Result<WlrSource, String> {
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
            display: display,
            display_events: display_events,
            shm: shm,
            output_manager: output_manager,
            screencopy_manager: screencopy_manager,
            outputs: outputs_clone,
            xdg_outputs: LinkedList::new(),
            output_metadata: Arc::new(RwLock::new(BTreeMap::new())),
            current_output: None,
        };

        ret.update_xdg_outputs();

        {
            let output_metadata = ret.output_metadata.read().unwrap();
            for (&id, metadata) in output_metadata.iter() {
                println!("output {}: {}", id, &metadata.name);
            }
        }
        let current_output_id = {
            let outputs = ret.outputs.read().unwrap();
            outputs.keys().next().map(|&id| id)
        };
        ret.current_output = current_output_id;

        Ok(ret)
    }

    fn update(&mut self, settings: &mut obs_sys::obs_data_t) {
        use obs::data::ObsData;

        let output_metadata = self.output_metadata.read().unwrap();

        self.current_output = output_metadata.iter()
            .find(|&(_id, meta)| meta.name == settings.get_str("output").unwrap_or(Cow::Borrowed(meta.name.as_ref())))
            .map(|(&id, _meta)| id);
    }
}

impl obs::source::VideoSource for WlrSource {
    fn width(&self) -> u32 {
        // TODO: implement
        0
    }

    fn height(&self) -> u32 {
        // TODO: implement
        0
    }
}
