use std::fmt;
use std::marker::PhantomData;

/// A generational handle to a backend resource.
///
/// Replaces the C++ `void*`-based `BufferHandle`/`TextureHandle`
/// (`RHITypes.h:37-41`), which could only be validated by `reinterpret_cast`
/// round trips. A handle whose slot has been freed and reused fails its
/// generation check instead of silently aliasing a different object.
///
/// Reuses the shape of `RGHandle<T>{index, generation}`
/// (`VulkanRenderGraphTypes.h:40-51`), promoted from render-graph-internal to
/// the whole RHI.
pub struct Handle<T> {
    index: u32,
    generation: u32,
    // `fn() -> T` rather than `T` so a handle is Send + Sync no matter what `T`
    // is. It is a marker, not an owner.
    _tag: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    pub(crate) fn new(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            _tag: PhantomData,
        }
    }

    pub fn index(self) -> u32 {
        self.index
    }

    pub fn generation(self) -> u32 {
        self.generation
    }
}

impl<T> Copy for Handle<T> {}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle({}#{})", self.index, self.generation)
    }
}

/// Typed handles for the resources a backend owns. The newtype is what stops a
/// `Texture` being passed where a `Pipeline` is expected — the class of bug the
/// `void*` handles allowed silently.
pub type Buffer = Handle<BufferTag>;
pub type Texture = Handle<TextureTag>;
pub type ImageView = Handle<ImageViewTag>;
pub type Pipeline = Handle<PipelineTag>;
pub type Shader = Handle<ShaderTag>;
pub type BindGroup = Handle<BindGroupTag>;

#[derive(Debug)]
pub struct BufferTag;
#[derive(Debug)]
pub struct TextureTag;
#[derive(Debug)]
pub struct ImageViewTag;
#[derive(Debug)]
pub struct PipelineTag;
#[derive(Debug)]
pub struct ShaderTag;
#[derive(Debug)]
pub struct BindGroupTag;

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// Slot storage with free-list reuse and generation validation.
///
/// `V` is the stored value, `T` is the handle's resource tag. They are separate
/// because a backend wants `Registry<MyBufferData, BufferTag>` handing out
/// `Buffer` handles, not a registry keyed by the tag itself. `T` defaults to
/// `V` so a plain `Registry<u32>` behaves like a set of `Handle<u32>`.
pub struct Registry<V, T = V> {
    slots: Vec<Slot<V>>,
    free: Vec<u32>,
    live: usize,
    _tag: PhantomData<fn() -> T>,
}

impl<V, T> Registry<V, T> {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
            _tag: PhantomData,
        }
    }

    pub fn create(&mut self, value: V) -> Handle<T> {
        self.live += 1;
        match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.value = Some(value);
                Handle::new(index, slot.generation)
            }
            None => {
                let index = self.slots.len() as u32;
                // Generations start at 1, so a zeroed handle is never valid.
                self.slots.push(Slot {
                    generation: 1,
                    value: Some(value),
                });
                Handle::new(index, 1)
            }
        }
    }

    pub fn get(&self, handle: Handle<T>) -> Option<&V> {
        let slot = self.slots.get(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.value.as_ref()
    }

    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut V> {
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.value.as_mut()
    }

    /// Frees the slot and bumps its generation, so every outstanding handle to
    /// the old value stops resolving.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<V> {
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        // Wraps after 2^32 frees of one slot. A handle surviving that many
        // round trips through a deferred deletion queue is not a real failure
        // mode, so this is left unhandled rather than paid for on every free.
        slot.generation = slot.generation.wrapping_add(1);
        let value = slot.value.take();
        if value.is_some() {
            self.free.push(handle.index);
            self.live -= 1;
        }
        value
    }

    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.get(handle).is_some()
    }

    /// Live entries, not slot count. Freed slots are kept for reuse.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Iterates live `(handle, &value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &V)> {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            slot.value
                .as_ref()
                .map(|v| (Handle::new(i as u32, slot.generation), v))
        })
    }
}

impl<V, T> Default for Registry<V, T> {
    fn default() -> Self {
        Self::new()
    }
}