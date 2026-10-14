//! C3 preview is view state: it never edits a client mesh or document history.
mod camera;
mod gpu;
#[cfg(test)]
mod multipart_tests;
#[cfg(not(target_arch = "wasm32"))]
mod scene_worker;
#[cfg(test)]
mod tests;
#[cfg(not(target_arch = "wasm32"))]
mod worker;
mod workspace;

use crate::{PhotocraftApp, theme::Tokens};
pub use camera::Camera;
use gpu::Image;
use photocraft_doc::{DocId, Document, LayerId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};
pub(crate) use workspace::{arranged_cells, panel, texture_bar, tiled};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub enabled: bool,
    /// Explicit Window-menu activation also shows the pane for new/ordinary images.
    pub show_for_all: bool,
    pub automatic: bool,
    pub client_root: Option<String>,
    pub camera: Camera,
    pub lit: bool,
    /// 0 = smooth alpha; 1 = cutout; 2 = opaque.
    pub alpha: u8,
    pub flip_v: bool,
    /// 0 = -Z (Eudemons), 1 = +Z, 2 = +Y, 3 = -Y.
    pub up_axis: u8,
    /// None = the visible image, Some(id) = one DDS surface/layer.
    pub texture_layer: Option<u64>,
    /// Add the object list to an older customized dock once, preserving its layout.
    pub object_tab_added: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            enabled: true,
            show_for_all: false,
            automatic: true,
            client_root: None,
            camera: Camera::default(),
            lit: false,
            alpha: 0,
            flip_v: false,
            up_axis: 0,
            texture_layer: None,
            object_tab_added: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub path: String,
    pub reason: String,
}
#[derive(Clone)]
pub(super) struct Loaded {
    pub model: Arc<photocraft_c3::Model>,
    pub textures: Vec<Option<Arc<Image>>>,
    pub selected: Vec<bool>,
    pub visible: Vec<bool>,
    pub path: String,
    pub warnings: Vec<String>,
    pub pieces: Vec<Piece>,
    pub assets: Vec<TextureAsset>,
    pub texture_slots: Vec<Option<usize>>,
    pub object_id: Option<String>,
    pub title: String,
}
#[derive(Clone)]
pub(super) struct Piece {
    pub key: String,
    pub path: String,
    pub range: std::ops::Range<usize>,
    pub visible: bool,
    pub note: Option<String>,
    pub textures: Vec<usize>,
}
#[derive(Clone)]
pub(super) struct TextureAsset {
    pub key: String,
    pub path: Option<String>,
    pub bytes: Option<Arc<Vec<u8>>>,
    pub source: bool,
    pub note: Option<String>,
}
pub(super) struct Found {
    pub complete: bool,
    pub candidates: Vec<Candidate>,
    pub loaded: Option<Loaded>,
    pub root: Option<String>,
    pub warnings: Vec<String>,
    pub objects: Vec<(String, String)>,
}
type FoundResult = Result<Found, String>;
struct Discovery {
    id: DocId,
    generation: u64,
    rx: Receiver<FoundResult>,
    cancel: Arc<AtomicBool>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct TextureKey {
    id: DocId,
    revision: u64,
    preview: u64,
    display: u64,
    layer: Option<u64>,
    targa: bool,
}
struct TextureJob {
    key: TextureKey,
    rx: Receiver<Result<Image, String>>,
}
#[derive(Default)]
struct DocPreview {
    source_doc: Option<DocId>,
    source: String,
    root: Option<String>,
    candidates: Vec<Candidate>,
    loaded: Option<Loaded>,
    image: Option<Arc<Image>>,
    key: Option<TextureKey>,
    generation: u64,
    pinned: bool,
    pending_textures: bool,
    camera: Camera,
    layer: Option<u64>,
    status: String,
    warnings: Vec<String>,
    objects: Vec<(String, String)>,
    texture_docs: HashMap<usize, DocId>,
    hidden_textures: std::collections::HashSet<usize>,
    tiled: bool,
    opening_textures: std::collections::HashSet<usize>,
    saved_textures: HashMap<usize, (DocId, u64, Arc<Image>)>,
}
#[derive(Default)]
pub(crate) struct Runtime {
    settings_loaded: bool,
    gpu: Option<gpu::Gpu>,
    docs: HashMap<DocId, DocPreview>,
    active: Option<DocId>,
    gpu_doc: Option<DocId>,
    discovery: Option<Discovery>,
    texture_job: Option<TextureJob>,
    pub rect: Option<egui::Rect>,
    error: Option<String>,
    owners: HashMap<DocId, DocId>,
    uploaded_keys: HashMap<DocId, TextureKey>,
    edited_doc: Option<DocId>,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(d) = &self.discovery {
            d.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl Runtime {
    pub(crate) fn release_gpu(&mut self) {
        self.gpu = None;
        self.gpu_doc = None;
        self.uploaded_keys.clear();
        self.edited_doc = None;
    }
    pub fn set_wgpu(&mut self, rs: eframe::egui_wgpu::RenderState) {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gpu::Gpu::new(rs))) {
            Ok(gpu) => {
                self.gpu = Some(gpu);
                self.error = None;
            }
            Err(_) => self.error = Some("Could not initialize the C3 preview renderer".into()),
        }
    }
    fn stop_discovery(&mut self) {
        if let Some(d) = self.discovery.take() {
            d.cancel.store(true, Ordering::Relaxed);
        }
    }
    fn discover(&mut self, id: DocId, source: String, root: Option<String>, manual: Option<(String, Vec<u8>)>, ctx: &egui::Context) {
        self.discover_choice(id, source, root, manual, None, ctx);
    }
    fn owner(&self, id: DocId) -> DocId {
        self.owners.get(&id).copied().unwrap_or(id)
    }
    fn discover_choice(
        &mut self,
        id: DocId,
        source: String,
        root: Option<String>,
        manual: Option<(String, Vec<u8>)>,
        object: Option<String>,
        ctx: &egui::Context,
    ) {
        self.stop_discovery();
        let source_doc = self.active.filter(|active| self.owner(*active) == id).unwrap_or(id);
        let p = self.docs.entry(id).or_default();
        if p.source != source || p.source_doc.is_none() {
            p.source_doc = Some(source_doc);
        }
        p.source = source.clone();
        p.generation = p.generation.saturating_add(1);
        p.pending_textures = true;
        p.status = if manual.is_some() { "Loading C3 model…" } else { "Finding matching C3 models…" }.into();
        let generation = p.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let cancel = cancel.clone();
            let ctx = ctx.clone();
            let spawn = std::thread::Builder::new().name("c3 discovery".into()).spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker::discover(&source, root.as_deref(), manual, object.as_deref(), cancel.as_ref(), |found| {
                        let _ = tx.send(Ok(found));
                        ctx.request_repaint();
                    })
                }))
                .unwrap_or_else(|_| Err("C3 import failed safely; the document is intact".into()));
                let _ = tx.send(result);
                ctx.request_repaint();
            });
            if let Err(e) = spawn {
                p.status = format!("Cannot start C3 loader: {e}");
                p.pending_textures = false;
                return;
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (source, root, object, cancel.as_ref(), ctx);
            let result = manual.ok_or_else(|| "Select a C3 model with Load mesh; automatic client discovery is available on desktop".into()).and_then(
                |(path, bytes)| {
                    let model = Arc::new(photocraft_c3::decode(&bytes).map_err(|e| e.to_string())?);
                    let count = model.meshes.len();
                    Ok(Found {
                        complete: true,
                        candidates: Vec::new(),
                        root: None,
                        warnings: Vec::new(),
                        objects: Vec::new(),
                        loaded: Some(Loaded {
                            selected: vec![true; count],
                            visible: visibility(&model),
                            textures: (0..count).map(|_| None).collect(),
                            warnings: model.warnings.clone(),
                            model,
                            path,
                            pieces: Vec::new(),
                            assets: Vec::new(),
                            texture_slots: vec![None; count],
                            object_id: None,
                            title: "Selected C3".into(),
                        }),
                    })
                },
            );
            let _ = tx.send(result);
        }
        self.discovery = Some(Discovery { id, generation, rx, cancel });
    }
    fn poll(&mut self) {
        if let Some(d) = &self.discovery {
            let result = match d.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("C3 loader stopped before completing".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let complete = result.as_ref().map_or(true, |found| found.complete);
                if let Some(p) = self.docs.get_mut(&d.id).filter(|p| p.generation == d.generation) {
                    p.pending_textures = !complete;
                    match result {
                        Ok(found) => {
                            p.candidates = found.candidates;
                            p.objects = found.objects;
                            p.root = found.root;
                            p.warnings = found.warnings;
                            p.status = if !complete {
                                "Loading other-part textures…"
                            } else if found.loaded.as_ref().is_some_and(|loaded| loaded.model.meshes.is_empty()) {
                                "INI object found; its C3 geometry is unavailable"
                            } else if found.loaded.is_some() {
                                "Live texture preview"
                            } else {
                                "No matching model found. Right-click to load a C3 mesh."
                            }
                            .into();
                            if let Some(mut loaded) = found.loaded {
                                if p.loaded
                                    .as_ref()
                                    .is_some_and(|old| old.assets.iter().map(|asset| &asset.key).ne(loaded.assets.iter().map(|asset| &asset.key)))
                                {
                                    p.texture_docs.clear();
                                    p.saved_textures.clear();
                                    p.opening_textures.clear();
                                    p.hidden_textures.clear();
                                }
                                if let Some(previous) = &p.loaded {
                                    for piece in &mut loaded.pieces {
                                        if let Some(old) = previous.pieces.iter().find(|old| old.key == piece.key) {
                                            piece.visible = old.visible;
                                        }
                                    }
                                }
                                for (slot, texture) in loaded.texture_slots.iter().zip(&mut loaded.textures) {
                                    if let Some((_, _, saved)) = slot.and_then(|slot| p.saved_textures.get(&slot)) {
                                        *texture = Some(saved.clone());
                                    }
                                }
                                p.warnings.extend(loaded.warnings.iter().cloned());
                                let same_model = p.loaded.as_ref().is_some_and(|old| Arc::ptr_eq(&old.model, &loaded.model));
                                let same_parts = p.loaded.as_ref().is_some_and(|old| {
                                    old.path == loaded.path
                                        && old.model.meshes.len() == loaded.model.meshes.len()
                                        && old.model.meshes.iter().zip(&loaded.model.meshes).all(|(a, b)| a.name == b.name)
                                });
                                if same_parts && let Some(previous) = &p.loaded {
                                    loaded.selected.clone_from(&previous.selected);
                                    loaded.visible.clone_from(&previous.visible);
                                }
                                if same_model && self.gpu_doc == Some(d.id) {
                                    if let Some(gpu) = &mut self.gpu {
                                        gpu.secondary_textures(&loaded.textures);
                                    }
                                } else {
                                    self.gpu_doc = None;
                                }
                                p.loaded = Some(loaded);
                            }
                        }
                        Err(e) => p.status = e,
                    }
                }
                if complete {
                    self.discovery = None;
                }
            }
        }
        if let Some(job) = &self.texture_job {
            let result = match job.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("Texture preview worker stopped".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                if let Some(p) = self.docs.get_mut(&job.key.id) {
                    match result {
                        Ok(image) => {
                            p.image = Some(Arc::new(image));
                            p.key = Some(job.key);
                        }
                        Err(e) => {
                            p.key = Some(job.key);
                            p.status = e;
                        }
                    }
                }
                let uploaded_id = job.key.id;
                workspace::upload_document(self, uploaded_id);
                self.texture_job = None;
            }
        }
    }
}

pub(super) fn marker(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "v_head"
            | "v_hair"
            | "v_l_weapon"
            | "v_r_weapon"
            | "v_mount"
            | "v_l_hand"
            | "v_r_hand"
            | "v_l_shield"
            | "v_r_shield"
            | "v_mantle"
            | "v_back"
            | "v_foot"
            | "v_camera"
            | "v_l_arm"
            | "v_r_arm"
            | "v_l_leg"
            | "v_r_leg"
            | "v_pet"
    )
}
pub(super) fn visibility(model: &photocraft_c3::Model) -> Vec<bool> {
    let sockets: Vec<_> = model
        .meshes
        .iter()
        .map(|m| {
            m.vertices.len() <= 32
                && m.indices.len() <= 96
                && (marker(&m.name)
                    || m.name.to_ascii_lowercase().starts_with("v_extend")
                    || m.name.eq_ignore_ascii_case("v_armet")
                    || m.name.eq_ignore_ascii_case("v_misc"))
        })
        .collect();
    let geometry = model.meshes.iter().zip(&sockets).any(|(m, s)| !m.indices.is_empty() && !*s);
    sockets.into_iter().map(|s| !s || !geometry).collect()
}

fn texture_image(doc: &Document, layer: Option<u64>, display: Option<&photocraft_engine::display_color::CanvasDisplay>, targa: bool) -> Result<Image, String> {
    if doc.size.width == 0 || doc.size.height == 0 {
        return Err("The texture has zero size".into());
    }
    let isolated;
    let doc = if let Some(id) = layer {
        let layer = doc.layer(LayerId(id)).ok_or_else(|| "The selected texture layer was removed; choose Visible image".to_string())?;
        isolated = {
            let mut d = doc.clone();
            if let Some(source) = &doc.metadata.dds
                && let Some(index) = source.layers.iter().position(|candidate| *candidate == LayerId(id))
                && let Ok((_, _, surfaces)) = photocraft_codecs::dds::inspect(&source.bytes)
                && let (Some(info), Some(base)) = (surfaces.get(index), surfaces.first())
            {
                d.size = photocraft_geom::Size::new(
                    (u64::from(info.width) * u64::from(doc.size.width) / u64::from(base.width.max(1))).max(1) as u32,
                    (u64::from(info.height) * u64::from(doc.size.height) / u64::from(base.height.max(1))).max(1) as u32,
                );
            }
            let mut l = layer.clone();
            l.visible = true;
            d.layers = vec![l];
            d
        };
        &isolated
    } else {
        doc
    };
    let scale = (2048.0 / doc.size.width.max(doc.size.height) as f64).min(1.0);
    let size = [(doc.size.width as f64 * scale).round().max(1.) as u32, (doc.size.height as f64 * scale).round().max(1.) as u32];
    let targa_alpha = if targa && !matches!(doc.mode, photocraft_doc::ColorMode::Grayscale | photocraft_doc::ColorMode::Bitmap) {
        let mut channels = doc.channels.iter().filter(|channel| channel.spot.is_none());
        channels.next().filter(|_| channels.next().is_none())
    } else {
        None
    };
    let mut buf = photocraft_compose::render_reduced(doc, size[0], size[1]);
    if targa_alpha.is_some() {
        // With an explicit Targa alpha channel, export mattes layer transparency
        // over white before attaching that channel. Composite in float precision.
        for [r, g, b, a] in &mut buf.px {
            let white = 1. - *a;
            *r += white;
            *g += white;
            *b += white;
            *a = 1.;
        }
    }
    let mut image = match display {
        Some(d) => d.to_rgba8(&buf),
        None => buf.to_rgba8(),
    };
    // Targa opens its alpha as a separate channel. Match Targa export without
    // changing the document, its layers, or the colours stored under alpha zero.
    if let Some(channel) = targa_alpha {
        for (i, pixel) in image.pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = ((i as u32 % size[0]) as f64 + 0.5) * f64::from(doc.size.width) / f64::from(size[0]) - 0.5;
            let y = ((i as u32 / size[0]) as f64 + 0.5) * f64::from(doc.size.height) / f64::from(size[1]) - 0.5;
            let x = x.clamp(0., f64::from(doc.size.width - 1));
            let y = y.clamp(0., f64::from(doc.size.height - 1));
            let (x0, y0) = (x.floor() as i32, y.floor() as i32);
            let (x1, y1) = ((x0 + 1).min(doc.size.width as i32 - 1), (y0 + 1).min(doc.size.height as i32 - 1));
            let (fx, fy) = ((x - f64::from(x0)) as f32, (y - f64::from(y0)) as f32);
            let top = channel.surface.sample_channel(x0, y0, 0) * (1. - fx) + channel.surface.sample_channel(x1, y0, 0) * fx;
            let bottom = channel.surface.sample_channel(x0, y1, 0) * (1. - fx) + channel.surface.sample_channel(x1, y1, 0) * fx;
            if let Some(alpha) = pixel.last_mut() {
                *alpha = ((top * (1. - fy) + bottom * fy).clamp(0., 1.) * 255.).round() as u8;
            }
        }
    }
    Ok(Image { size, pixels: image.pixels })
}

fn upkeep(app: &mut PhotocraftApp, ctx: &egui::Context) {
    workspace::sync_documents(app);
    let open: Vec<_> = app.session.documents().iter().map(|d| d.doc.id).collect();
    app.c3_preview.docs.retain(|id, p| open.contains(id) || p.texture_docs.values().any(|id| open.contains(id)));
    if app.c3_preview.discovery.as_ref().is_some_and(|d| !app.c3_preview.docs.contains_key(&d.id)) {
        app.c3_preview.stop_discovery();
    }
    let active = app.session.active().map(|d| (d.doc.id, d.path.clone().unwrap_or_else(|| d.doc.name.clone())));
    let Some((id, source)) = active else {
        app.c3_preview.active = None;
        app.c3_preview.gpu_doc = None;
        if let Some(gpu) = &mut app.c3_preview.gpu {
            gpu.clear();
        }
        return;
    };
    let owner = app.c3_preview.owner(id);
    if app.c3_preview.active != Some(id) {
        if let Some(previous) = app.c3_preview.active.and_then(|id| app.c3_preview.docs.get_mut(&id)) {
            previous.layer = app.ui.c3_preview.texture_layer;
        }
        let previous_owner = app.c3_preview.active.map(|id| app.c3_preview.owner(id));
        if let Some(previous) = previous_owner.and_then(|id| app.c3_preview.docs.get_mut(&id)) {
            previous.camera = app.ui.c3_preview.camera.clone();
        }
        let camera = app.c3_preview.docs.get(&owner).map(|preview| preview.camera.clone()).unwrap_or_default();
        let first = !app.c3_preview.docs.contains_key(&id);
        let p = app.c3_preview.docs.entry(id).or_default();
        if previous_owner != Some(owner) {
            app.ui.c3_preview.camera = camera;
        }
        app.ui.c3_preview.texture_layer = p.layer;
        if p.source.is_empty() {
            p.source = source.clone();
        }
        app.c3_preview.active = Some(id);
        if app.c3_preview.gpu_doc != Some(owner) {
            app.c3_preview.gpu_doc = None;
        }
        if first
            && owner == id
            && app.ui.c3_preview.automatic
            && (app.session.active().is_some_and(|d| d.path.is_some()) || app.ui.c3_preview.client_root.is_some())
        {
            app.c3_preview.discover(id, source, app.ui.c3_preview.client_root.clone(), None, ctx);
        } else if app.c3_preview.docs.get(&owner).is_some_and(|p| p.pending_textures) && !app.c3_preview.discovery.as_ref().is_some_and(|d| d.id == owner) {
            // Switching tabs cancels the old worker. Resume its selected model
            // when returning so a partial preview can finish its other textures.
            let p = app.c3_preview.docs.get(&owner);
            let mesh = p.and_then(|p| p.loaded.as_ref()).map(|m| (m.path.clone(), Vec::new()));
            let object = p.and_then(|p| p.loaded.as_ref()).and_then(|loaded| loaded.object_id.clone());
            let source = p.map(|preview| preview.source.clone()).unwrap_or(source);
            let root = p.and_then(|p| p.root.clone()).or_else(|| app.ui.c3_preview.client_root.clone());
            if mesh.is_some() || app.ui.c3_preview.automatic {
                let mesh = if object.is_some() { None } else { mesh };
                app.c3_preview.discover_choice(owner, source, root, mesh, object, ctx);
            }
        }
    }
    app.c3_preview.poll();
    if app.c3_preview.gpu_doc != Some(owner)
        && let Some(p) = app.c3_preview.docs.get(&owner)
        && let (Some(gpu), Some(loaded)) = (&mut app.c3_preview.gpu, &p.loaded)
    {
        gpu.model(loaded.model.clone(), &loaded.textures);
        app.c3_preview.uploaded_keys.clear();
        app.c3_preview.edited_doc = None;
        if let Some(image) = &p.image {
            gpu.upload(image);
        }
        app.c3_preview.gpu_doc = Some(owner);
        let linked: Vec<_> = p.texture_docs.values().copied().collect();
        for id in linked {
            workspace::upload_document(&mut app.c3_preview, id);
        }
    }
    workspace::upload_document(&mut app.c3_preview, id);
    let has_model = app.c3_preview.docs.get(&owner).is_some_and(|p| p.loaded.is_some());
    if !has_model || app.c3_preview.texture_job.is_some() {
        return;
    }
    let Some(index) = app.session.active_index() else {
        return;
    };
    let (doc, preview) = crate::canvas::display_doc(app, index);
    let Some(st) = app.session.active() else {
        return;
    };
    let revision = st.revision;
    let (display, display_key) = crate::canvas::canvas_display(app, &doc, None);
    let is_targa = |path: &str| photocraft_codecs::from_extension(path) == Some(photocraft_codecs::Format::Tga);
    let targa = st.path.as_deref().is_some_and(is_targa)
        || is_targa(&doc.name)
        || app.c3_preview.docs.get(&owner).is_some_and(|scene| {
            scene.texture_docs.iter().any(|(slot, document)| {
                *document == id && scene.loaded.as_ref().and_then(|loaded| loaded.assets.get(*slot)).is_some_and(|asset| is_targa(&asset.key))
            })
        });
    let key = TextureKey { id, revision, preview, display: display_key, layer: app.ui.c3_preview.texture_layer, targa };
    if app.c3_preview.docs.get(&id).is_some_and(|p| p.key == Some(key)) {
        return;
    }
    let (tx, rx) = mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ctx = ctx.clone();
        if let Err(e) = std::thread::Builder::new().name("c3 live texture".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| texture_image(&doc, key.layer, display.as_deref(), key.targa)))
                .unwrap_or_else(|_| Err("Texture preview failed safely".into()));
            let _ = tx.send(result);
            ctx.request_repaint();
        }) {
            if let Some(p) = app.c3_preview.docs.get_mut(&id) {
                p.status = e.to_string();
                p.key = Some(key);
            }
            return;
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = tx.send(texture_image(&doc, key.layer, display.as_deref(), key.targa));
    }
    app.c3_preview.texture_job = Some(TextureJob { key, rx });
}

pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let preference_id = egui::Id::new("soulcraft-c3-preview-settings");
    if !app.c3_preview.settings_loaded {
        if let Some(mut saved) = ui.ctx().data_mut(|d| d.get_persisted::<State>(preference_id)) {
            saved.camera = Camera::default();
            saved.texture_layer = None;
            saved.alpha = saved.alpha.min(2);
            saved.up_axis = saved.up_axis.min(3);
            app.ui.c3_preview = saved;
        }
        app.c3_preview.settings_loaded = true;
    }
    persist(app, ui.ctx());
    if !app.ui.c3_preview.enabled {
        // The object workspace remains usable when only the 3D pane is hidden.
        workspace::sync_documents(app);
        app.c3_preview.poll();
        if workspace::scene_id(app).and_then(|id| app.c3_preview.docs.get(&id)).is_some_and(|preview| preview.loaded.is_some()) {
            let ctx = ui.ctx().clone();
            upkeep(app, &ctx);
        }
        app.c3_preview.rect = None;
        return;
    }
    let ctx = ui.ctx().clone();
    upkeep(app, &ctx);
    if app.c3_preview.discovery.is_some() || app.c3_preview.texture_job.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
    if !visible(app) {
        app.c3_preview.rect = None;
        return;
    }
    let t = Tokens::get(&ctx);
    egui::Panel::right("c3-preview-panel")
        .resizable(true)
        .default_size(350.)
        .size_range(260.0..=700.0)
        .frame(egui::Frame::NONE.fill(t.dock).inner_margin(egui::Margin::same(8)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("C3 Texture Preview");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Hide").clicked() {
                        app.ui.c3_preview.enabled = false;
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(app.session.active().is_some(), egui::Button::new("Load mesh…")).clicked() {
                    let result = pick_mesh(app, &ctx);
                    report(app, result);
                }
                if ui.button("Client folder…").on_hover_text("Select the client containing c3 and ini folders, or an unpacked asset folder").clicked() {
                    let result = pick_root(app, &ctx);
                    report(app, result);
                }
                if app.c3_preview.discovery.is_some() && ui.small_button("Cancel").clicked() {
                    app.c3_preview.stop_discovery();
                    if let Some(p) = app.session.active().map(|d| app.c3_preview.owner(d.doc.id)).and_then(|id| app.c3_preview.docs.get_mut(&id)) {
                        p.status = "Model lookup cancelled".into();
                        p.pending_textures = false;
                    }
                }
            });
            let id = workspace::scene_id(app);
            if let Some(p) = id.and_then(|id| app.c3_preview.docs.get(&id)) {
                let status = p.status.clone();
                let warnings = p.warnings.clone();
                let path = p.loaded.as_ref().map(|l| l.path.clone());
                if let Some(path) = path {
                    ui.label(egui::RichText::new(crate::file_open::display_name(&path)).strong()).on_hover_text(path);
                }
                ui.label(egui::RichText::new(status).small());
                if !warnings.is_empty() {
                    ui.collapsing("Preview notes", |ui| {
                        egui::ScrollArea::vertical().id_salt("c3-notes-scroll").max_height(120.).show(ui, |ui| {
                            for w in &warnings {
                                ui.label(w);
                            }
                        });
                    });
                }
                let choices: Vec<_> = p.candidates.iter().map(|c| (c.path.clone(), c.reason.clone())).collect();
                if choices.len() > 1 {
                    let mut chosen = None;
                    egui::ComboBox::from_id_salt("c3 candidates")
                        .width(ui.available_width() - 12.)
                        .selected_text(format!("{} matching meshes", choices.len()))
                        .show_ui(ui, |ui| {
                            for (path, reason) in choices {
                                if ui.selectable_label(false, &path).on_hover_text(reason).clicked() {
                                    chosen = Some(path);
                                }
                            }
                        });
                    if let Some(path) = chosen {
                        let result = load_candidate(app, &path, &ctx);
                        report(app, result);
                    }
                }
            } else {
                ui.label("Open a texture to preview it on a C3 model.");
            }
            ui.horizontal_wrapped(|ui| {
                for (label, view) in [("Front", "front"), ("Back", "back"), ("Side", "right"), ("Top", "top")] {
                    if ui.small_button(label).clicked() {
                        app.ui.c3_preview.camera.preset(view);
                    }
                }
                if ui.small_button("Fit").clicked() {
                    app.ui.c3_preview.camera.reset();
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut app.ui.c3_preview.lit, "Lighting");
                ui.checkbox(&mut app.ui.c3_preview.flip_v, "Flip V");
                egui::ComboBox::from_id_salt("c3 alpha")
                    .width(90.)
                    .selected_text(match app.ui.c3_preview.alpha {
                        1 => "Cutout",
                        2 => "Opaque",
                        _ => "Alpha",
                    })
                    .show_ui(ui, |ui| {
                        for (i, label) in [(0, "Alpha"), (1, "Cutout"), (2, "Opaque")] {
                            ui.selectable_value(&mut app.ui.c3_preview.alpha, i, label);
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Up axis:");
                egui::ComboBox::from_id_salt("c3-up-axis")
                    .width(150.)
                    .selected_text(match app.ui.c3_preview.up_axis {
                        1 => "+Z",
                        2 => "+Y",
                        3 => "-Y",
                        _ => "-Z (Eudemons)",
                    })
                    .show_ui(ui, |ui| {
                        for (value, label) in [(0, "-Z (Eudemons)"), (1, "+Z"), (2, "+Y"), (3, "-Y")] {
                            ui.selectable_value(&mut app.ui.c3_preview.up_axis, value, label);
                        }
                    });
            });
            if let Some(st) = app.session.active() {
                let current =
                    app.ui.c3_preview.texture_layer.and_then(|id| st.doc.layer(LayerId(id))).map(|l| l.name.clone()).unwrap_or_else(|| "Visible image".into());
                egui::ComboBox::from_id_salt("c3 texture source").width(ui.available_width() - 12.).selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.ui.c3_preview.texture_layer, None, "Visible image");
                    for layer in st.doc.layers.iter().filter(|l| matches!(l.content, photocraft_doc::LayerContent::Raster(_))) {
                        ui.selectable_value(&mut app.ui.c3_preview.texture_layer, Some(layer.id.0), &layer.name);
                    }
                });
            }
            if let Some(loaded) = id.and_then(|id| app.c3_preview.docs.get_mut(&id)).and_then(|p| p.loaded.as_mut()) {
                ui.collapsing("Mesh parts", |ui| {
                    let linked = loaded.object_id.is_some();
                    ui.label(if linked {
                        "Textures follow their INI bindings. File visibility is in C3 Preview."
                    } else {
                        "Eye = visible; Override = use the active texture on this part."
                    });
                    egui::ScrollArea::vertical().id_salt("c3-parts-scroll").max_height(180.).show(ui, |ui| {
                        for ((mesh, visible), selected) in loaded.model.meshes.iter().zip(&mut loaded.visible).zip(&mut loaded.selected) {
                            if mesh.indices.is_empty() {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                ui.checkbox(visible, "Eye");
                                if !linked {
                                    ui.checkbox(selected, "Override").on_hover_text(
                                        "Override this part's assigned material with the active texture. Linked materials keep their live edits.",
                                    );
                                }
                                let width = ui.available_width();
                                crate::widgets::elided_label(ui, &mesh.name, width).on_hover_text(&mesh.texture);
                            });
                        }
                    });
                });
            }
            let height = (ui.available_height() - 40.).max(100.);
            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click_and_drag());
            app.c3_preview.rect = Some(rect);
            ui.painter().rect_filled(rect, 4., t.canvas);
            let painter = ui.painter().with_clip_rect(rect);
            let grid = 32.;
            let mut x = rect.left();
            while x < rect.right() {
                painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], egui::Stroke::new(0.5, t.separator));
                x += grid;
            }
            let mut y = rect.top();
            while y < rect.bottom() {
                painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], egui::Stroke::new(0.5, t.separator));
                y += grid;
            }
            if response.dragged_by(egui::PointerButton::Primary) || response.dragged_by(egui::PointerButton::Middle) {
                let (pos, delta, shift) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.delta(), i.modifiers.shift));
                if response.dragged_by(egui::PointerButton::Middle) || shift {
                    app.ui.c3_preview.camera.move_by(delta, rect.size());
                } else if let Some(pos) = pos {
                    app.ui.c3_preview.camera.orbit(pos - delta, pos, rect);
                }
                ui.ctx().request_repaint();
            }
            if response.hovered() {
                let delta = ui.input(|i| i.smooth_scroll_delta.y);
                if delta != 0. {
                    app.ui.c3_preview.camera.zoom(delta);
                    ui.ctx().request_repaint();
                }
            }
            if response.double_clicked() {
                app.ui.c3_preview.camera.reset();
            }
            let state = &app.ui.c3_preview;
            let mut rendered = false;
            if app.gpu_active()
                && let Some(loaded) = id.and_then(|id| app.c3_preview.docs.get(&id)).and_then(|p| p.loaded.as_ref())
                && let Some(gpu) = app.c3_preview.gpu.as_mut()
            {
                let size = [(rect.width() * ctx.pixels_per_point()).ceil() as u32, (rect.height() * ctx.pixels_per_point()).ceil() as u32];
                if let Some(texture) = gpu.draw(size, state, &loaded.selected, &workspace::effective_visibility(loaded)) {
                    painter.image(texture, rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)), egui::Color32::WHITE);
                    rendered = true;
                }
            }
            if !rendered {
                let text = app.c3_preview.error.as_deref().unwrap_or(if app.c3_preview.gpu.is_none() {
                    "3D preview requires the GPU renderer"
                } else {
                    "Right-click to load a C3 mesh"
                });
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(12.), t.text);
            }
            response.context_menu(|ui| {
                if ui.button("Load different C3 mesh…").clicked() {
                    let result = pick_mesh(app, &ctx);
                    report(app, result);
                    ui.close();
                }
                if ui.button("Select client folder…").clicked() {
                    let result = pick_root(app, &ctx);
                    report(app, result);
                    ui.close();
                }
                if ui.button("Find matching model again").clicked() {
                    let result = rematch(app, &ctx);
                    report(app, result);
                    ui.close();
                }
                ui.separator();
                if ui.button("Reset rotation, zoom and pan").clicked() {
                    app.ui.c3_preview.camera.reset();
                    ui.close();
                }
                ui.checkbox(&mut app.ui.c3_preview.automatic, "Automatically match new textures");
            });
            ui.label(egui::RichText::new("Drag: rotate · Wheel: zoom\nShift-drag / middle drag: pan · Right-click: mesh").small());
        });
    persist(app, ui.ctx());
}
fn persist(app: &PhotocraftApp, ctx: &egui::Context) {
    let mut saved = app.ui.c3_preview.clone();
    saved.camera = Camera::default();
    saved.texture_layer = None;
    ctx.data_mut(|d| d.insert_persisted(egui::Id::new("soulcraft-c3-preview-settings"), saved));
}
pub(crate) fn visible(app: &PhotocraftApp) -> bool {
    app.ui.c3_preview.enabled
        && (app.ui.c3_preview.show_for_all
            || app.session.active().is_some_and(|d| {
                d.doc.metadata.dds.is_some()
                    || photocraft_codecs::from_extension(d.path.as_deref().unwrap_or(&d.doc.name)) == Some(photocraft_codecs::Format::Tga)
                    || app.c3_preview.docs.get(&app.c3_preview.owner(d.doc.id)).is_some_and(|p| p.loaded.is_some())
            }))
}
fn report(app: &mut PhotocraftApp, result: Result<Value, String>) {
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}
fn pick_mesh(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<Value, String> {
    let id = workspace::scene_id(app).ok_or_else(|| "Open a texture first".to_string())?;
    let (texture_id, source) = app
        .session
        .active()
        .map(|state| (state.doc.id, state.path.clone().unwrap_or_else(|| state.doc.name.clone())))
        .ok_or_else(|| "Open a texture first".to_string())?;
    let ctx = ctx.clone();
    let initial_dir = app.ui.c3_preview.client_root.clone().or_else(|| crate::file_dialog::last_used_dir(&app.ui.recent_files));
    app.ask_file(crate::file_dialog::FileDialogRequest::Open { multiple: false, initial_dir, extensions: Some(vec!["c3".into()]) }, move |app, answer| {
        let (path, bytes) = match answer {
            crate::file_dialog::FileDialogAnswer::Paths(paths) => (paths.into_iter().next().ok_or_else(|| "No mesh was selected".to_string())?, Vec::new()),
            crate::file_dialog::FileDialogAnswer::Contents(path, bytes) => (path, bytes),
            _ => return Err("Select one C3 mesh file".into()),
        };
        if bytes.len() > photocraft_c3::MAX_FILE_BYTES {
            return Err("C3 exceeds the 128 MiB preview limit".into());
        }
        if !app.session.documents().iter().any(|d| d.doc.id == texture_id) {
            return Err("The texture document was closed".into());
        }
        if let Some(p) = app.c3_preview.docs.get_mut(&id) {
            p.pinned = true;
        }
        app.c3_preview.discover(id, source, app.ui.c3_preview.client_root.clone(), Some((path, bytes)), &ctx);
        if let Some(preview) = app.c3_preview.docs.get_mut(&id) {
            preview.source_doc = Some(texture_id);
        }
        Ok(json!({"loading":true}))
    })
}
fn pick_root(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<Value, String> {
    let ctx = ctx.clone();
    app.ask_folder(move |app, root| {
        app.ui.c3_preview.client_root = Some(root);
        if app.session.active().is_some() { match_client(app, &ctx) } else { Ok(json!({"clientFolder":app.ui.c3_preview.client_root})) }
    })
}
fn rematch(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<Value, String> {
    app.c3_preview.stop_discovery();
    #[cfg(not(target_arch = "wasm32"))]
    worker::clear_cache();
    match_client(app, ctx)
}
fn match_client(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<Value, String> {
    let st = app.session.active().ok_or_else(|| "Open a texture first".to_string())?;
    let id = app.c3_preview.owner(st.doc.id);
    let source_doc = st.doc.id;
    // An explicit client selection must not be overridden by the old file's
    // ancestor client. Automatic document opens still use their full path.
    let source = if app.ui.c3_preview.client_root.is_some() { st.doc.name.clone() } else { st.path.clone().unwrap_or_else(|| st.doc.name.clone()) };
    let p = app.c3_preview.docs.entry(id).or_default();
    p.pinned = false;
    app.c3_preview.discover(id, source, app.ui.c3_preview.client_root.clone(), None, ctx);
    if let Some(preview) = app.c3_preview.docs.get_mut(&id) {
        preview.source_doc = Some(source_doc);
    }
    Ok(json!({"loading":true}))
}
fn load_candidate(app: &mut PhotocraftApp, path: &str, ctx: &egui::Context) -> Result<Value, String> {
    let id = workspace::scene_id(app).ok_or_else(|| "Open a texture first".to_string())?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (source_doc, source) = app
            .session
            .active()
            .map(|state| (state.doc.id, state.path.clone().unwrap_or_else(|| state.doc.name.clone())))
            .ok_or_else(|| "Open a texture first".to_string())?;
        let p = app.c3_preview.docs.get_mut(&id).ok_or_else(|| "Preview has not initialized".to_string())?;
        let root = p.root.clone().or_else(|| app.ui.c3_preview.client_root.clone());
        // A selected candidate remains client-relative; the background worker
        // reads it through the same bounded client authority as discovery.
        p.pinned = true;
        app.c3_preview.discover(id, source, root, Some((path.into(), Vec::new())), ctx);
        if let Some(preview) = app.c3_preview.docs.get_mut(&id) {
            preview.source_doc = Some(source_doc);
        }
        Ok(json!({"loading":true}))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (id, path, ctx);
        Err("Load a mesh file instead".into())
    }
}

pub(crate) fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if let Some(result) = workspace::command(app, ctx, id, params) {
        return Some(result);
    }
    Some(match id {
        "window.toggle.c3Preview" => {
            app.ui.c3_preview.enabled = !visible(app);
            app.ui.c3_preview.show_for_all = true;
            Ok(json!(app.ui.c3_preview.enabled))
        }
        "preview.c3.load" => {
            if let Some(path) = params.get("path").and_then(Value::as_str) {
                (|| {
                    let st = app.session.active().ok_or_else(|| "Open a texture first".to_string())?;
                    let id = app.c3_preview.owner(st.doc.id);
                    let source_doc = st.doc.id;
                    let source = st.path.clone().unwrap_or_else(|| st.doc.name.clone());
                    let read = app.services.automation_read.as_mut().ok_or_else(|| "Automation file reads are not configured".to_string())?;
                    let (_, bytes) = read(path)?;
                    if bytes.len() > photocraft_c3::MAX_FILE_BYTES {
                        return Err("C3 exceeds the preview file limit".into());
                    }
                    app.c3_preview.discover(id, source, None, Some((path.into(), bytes)), ctx);
                    if let Some(preview) = app.c3_preview.docs.get_mut(&id) {
                        preview.source_doc = Some(source_doc);
                    }
                    app.ui.c3_preview.enabled = true;
                    Ok(json!({"loading":true}))
                })()
            } else if params.get("path").is_some() {
                Err("path must be a string".into())
            } else {
                pick_mesh(app, ctx)
            }
        }
        "preview.c3.rematch" => rematch(app, ctx),
        "preview.c3.clientFolder" => pick_root(app, ctx),
        "preview.c3.reset" => {
            app.ui.c3_preview.camera.reset();
            Ok(Value::Null)
        }
        "preview.c3.view" => {
            if let Some(view) = params.get("view").and_then(Value::as_str) {
                if app.ui.c3_preview.camera.preset(view) { Ok(Value::Null) } else { Err("view must be front, back, left, right, top, or bottom".into()) }
            } else {
                Err("view must be a string".into())
            }
        }
        "preview.c3.settings" => settings(app, params),
        "preview.c3.part" => (|| {
            let index = params
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|i| usize::try_from(i).ok())
                .ok_or_else(|| "index must be a mesh part index".to_string())?;
            let visible = boolean(params, "visible")?;
            let edited = boolean(params, "edited")?;
            let loaded = app
                .session
                .active()
                .map(|d| app.c3_preview.owner(d.doc.id))
                .and_then(|id| app.c3_preview.docs.get_mut(&id))
                .and_then(|p| p.loaded.as_mut())
                .ok_or_else(|| "Load a C3 model first".to_string())?;
            if index >= loaded.model.meshes.len() {
                return Err("mesh part index is out of range".into());
            }
            if let (Some(value), Some(slot)) = (visible, loaded.visible.get_mut(index)) {
                *slot = value;
            }
            if let (Some(value), Some(slot)) = (edited, loaded.selected.get_mut(index)) {
                *slot = value;
            }
            Ok(Value::Null)
        })(),
        _ => return None,
    })
}
fn boolean(p: &Value, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| format!("{key} must be a boolean")),
    }
}
fn settings(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let object = params.as_object().ok_or_else(|| "preview settings must be an object".to_string())?;
    if let Some(key) = object
        .keys()
        .find(|k| !matches!(k.as_str(), "enabled" | "automatic" | "lit" | "flipV" | "alpha" | "rotation" | "distance" | "pan" | "textureLayer" | "upAxis"))
    {
        return Err(format!("unknown C3 preview setting: {key}"));
    }
    let mut state = app.ui.c3_preview.clone();
    if params.get("enabled") == Some(&Value::Bool(true)) {
        state.show_for_all = true;
    }
    if let Some(v) = params.get("upAxis") {
        state.up_axis = match v.as_str() {
            Some("-z") => 0,
            Some("+z") => 1,
            Some("+y") => 2,
            Some("-y") => 3,
            _ => return Err("upAxis must be -z, +z, +y, or -y".into()),
        };
    }
    for (key, slot) in [("enabled", &mut state.enabled), ("automatic", &mut state.automatic), ("lit", &mut state.lit), ("flipV", &mut state.flip_v)] {
        if let Some(value) = boolean(params, key)? {
            *slot = value;
        }
    }
    if let Some(v) = params.get("alpha") {
        let mode = v.as_u64().filter(|n| *n <= 2).ok_or_else(|| "alpha must be 0 (smooth), 1 (cutout), or 2 (opaque)".to_string())?;
        state.alpha = mode as u8;
    }
    if let Some(v) = params.get("distance") {
        let distance = v.as_f64().filter(|n| n.is_finite() && (1.15..=30.).contains(n)).ok_or_else(|| "distance must be from 1.15 to 30".to_string())?;
        state.camera.distance = distance as f32;
    }
    if let Some(v) = params.get("rotation") {
        let q: [f32; 4] = serde_json::from_value(v.clone()).map_err(|_| "rotation must be four finite quaternion values".to_string())?;
        let length = q.iter().map(|v| v * v).sum::<f32>().sqrt();
        if !length.is_finite() || length < 1.0e-8 {
            return Err("rotation must be a nonzero finite quaternion".into());
        }
        state.camera.rotation = q.map(|v| v / length);
    }
    if let Some(v) = params.get("pan") {
        let pan: [f32; 2] = serde_json::from_value(v.clone()).map_err(|_| "pan must be two values".to_string())?;
        if pan.iter().any(|n| !n.is_finite() || n.abs() > 30.) {
            return Err("pan must be finite and within ±30".into());
        }
        state.camera.pan = pan;
    }
    if let Some(v) = params.get("textureLayer") {
        state.texture_layer = if v.is_null() {
            None
        } else {
            Some(
                v.as_u64()
                    .filter(|id| app.session.active().is_some_and(|d| d.doc.layer(LayerId(*id)).is_some()))
                    .ok_or_else(|| "textureLayer must identify a layer in the active document".to_string())?,
            )
        };
    }
    app.ui.c3_preview = state;
    Ok(json!(app.ui.c3_preview))
}
pub(crate) fn inspect(app: &PhotocraftApp) -> Value {
    let p = workspace::scene_id(app).and_then(|id| app.c3_preview.docs.get(&id));
    let active = app.session.active().and_then(|state| app.c3_preview.docs.get(&state.doc.id));
    let loaded = p.and_then(|preview| preview.loaded.as_ref());
    let visibility = loaded.map(workspace::effective_visibility).unwrap_or_default();
    json!({
        "state":app.ui.c3_preview,"rect":app.c3_preview.rect.map(|r|[r.min.x,r.min.y,r.max.x,r.max.y]),
        "loading":app.c3_preview.discovery.is_some(),"textureUpdating":app.c3_preview.texture_job.is_some(),
        "status":p.map(|p|p.status.as_str()),"root":p.and_then(|p|p.root.as_deref()),"path":loaded.map(|l|l.path.as_str()),
        "objectId":loaded.and_then(|loaded|loaded.object_id.as_deref()),"objects":p.map(|p|&p.objects),"tiled":p.is_some_and(|p|p.tiled),
        "candidates":p.map(|p|p.candidates.iter().map(|c|json!({"path":c.path,"reason":c.reason})).collect::<Vec<_>>()),
        "pieces":loaded.map(|loaded|loaded.pieces.iter().enumerate().map(|(i,piece)|json!({"index":i,"key":piece.key,"path":piece.path,"visible":piece.visible,"meshes":piece.range.len(),"note":piece.note,"textures":piece.textures})).collect::<Vec<_>>()),
        "textures":loaded.map(|loaded|loaded.assets.iter().enumerate().map(|(index,asset)|json!({"index":index,"key":asset.key,"path":asset.path,"ready":asset.bytes.is_some(),"note":asset.note,"shown":p.is_none_or(|p|!p.hidden_textures.contains(&index)),"document":p.and_then(|p|p.texture_docs.get(&index)).map(|id|id.0)})).collect::<Vec<_>>()),
        "parts":loaded.map(|l|l.model.meshes.iter().zip(&l.selected).enumerate().map(|(i,(m,s))|json!({"name":m.name,"variant":m.variant,"texture":m.texture,"vertices":m.vertices.len(),"triangles":m.indices.len()/3,"edited":s,"visible":visibility.get(i)})).collect::<Vec<_>>()),
        "warnings":p.map(|p|&p.warnings),"textureRevision":active.and_then(|p|p.key).map(|k|json!({"revision":k.revision,"preview":k.preview})),"textureSize":active.and_then(|p|p.image.as_ref()).map(|i|i.size)
    })
}
