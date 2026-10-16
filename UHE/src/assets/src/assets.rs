use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use grind::Jobs;

use crate::error::{Error, Result};
use crate::id::{AssetData, AssetId, AssetKind, Handle};
use crate::image::{decode_image, Image, MipLevel};
use crate::ktx2::{is_ktx2, load_ktx2};
use crate::load_model;
use crate::mesh::{MeshData, ModelData};
use crate::vfs::Vfs;

/// What a finished load reports: the kind that landed, or why it did not.
///
/// A plain `Result` rather than a bespoke enum, because that is how a caller
/// wants to read it — `for (id, result) in assets.poll() { if let Err(error) =
/// result { … } }`.
pub type LoadResult = std::result::Result<AssetKind, Error>;

/// Everything one load produced.
///
/// One type for the synchronous and the worker path, so both reach the registry
/// through the same code and cannot diverge on what a slot holds.
struct Decoded {
    kind: AssetKind,
    bytes: Vec<u8>,
    image: Option<Image>,
    mips: Vec<MipLevel>,
    meshes: Vec<MeshData>,
    model: Option<ModelData>,
}

/// A finished asynchronous load, sent from a worker thread.
struct Finished {
    id: AssetId,
    decoded: Result<Decoded>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadState {
    Loading,
    Ready,
}

/// One asset: what it is, where it came from, and what decoding it produced.
struct Slot {
    /// Bumped on every release, so an id from before the release stops
    /// resolving even when the slot is handed out again.
    generation: u32,
    path: String,
    kind: AssetKind,
    state: LoadState,
    bytes: Vec<u8>,
    image: Option<Image>,
    mips: Vec<MipLevel>,
    meshes: Vec<MeshData>,
    model: Option<ModelData>,
}

impl Slot {
    fn new() -> Self {
        Self {
            // Generations start at 1, so a zeroed or default id is never valid.
            generation: 1,
            path: String::new(),
            kind: AssetKind::Binary,
            state: LoadState::Loading,
            bytes: Vec::new(),
            image: None,
            mips: Vec::new(),
            meshes: Vec::new(),
            model: None,
        }
    }
}

/// The asset registry.
///
/// Port of `AssetManager` (`UHE/src/UHE/AssestsManager/AssetManager.{h,cpp}`),
/// with the parts the C++ needed and this design does not: no `shared_ptr`
/// refcount per asset (the registry owns every payload and a handle is an
/// index), no `finalized` flag (there is no GPU upload step in this crate, so
/// "decoded" is the whole of "ready"), and no mutex (a registry is reached
/// through `&mut Assets`, which is the one owner the spine asks for).
///
/// `IDEA.md:61` is why this exists at all: the C++ was path-based with no
/// registry, so every call re-walked a `path -> ID` map and every handle had to
/// re-check a generation counter on a shared record.
pub struct Assets {
    vfs: Vfs,
    slots: Vec<Slot>,
    /// Indices of released slots, for reuse.
    free: Vec<u32>,
    /// Virtual path -> the live id for it, so a second request shares the first
    /// decode.
    paths: HashMap<String, AssetId>,
    sender: mpsc::Sender<Finished>,
    receiver: mpsc::Receiver<Finished>,
    /// Loads that were already finished when they were asked for, reported by
    /// the next `poll` like any other so a caller waiting on it is not left
    /// waiting forever.
    ready: Vec<(AssetId, LoadResult)>,
    live: usize,
}

impl Assets {
    /// A registry with `root` as its first (and initially only) search root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let mut vfs = Vfs::new();
        vfs.mount(root);
        let (sender, receiver) = mpsc::channel();
        Self {
            vfs,
            slots: Vec::new(),
            free: Vec::new(),
            paths: HashMap::new(),
            sender,
            receiver,
            ready: Vec::new(),
            live: 0,
        }
    }

    /// Appends a search root. Earlier roots win, so a mod directory mounted
    /// ahead of the game directory overrides it.
    pub fn mount(&mut self, dir: impl Into<PathBuf>) {
        self.vfs.mount(dir);
    }

    pub fn roots(&self) -> &[PathBuf] {
        self.vfs.roots()
    }

    /// The virtual file system this registry resolves against.
    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }

    /// Resolves, reads and decodes one asset, returning a ready id.
    ///
    /// A second load of the same path returns the first id rather than decoding
    /// twice; an id whose slot a `load_async` already owns is returned in its
    /// current (possibly not yet decoded) state, which is what the C++ did when
    /// it found a record mid-decode.
    pub fn load(&mut self, path: &str) -> Result<AssetId> {
        if let Some(id) = self.live_id(path) {
            return Ok(id);
        }
        let physical = self.vfs.resolve(path)?;
        let decoded = decode_file(&physical, path)?;
        Ok(self.store(path, decoded))
    }

    /// Submits a load to the job system and returns its id immediately.
    ///
    /// The worker reads the file and decodes it — CPU work only, exactly the
    /// contract `IAssetLoader::LoadFromFile` had in C++ — and hands the result
    /// back through a channel that [`Assets::poll`] drains. A load that fails is
    /// reported once, and its slot is released, so nothing is re-delivered
    /// forever.
    pub fn load_async(&mut self, jobs: &Jobs, path: &str) -> Result<AssetId> {
        if let Some(id) = self.live_id(path) {
            let kind = self.slot(id).map(|slot| slot.kind);
            if let Some(kind) = kind {
                self.ready.push((id, Ok(kind)));
            }
            return Ok(id);
        }

        let physical = self.vfs.resolve(path)?;
        let id = self.allocate(path, classify(path));
        let sender = self.sender.clone();
        let virtual_path = path.to_owned();
        jobs.execute(
            Box::new(move || {
                let decoded = decode_file(&physical, &virtual_path);
                // A closed receiver means the registry is gone; the result has
                // nowhere to go and nothing to clean up.
                let _ = sender.send(Finished { id, decoded });
            }),
            None,
        );
        Ok(id)
    }

    /// Every load that has finished since the last call, in completion order.
    pub fn poll(&mut self) -> Vec<(AssetId, LoadResult)> {
        let mut out: Vec<(AssetId, LoadResult)> = std::mem::take(&mut self.ready);

        while let Ok(finished) = self.receiver.try_recv() {
            let id = finished.id;
            // Looked up without the `Ready` requirement the accessors apply: a
            // loading slot is exactly what a finished job exists to resolve.
            let Some(slot) = self.slots.get(id.index() as usize) else {
                continue;
            };
            if slot.generation != id.generation() {
                // Released since the job was submitted.
                continue;
            }
            if slot.state != LoadState::Loading {
                // A synchronous load already resolved this slot; the worker's
                // answer is stale.
                continue;
            }

            match finished.decoded {
                Ok(decoded) => {
                    let kind = decoded.kind;
                    self.store_at(id, decoded);
                    out.push((id, Ok(kind)));
                }
                Err(error) => {
                    self.release(id);
                    out.push((id, Err(error)));
                }
            }
        }

        out
    }

    /// The encoded file bytes, for every kind. Valid UTF-8 for a text asset is
    /// not a promise this makes; [`Assets::text`] is the accessor for that.
    pub fn bytes(&self, id: AssetId) -> Option<&[u8]> {
        self.slot(id).map(|slot| slot.bytes.as_slice())
    }

    /// The decoded image, which is level 0 of a KTX2 chain.
    pub fn image(&self, id: AssetId) -> Option<&Image> {
        self.slot(id)?.image.as_ref()
    }

    /// The mip levels the file shipped, empty when it shipped none or the
    /// asset is not an image.
    pub fn mips(&self, id: AssetId) -> Option<&[MipLevel]> {
        self.slot(id).map(|slot| slot.mips.as_slice())
    }

    /// The first mesh of a model, which is also the one carrying the model's
    /// animation clips and skeleton.
    pub fn mesh(&self, id: AssetId) -> Option<&MeshData> {
        self.slot(id)?.meshes.first()
    }

    /// Every mesh primitive of a model, in file order.
    pub fn meshes(&self, id: AssetId) -> Option<&[MeshData]> {
        self.slot(id).map(|slot| slot.meshes.as_slice())
    }

    /// A model's node graph, skins and unimplemented-extension list.
    pub fn model(&self, id: AssetId) -> Option<&ModelData> {
        self.slot(id)?.model.as_ref()
    }

    /// The asset as text, or `None` when it is not a text asset or not UTF-8.
    pub fn text(&self, id: AssetId) -> Option<&str> {
        let slot = self.slot(id)?;
        if slot.kind != AssetKind::Text {
            return None;
        }
        std::str::from_utf8(&slot.bytes).ok()
    }

    pub fn kind(&self, id: AssetId) -> Option<AssetKind> {
        self.slot(id).map(|slot| slot.kind)
    }

    /// The virtual path this asset was loaded from.
    pub fn path(&self, id: AssetId) -> Option<&str> {
        self.slot(id).map(|slot| slot.path.as_str())
    }

    /// A typed handle, or `None` when the id names nothing or names a different
    /// payload kind than `T`.
    pub fn handle<T: ?Sized + AssetData>(&self, id: AssetId) -> Option<Handle<T>> {
        let slot = self.slot(id)?;
        T::accepts(slot.kind).then(|| Handle::new(id))
    }

    /// The live id for a virtual path, which is the registry's `name -> handle`
    /// half. `None` for a path that was never loaded or has been released.
    pub fn find(&self, path: &str) -> Option<AssetId> {
        let id = self.paths.get(path).copied()?;
        self.slot(id).map(|_| id)
    }

    /// Frees a slot: the payload is dropped, the path mapping is removed, and
    /// the generation is bumped so every outstanding id for this asset stops
    /// resolving instead of aliasing whatever takes the slot next.
    ///
    /// Returns whether the id was live. A stale id is refused, not ignored.
    pub fn release(&mut self, id: AssetId) -> bool {
        if id.is_null() {
            return false;
        }
        let Some(slot) = self.slots.get_mut(id.index() as usize) else {
            return false;
        };
        if slot.generation != id.generation() {
            return false;
        }

        if self
            .paths
            .get(slot.path.as_str())
            .is_some_and(|current| *current == id)
        {
            let path = slot.path.clone();
            self.paths.remove(&path);
        }

        // Wraps after 2^32 releases of one slot, which no session reaches.
        slot.generation = slot.generation.wrapping_add(1);
        slot.state = LoadState::Loading;
        slot.path.clear();
        slot.kind = AssetKind::Binary;
        slot.bytes = Vec::new();
        slot.image = None;
        slot.mips.clear();
        slot.meshes.clear();
        slot.model = None;

        self.free.push(id.index());
        self.live -= 1;
        true
    }

    /// Whether the id names a decoded asset right now.
    pub fn contains(&self, id: AssetId) -> bool {
        self.slot(id).is_some()
    }

    /// Live assets, not slot count: released slots are kept for reuse.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    fn slot(&self, id: AssetId) -> Option<&Slot> {
        if id.is_null() {
            return None;
        }
        let slot = self.slots.get(id.index() as usize)?;
        (slot.generation == id.generation() && slot.state == LoadState::Ready).then_some(slot)
    }

    fn live_id(&self, path: &str) -> Option<AssetId> {
        let id = self.paths.get(path).copied()?;
        self.slots
            .get(id.index() as usize)
            .filter(|slot| slot.generation == id.generation())
            .map(|_| id)
    }

    fn allocate(&mut self, path: &str, kind: AssetKind) -> AssetId {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                self.slots.push(Slot::new());
                (self.slots.len() - 1) as u32
            }
        };

        // The free list only ever holds indices this table created, so the
        // index is in range.
        let slot = &mut self.slots[index as usize];
        let id = AssetId::from_parts(index, slot.generation);
        slot.path = path.to_owned();
        slot.kind = kind;
        slot.state = LoadState::Loading;

        self.paths.insert(path.to_owned(), id);
        self.live += 1;
        id
    }

    fn store(&mut self, path: &str, decoded: Decoded) -> AssetId {
        match self.live_id(path) {
            Some(id) => {
                self.store_at(id, decoded);
                id
            }
            None => {
                let id = self.allocate(path, decoded.kind);
                self.store_at(id, decoded);
                id
            }
        }
    }

    fn store_at(&mut self, id: AssetId, decoded: Decoded) {
        let Some(slot) = self.slots.get_mut(id.index() as usize) else {
            return;
        };
        if slot.generation != id.generation() {
            return;
        }

        slot.kind = decoded.kind;
        slot.state = LoadState::Ready;
        slot.bytes = decoded.bytes;
        slot.image = decoded.image;
        slot.mips = decoded.mips;
        slot.meshes = decoded.meshes;
        slot.model = decoded.model;
    }
}

/// The kind a virtual path's extension names.
///
/// The C++ chose a loader from the payload type the caller asked for
/// (`Load<TextAsset>`), which cannot be right for a registry keyed by path: the
/// path is the only thing a caller has. Unknown extensions are [`AssetKind::Binary`]
/// rather than an error, because a `.bin` nobody can decode is still worth
/// having the bytes of.
fn classify(path: &str) -> AssetKind {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "tga" | "ktx2" => AssetKind::Image,
        "gltf" | "glb" => AssetKind::Model,
        "txt" | "json" | "md" | "toml" | "xml" | "csv" | "yaml" | "yml" | "unhuman" | "slang"
        | "wgsl" | "glsl" | "vert" | "frag" | "comp" => AssetKind::Text,
        _ => AssetKind::Binary,
    }
}

/// Reads one file and decodes whatever its extension claims it is.
///
/// The single place a load happens, shared by the synchronous path and the job
/// system. A file that is read but does not match its extension is an error,
/// not a silent binary asset: the caller asked for it as the other kind.
fn decode_file(physical: &Path, virtual_path: &str) -> Result<Decoded> {
    let bytes = std::fs::read(physical)?;
    let kind = classify(virtual_path);
    let mut decoded = Decoded {
        kind,
        bytes,
        image: None,
        mips: Vec::new(),
        meshes: Vec::new(),
        model: None,
    };

    match kind {
        AssetKind::Image => {
            if is_ktx2(&decoded.bytes) {
                let ktx = load_ktx2(&decoded.bytes)?;
                decoded.image = Some(Image {
                    width: ktx.width,
                    height: ktx.height,
                    rgba8: ktx.rgba8,
                    srgb: ktx.srgb,
                });
                decoded.mips = ktx.mips;
            } else {
                // A standalone image is colour data unless a caller says
                // otherwise; the linear maps only exist inside a material, where
                // the slot decides.
                decoded.image = Some(decode_image(&decoded.bytes, true)?);
            }
        }
        AssetKind::Model => {
            let loaded = load_model::load(physical, &decoded.bytes)?;
            decoded.meshes = loaded.meshes;
            decoded.model = Some(loaded.model);
        }
        AssetKind::Text | AssetKind::Binary => {}
    }

    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_is_by_extension_and_case_insensitive() {
        assert_eq!(classify("textures/wall.PNG"), AssetKind::Image);
        assert_eq!(classify("models/scene.glb"), AssetKind::Model);
        assert_eq!(classify("shaders/pbr.wgsl"), AssetKind::Text);
        assert_eq!(classify("data/blob"), AssetKind::Binary);
        assert_eq!(classify("data/thing.unknownext"), AssetKind::Binary);
    }

    #[test]
    fn a_missing_path_is_an_error_and_allocates_nothing() {
        let mut assets = Assets::new(".");
        assert!(matches!(
            assets.load("nothing/here.png"),
            Err(Error::NotFound(_))
        ));
        assert!(assets.is_empty());
    }
}
