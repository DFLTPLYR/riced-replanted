//! Screenshots through `zwlr_screencopy_manager_v1`.
//!
//! shortcut: wlroots-family compositors only (hyprland, sway, niri, wlroots);
//! the upgrade path for GNOME/KDE is `ext-image-copy-capture-v1`.
//! shortcut: a region must fit inside one output (wlr screencopy is per
//! output); spanning selections would need per-output stitching.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Cursor},
    os::unix::fs::{FileExt, OpenOptionsExt},
    os::unix::io::AsFd,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use image::{ImageBuffer, Rgba};
use riced_extension_sdk::write_atomic::write_atomic_bytes;
use riced_extension_sdk::{Plugin, expand_home, log};
use serde_json::{Value, json};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
    protocol::{
        wl_buffer,
        wl_output::{self, WlOutput},
        wl_registry::{self, WlRegistry},
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
    },
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

/// How long a capture may take before the caller gets a timeout; the capture
/// keeps running and still writes its file.
const WAIT: Duration = Duration::from_secs(30);

fn main() -> io::Result<()> {
    // The Context only exists inside `serve`, so handlers pick the shared
    // state up from here once the run closure has published it.
    let ready = Arc::new(Mutex::new(None::<Arc<Shared>>));
    let handler = Arc::clone(&ready);
    let region_handler = Arc::clone(&ready);
    Plugin::new("screenshot")
        .state(["connected", "capturing", "last"])
        .method("capture", false, move |args| {
            let (output, path) = arguments(args)?;
            let shared = handler
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| "extension is still starting".to_owned())?;
            capture(&shared, Target::Output(output), path)
        })
        .method("capture_region", false, move |args| {
            let (region, path) = region_arguments(args)?;
            let shared = region_handler
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| "extension is still starting".to_owned())?;
            capture(&shared, Target::Region(region), path)
        })
        .serve(move |context| {
            let shared = Arc::new(Shared {
                context,
                store: Mutex::new(Store {
                    connected: false,
                    capturing: false,
                    serial: 0,
                    last: Value::Null,
                }),
            });
            *ready.lock().unwrap() = Some(Arc::clone(&shared));
            publish(&shared);
            // Captures run on their own threads; this one just keeps the
            // process alive until riced disconnects (the SDK then exits).
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        })
}

struct Store {
    connected: bool,
    capturing: bool,
    // Monotonic id of the last capture; a timed-out worker must not clear the
    // flag for a capture started after it.
    serial: u64,
    last: Value,
}

struct Shared {
    context: riced_extension_sdk::Context,
    store: Mutex<Store>,
}

/// Publishes the whole snapshot; capture threads are the only writers.
fn publish(shared: &Shared) {
    let store = shared.store.lock().unwrap();
    shared.context.publish(json!({
        "connected": store.connected,
        "capturing": store.capturing,
        "last": store.last,
    }));
}

/// Parses `[output_name_or_null, path]`; a leading `~` expands against `$HOME`.
fn arguments(args: &Value) -> Result<(Option<String>, PathBuf), String> {
    let args = args
        .as_array()
        .filter(|args| args.len() == 2)
        .ok_or("capture requires [output_or_null, path]")?;
    let output = match &args[0] {
        Value::Null => None,
        Value::String(name) if !name.is_empty() => Some(name.clone()),
        _ => return Err("output must be null or a non-empty name".into()),
    };
    Ok((output, path_argument(&args[1])?))
}

/// Parses `[x, y, width, height, path]` in global logical coordinates.
fn region_arguments(args: &Value) -> Result<(Rect, PathBuf), String> {
    let args = args
        .as_array()
        .filter(|args| args.len() == 5)
        .ok_or("capture_region requires [x, y, width, height, path]")?;
    let number = |index: usize, what: &str| {
        args[index]
            .as_i64()
            .and_then(|number| i32::try_from(number).ok())
            .ok_or_else(|| format!("{what} must be a 32-bit integer"))
    };
    let region = Rect {
        x: number(0, "x")?,
        y: number(1, "y")?,
        width: number(2, "width")?,
        height: number(3, "height")?,
    };
    if region.width <= 0 || region.height <= 0 {
        return Err("width and height must be positive".into());
    }
    Ok((region, path_argument(&args[4])?))
}

/// Expands `~` and rejects empty paths and directories.
fn path_argument(value: &Value) -> Result<PathBuf, String> {
    let path = value
        .as_str()
        .filter(|path| !path.is_empty())
        .ok_or("path must be a non-empty string")?;
    let path = expand_home(path);
    if path.is_dir() {
        return Err(format!("{} is a directory", path.display()));
    }
    Ok(path)
}

/// What to capture: one whole output, or a rectangle inside one output.
#[derive(Clone)]
enum Target {
    Output(Option<String>),
    Region(Rect),
}

/// A screen rectangle in global logical coordinates.
#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Rect {
    fn contains(self, inner: Rect) -> bool {
        let (left, top) = (i64::from(self.x), i64::from(self.y));
        let (right, bottom) = (left + i64::from(self.width), top + i64::from(self.height));
        let (inner_left, inner_top) = (i64::from(inner.x), i64::from(inner.y));
        inner_left >= left
            && inner_top >= top
            && inner_left + i64::from(inner.width) <= right
            && inner_top + i64::from(inner.height) <= bottom
    }
}

/// Serializes captures so a second concurrent one is rejected immediately.
fn capture(shared: &Arc<Shared>, target: Target, path: PathBuf) -> Result<Value, String> {
    let serial = {
        let mut store = shared.store.lock().unwrap();
        if store.capturing {
            return Err("a capture is already in progress".into());
        }
        store.capturing = true;
        store.serial = store.serial.wrapping_add(1);
        store.serial
    };
    publish(shared);
    let (tx, rx) = mpsc::channel();
    let worker = {
        let shared = Arc::clone(shared);
        thread::Builder::new()
            .name("riced-screenshot".into())
            .spawn(move || {
                let result = shoot(&shared, &target, &path);
                {
                    let mut store = shared.store.lock().unwrap();
                    // A capture that timed out may finish after a newer one
                    // started; only its own generation may clear the flag.
                    if store.serial == serial {
                        store.capturing = false;
                    }
                    if let Ok(entry) = &result {
                        store.last = entry.clone();
                    }
                }
                publish(&shared);
                if let Err(error) = &result {
                    log::warn!(target: "screenshot.capture", "{error}");
                }
                let _ = tx.send(result);
            })
    };
    if let Err(error) = worker {
        shared.store.lock().unwrap().capturing = false;
        publish(shared);
        return Err(format!("cannot start a capture thread: {error}"));
    }
    match rx.recv_timeout(WAIT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // The worker is stuck in the compositor; let the caller try again.
            // A late result still records `last`, but cannot clear a newer flag.
            let mut store = shared.store.lock().unwrap();
            if store.serial == serial {
                store.capturing = false;
            }
            drop(store);
            publish(shared);
            Err(format!(
                "capture timed out after {}s and continues in the background",
                WAIT.as_secs()
            ))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("capture thread crashed".into()),
    }
}

/// One capture: connect, bind, copy a frame into shm, encode, write.
fn shoot(shared: &Shared, target: &Target, path: &Path) -> Result<Value, String> {
    let connection = Connection::connect_to_env()
        .map_err(|error| format!("cannot connect to the Wayland display: {error}"))?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    let mut state = Shot {
        _registry: Some(connection.display().get_registry(&qh, ())),
        ..Default::default()
    };
    queue
        .roundtrip(&mut state)
        .map_err(|error| format!("wayland registry roundtrip failed: {error}"))?;
    // xdg-output gives each output's layout position and size, which maps a
    // global region onto one output.
    if let Some(manager) = state.xdg_manager.clone() {
        for output in &mut state.outputs {
            output._xdg = Some(manager.get_xdg_output(&output.output, &qh, output.global));
        }
        queue
            .roundtrip(&mut state)
            .map_err(|error| format!("wayland output layout roundtrip failed: {error}"))?;
    }
    log::debug!(
        target: "screenshot.registry",
        "bound {} output(s), screencopy manager {}",
        state.outputs.len(),
        if state.manager.is_some() { "yes" } else { "no" },
    );
    shared.store.lock().unwrap().connected = state.manager.is_some();
    publish(shared);

    let manager = state.manager.clone().ok_or(
        "compositor does not support zwlr_screencopy_manager_v1 (wlroots-family compositors only)",
    )?;
    let names: Vec<String> = state.outputs.iter().map(Output::name).collect();
    let (index, region) = match target {
        Target::Output(want) => (select(&names, want.as_deref())?, None),
        Target::Region(region) => (select_region(&state.outputs, *region)?, Some(*region)),
    };
    let selected = &state.outputs[index];
    let output = selected.output.clone();
    let name = names[index].clone();
    let frame = match region {
        Some(region) => {
            if state.manager_version < 2 {
                return Err(
                    "compositor's screencopy manager is too old for region capture (needs v2)"
                        .into(),
                );
            }
            let rect = selected
                .rect()
                .ok_or("compositor does not expose the output's layout position")?;
            manager.capture_output_region(
                0,
                &output,
                region.x - rect.x,
                region.y - rect.y,
                region.width,
                region.height,
                &qh,
                (),
            )
        }
        None => manager.capture_output(0, &output, &qh, ()),
    };
    queue
        .roundtrip(&mut state)
        .map_err(|error| format!("capture setup failed: {error}"))?;
    if let Some(error) = state.error.take() {
        return Err(error);
    }
    let pixels = match state.result.take() {
        Some(result) => result?,
        None => {
            let buffer = state
                .buffer
                .clone()
                .ok_or("compositor sent no buffer parameters")?;
            frame.copy(&buffer);
            connection
                .flush()
                .map_err(|error| format!("cannot send the copy request: {error}"))?;
            loop {
                queue
                    .blocking_dispatch(&mut state)
                    .map_err(|error| format!("wayland dispatch failed: {error}"))?;
                if let Some(result) = state.result.take() {
                    break result?;
                }
            }
        }
    };
    entry(&pixels, &name, path)
}

/// Picks the index of the requested output name, or the first one for `None`.
fn select(names: &[String], want: Option<&str>) -> Result<usize, String> {
    match (
        names
            .iter()
            .position(|name| want.is_none_or(|want| name == want)),
        want,
    ) {
        (Some(index), _) => Ok(index),
        (None, Some(want)) => Err(format!(
            "unknown output {want}; available: {}",
            names.join(", ")
        )),
        (None, None) => Err("compositor has no outputs".into()),
    }
}

/// Picks the output that fully contains the region. wlr screencopy region
/// capture is per output, so a region spanning outputs is rejected.
fn select_region(outputs: &[Output], region: Rect) -> Result<usize, String> {
    outputs
        .iter()
        .position(|output| output.rect().is_some_and(|rect| rect.contains(region)))
        .ok_or_else(|| {
            let available: Vec<String> = outputs
                .iter()
                .filter_map(|output| {
                    output
                        .rect()
                        .map(|rect| format!("{} {}x{}+{}+{}", output.name(), rect.width, rect.height, rect.x, rect.y))
                })
                .collect();
            if available.is_empty() {
                return "compositor does not expose output positions (xdg-output), so a global region cannot be mapped".to_owned();
            }
            format!(
                "region {}x{}+{}+{} is not inside one output; available: {}",
                region.width,
                region.height,
                region.x,
                region.y,
                available.join(", ")
            )
        })
}

/// Encodes and writes the PNG, returning the store/caller entry.
fn entry(pixels: &Pixels, output: &str, path: &Path) -> Result<Value, String> {
    let bytes = encode_png(pixels)?;
    write_atomic_bytes(path, &bytes)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    log::info!(
        target: "screenshot.capture",
        "saved {} ({}x{}, {} bytes) from {output}",
        path.display(),
        pixels.width,
        pixels.height,
        bytes.len(),
    );
    Ok(json!({
        "path": path.display().to_string(),
        "output": output,
        "bytes": bytes.len(),
        "at": now(),
    }))
}

fn encode_png(pixels: &Pixels) -> Result<Vec<u8>, String> {
    let image =
        ImageBuffer::<Rgba<u8>, _>::from_raw(pixels.width, pixels.height, pixels.rgba.clone())
            .ok_or_else(|| format!("invalid frame {}x{}", pixels.width, pixels.height))?;
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| format!("cannot encode PNG: {error}"))?;
    Ok(png.into_inner())
}

/// An unlinked file of `size` bytes; its fd backs the shm pool and lets us
/// read the copied pixels back without mmap. It holds screen contents, so it
/// is created 0600 before the name is removed.
fn shm_file(size: u64) -> Result<File, String> {
    let path = std::env::temp_dir().join(format!(
        "riced-screenshot-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| format!("cannot create the shm file: {error}"))?;
    file.set_len(size)
        .map_err(|error| format!("cannot size the shm file: {error}"))?;
    let _ = fs::remove_file(&path); // the name goes away; the fd stays valid
    Ok(file)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

/// One output as seen from the registry.
struct Output {
    global: u32,
    title: String,
    output: WlOutput,
    // Kept alive for the layout position/size events; never read directly.
    _xdg: Option<ZxdgOutputV1>,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
}

impl Output {
    /// Compositor name, or `output-N` when it predates wl_output name events.
    fn name(&self) -> String {
        if self.title.is_empty() {
            format!("output-{}", self.global)
        } else {
            self.title.clone()
        }
    }

    /// Global logical rectangle, once xdg-output reported it.
    fn rect(&self) -> Option<Rect> {
        let (x, y) = self.position?;
        let (width, height) = self.size?;
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    }
}

/// Buffer parameters announced by the frame's `buffer` event; these size the
/// shm pool (they already account for the output's mode and scale).
struct Layout {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

/// Converted RGBA pixels of one capture.
struct Pixels {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// Wayland state of one capture; lives only on the capture thread.
#[derive(Default)]
struct Shot {
    _registry: Option<WlRegistry>,
    manager: Option<ZwlrScreencopyManagerV1>,
    manager_version: u32,
    xdg_manager: Option<ZxdgOutputManagerV1>,
    shm: Option<WlShm>,
    outputs: Vec<Output>,
    layout: Option<Layout>,
    y_invert: bool,
    buffer: Option<wl_buffer::WlBuffer>,
    file: Option<File>,
    error: Option<String>,
    result: Option<Result<Pixels, String>>,
}

impl Shot {
    /// Reads the copied frame back from shm and converts it to RGBA.
    fn convert(&self) -> Result<Pixels, String> {
        let layout = self
            .layout
            .as_ref()
            .ok_or("compositor sent no buffer parameters")?;
        let file = self.file.as_ref().ok_or("no shm backing file")?;
        let mut shm = vec![0u8; layout.stride as usize * layout.height as usize];
        file.read_exact_at(&mut shm, 0)
            .map_err(|error| format!("cannot read the copied frame: {error}"))?;
        let rgba = to_rgba(
            &shm,
            layout.stride as usize,
            layout.width as usize,
            layout.height as usize,
            layout.format == wl_shm::Format::Argb8888,
            self.y_invert,
        )?;
        Ok(Pixels {
            rgba,
            width: layout.width,
            height: layout.height,
        })
    }
}

/// Converts ARGB8888/XRGB8888 shm bytes (BGRA on little-endian) to RGBA8;
/// XRGB ignores the alpha bits, so those pixels become opaque.
fn to_rgba(
    shm: &[u8],
    stride: usize,
    width: usize,
    height: usize,
    alpha: bool,
    y_invert: bool,
) -> Result<Vec<u8>, String> {
    let row = width * 4;
    if row > stride {
        return Err("frame stride is smaller than one row".into());
    }
    if shm.len() < stride * height {
        return Err("frame buffer is truncated".into());
    }
    let mut rgba = Vec::with_capacity(row * height);
    for y in 0..height {
        let source = (if y_invert { height - 1 - y } else { y }) * stride;
        for pixel in shm[source..source + row].chunks_exact(4) {
            rgba.extend_from_slice(&[
                pixel[2],
                pixel[1],
                pixel[0],
                if alpha { pixel[3] } else { 255 },
            ]);
        }
    }
    Ok(rgba)
}

/// Creates the shm pool and buffer sized to the frame's `buffer` event.
fn attach(
    state: &mut Shot,
    format: WEnum<wl_shm::Format>,
    width: u32,
    height: u32,
    stride: u32,
    qh: &QueueHandle<Shot>,
) -> Result<(), String> {
    let shm = state.shm.clone().ok_or("compositor has no wl_shm")?;
    let format = match format {
        WEnum::Value(format @ (wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888)) => format,
        WEnum::Value(other) => return Err(format!("unsupported shm pixel format {other:?}")),
        WEnum::Unknown(code) => return Err(format!("unsupported shm pixel format {code}")),
    };
    let size = stride
        .checked_mul(height)
        .and_then(|bytes| i32::try_from(bytes).ok())
        .ok_or_else(|| format!("frame {width}x{height} is too large"))?;
    let file = shm_file(size as u64)?;
    let pool = shm.create_pool(file.as_fd(), size, qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        format,
        qh,
        (),
    );
    state.layout = Some(Layout {
        format,
        width,
        height,
        stride,
    });
    state.buffer = Some(buffer);
    state.file = Some(file);
    Ok(())
}

impl Dispatch<WlRegistry, ()> for Shot {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_output" => {
                let output = registry.bind::<WlOutput, _, _>(name, version.min(4), qh, name);
                state.outputs.push(Output {
                    global: name,
                    title: String::new(),
                    output,
                    _xdg: None,
                    position: None,
                    size: None,
                });
            }
            "wl_shm" if state.shm.is_none() => {
                state.shm = Some(registry.bind::<WlShm, _, _>(name, 1, qh, ()));
            }
            "zwlr_screencopy_manager_v1" if state.manager.is_none() => {
                // v2 adds region capture; v3 only adds dmabuf buffer events.
                state.manager_version = version.min(2);
                state.manager = Some(registry.bind::<ZwlrScreencopyManagerV1, _, _>(
                    name,
                    state.manager_version,
                    qh,
                    (),
                ));
            }
            "zxdg_output_manager_v1" if state.xdg_manager.is_none() => {
                state.xdg_manager =
                    Some(registry.bind::<ZxdgOutputManagerV1, _, _>(name, version.min(3), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for Shot {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wl_output::Event::Name { name } = event else {
            return; // mode and scale come with the frame's buffer event
        };
        if let Some(output) = state
            .outputs
            .iter_mut()
            .find(|output| output.global == *global)
        {
            output.title = name;
        }
    }
}

impl Dispatch<ZxdgOutputV1, u32> for Shot {
    fn event(
        state: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state
            .outputs
            .iter_mut()
            .find(|output| output.global == *global)
        else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => output.position = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                output.size = Some((width, height));
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for Shot {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                if let Err(error) = attach(state, format, width, height, stride, qh) {
                    state.error = Some(error);
                }
            }
            zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                state.y_invert = matches!(
                    flags,
                    WEnum::Value(flags) if flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert)
                );
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                state.result = Some(state.convert());
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.result = Some(Err("the compositor failed to copy the frame".into()));
            }
            _ => {}
        }
    }
}

delegate_noop!(Shot: ignore WlShm);
delegate_noop!(Shot: ignore WlShmPool);
delegate_noop!(Shot: ignore wl_buffer::WlBuffer);
delegate_noop!(Shot: ignore ZwlrScreencopyManagerV1);
delegate_noop!(Shot: ignore ZxdgOutputManagerV1);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_argb_xrgb_and_flips_rows() {
        // Two 2-pixel rows (8 bytes each) padded to a 16-byte stride.
        let shm: Vec<u8> = [1, 2, 3, 4, 5, 6, 7, 8]
            .into_iter()
            .chain([0; 8])
            .chain([9, 10, 11, 12, 13, 14, 15, 16])
            .chain([0; 8])
            .collect();
        assert_eq!(
            to_rgba(&shm, 16, 2, 2, true, false).unwrap(),
            [3, 2, 1, 4, 7, 6, 5, 8, 11, 10, 9, 12, 15, 14, 13, 16]
        );
        assert_eq!(
            to_rgba(&shm, 16, 2, 2, true, true).unwrap(),
            [11, 10, 9, 12, 15, 14, 13, 16, 3, 2, 1, 4, 7, 6, 5, 8]
        );
        // XRGB ignores the alpha bits and comes out opaque.
        let opaque = to_rgba(&shm, 16, 2, 2, false, false).unwrap();
        assert_eq!(opaque[3], 255);
        assert_eq!(&opaque[..3], &[3, 2, 1]);
        assert_eq!(&opaque[12..16], &[15, 14, 13, 255]);
        assert_eq!(
            to_rgba(&shm, 7, 2, 2, true, false),
            Err("frame stride is smaller than one row".into())
        );
        assert_eq!(
            to_rgba(&shm[..20], 16, 2, 2, true, false),
            Err("frame buffer is truncated".into())
        );
    }

    #[test]
    fn arguments_validate_output_and_path() {
        let dir = std::env::temp_dir().join(format!("riced-screenshot-dir-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for args in [
            json!([]),
            json!(["DP-1"]),
            json!(["DP-1", "shot.png", 3]),
            json!([7, "shot.png"]),
            json!(["", "shot.png"]),
            json!([null, ""]),
            json!([null, 5]),
            json!([null, dir.to_str().unwrap()]),
        ] {
            assert!(arguments(&args).is_err(), "{args}");
        }
        assert_eq!(
            arguments(&json!([null, "/tmp/riced-screenshot-nothing.png"]))
                .unwrap()
                .0,
            None
        );
        let (output, path) = arguments(&json!(["DP-1", "~/shot.png"])).unwrap();
        assert_eq!(output.as_deref(), Some("DP-1"));
        assert!(path.ends_with("shot.png"));
        let _ = fs::remove_dir(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn shm_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let file = shm_file(16).unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn atomic_write_roundtrips_binary_and_replaces() {
        let path =
            std::env::temp_dir().join(format!("riced-screenshot-bytes-{}.png", std::process::id()));
        let png = [0u8, 137, 80, 78, 71, 13, 10, 26, 10, 255];
        write_atomic_bytes(&path, &png).unwrap();
        assert_eq!(fs::read(&path).unwrap(), png);
        write_atomic_bytes(&path, &[1]).unwrap();
        assert_eq!(fs::read(&path).unwrap(), [1]);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn encodes_and_decodes_png() {
        let pixels = Pixels {
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
            width: 2,
            height: 1,
        };
        let bytes = encode_png(&pixels).unwrap();
        assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 1));
        assert_eq!(decoded.get_pixel(0, 0), &Rgba([1, 2, 3, 255]));
    }

    #[test]
    fn region_arguments_validate_and_reject_bad_rectangles() {
        for args in [
            json!([]),
            json!([0, 0, 10, 10]),
            json!([0, 0, 0, 10, "/tmp/riced-screenshot-nothing.png"]),
            json!([0, 0, 10, -1, "/tmp/riced-screenshot-nothing.png"]),
            json!([0.5, 0, 10, 10, "/tmp/riced-screenshot-nothing.png"]),
            json!([0, 0, 10, 10, ""]),
            json!([0, 0, 10, 10, 7]),
        ] {
            assert!(region_arguments(&args).is_err(), "{args}");
        }
        let (region, path) = region_arguments(&json!([-1920, 40, 800, 600, "~/shot.png"])).unwrap();
        assert_eq!(
            (region.x, region.y, region.width, region.height),
            (-1920, 40, 800, 600)
        );
        assert!(path.ends_with("shot.png"));
    }

    #[test]
    fn rect_contains_handles_edges_and_neighbouring_outputs() {
        let monitor = Rect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        };
        assert!(monitor.contains(Rect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440
        }));
        assert!(monitor.contains(Rect {
            x: 2559,
            y: 1439,
            width: 1,
            height: 1
        }));
        assert!(!monitor.contains(Rect {
            x: 2560,
            y: 0,
            width: 1,
            height: 1
        }));
        assert!(!monitor.contains(Rect {
            x: -1,
            y: 0,
            width: 1,
            height: 1
        }));
        assert!(!monitor.contains(Rect {
            x: 2560,
            y: 0,
            width: 1920,
            height: 1080
        }));
    }

    #[test]
    fn select_matches_names_and_falls_back_to_the_first_output() {
        let names = ["DP-1".to_owned(), "HDMI-A-1".to_owned()];
        assert_eq!(select(&names, None).unwrap(), 0);
        assert_eq!(select(&names, Some("HDMI-A-1")).unwrap(), 1);
        assert_eq!(
            select(&names, Some("unknown")),
            Err("unknown output unknown; available: DP-1, HDMI-A-1".into())
        );
        // Without wl_output name events the fallback name is used.
        let fallback = ["output-7".to_owned()];
        assert_eq!(select(&fallback, Some("output-7")).unwrap(), 0);
    }
}
