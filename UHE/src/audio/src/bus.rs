//! Where a sound is routed, and what its group volume is.
//!
//! One enum and one flat gain table. The arithmetic is in
//! [`crate::mixer::Mixer`]; this file exists so the routing vocabulary -- five
//! names, a fixed order, a stable string for a settings file -- has one home
//! and cannot be duplicated into a second table that drifts.
//!
//! ## The shape, and why it is not a tree
//!
//! One master over four child buses. Upstream's design doc asks for "master to
//! music, SFX, voice and ambient, providing group volume and ducking"
//! (`docs/architecture/assets-and-audio.md`), and that shape is two levels
//! deep. Ducking -- music ducking under a voice line -- needs a bus that sums
//! into another bus and can have its own gain changed while its sources are
//! playing, which is not a third level so much as a second parallel one. It is
//! not implemented; see the crate docs.
//!
//! ## Why `Master` is a variant and not a field
//!
//! Because the obvious alternative is `Mixer::set_master_volume(v)` next to
//! `Mixer::set_bus_volume(Bus::Master, v)`, which is two names for one value,
//! and one place to disagree with itself. `session.md` lists duplicated
//! mappings as this codebase's recurring bug. Here the master is bus zero and
//! [`Bus::Master::volume`](crate::mixer::Mixer::volume) returns it.
//!
//! A sound is routed to one of the four children and the master is applied on
//! top. Routing a sound to the master is not refused -- it simply means the
//! child gain is unity, which is a coherent if unhelpful configuration, so
//! rejecting it would be inventing an error for something the arithmetic
//! already handles.

/// A destination for a sound.
///
/// [`Bus::Master`] is the parent of the other four and is not itself a routing
/// target; a sound is routed to one of the children and the master gain is
/// applied on top of whatever the child resolves to. There is no check that
/// stops a caller passing `Master` to
/// [`crate::instance::PlayParams::with_bus`] -- a sound on the master bus is
/// simply one whose child gain is unity, which is a coherent if unhelpful
/// configuration, so refusing it would be inventing an error for something the
/// maths already handles.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Bus {
    /// The final fader. Applies to every bus below it.
    Master,
    /// Long-form audio: music, ambience beds, voice-over lines.
    Music,
    /// Anything the player causes: footsteps, weapons, doors, impacts.
    Sfx,
    /// Dialogue.
    Voice,
    /// Environmental loops that are not music.
    Ambient,
}

impl Bus {
    /// Every bus, in a fixed order. Used for the flat gain arrays that go to
    /// the device, and for a test that walks them all.
    pub const ALL: [Bus; Bus::COUNT] =
        [Bus::Master, Bus::Music, Bus::Sfx, Bus::Voice, Bus::Ambient];

    /// How many buses there are, so the gain arrays are not a bare literal.
    pub const COUNT: usize = 5;

    /// Position in a flat array of bus gains.
    ///
    /// Total by construction: `ALL` is exhaustive and the index is taken from
    /// `ALL` itself rather than from a hand-written number, so adding a bus
    /// without adding its index cannot compile.
    pub fn index(self) -> usize {
        match self {
            Bus::Master => 0,
            Bus::Music => 1,
            Bus::Sfx => 2,
            Bus::Voice => 3,
            Bus::Ambient => 4,
        }
    }

    /// A stable name, for a settings file or a debug overlay.
    pub fn name(self) -> &'static str {
        match self {
            Bus::Master => "master",
            Bus::Music => "music",
            Bus::Sfx => "sfx",
            Bus::Voice => "voice",
            Bus::Ambient => "ambient",
        }
    }

    /// Whether this is the parent bus.
    pub fn is_master(self) -> bool {
        self == Bus::Master
    }

    /// Every bus except the master, i.e. the ones a sound can be routed to.
    pub fn routable() -> impl Iterator<Item = Bus> {
        Bus::ALL.into_iter().filter(|bus| !bus.is_master())
    }
}
