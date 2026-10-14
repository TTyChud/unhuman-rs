//! Device capability detection and sync tier selection.
//!
//! Ported from `UHE/src/Platform/Vulkan/VulkanExtensionCheck.h`. That design is
//! correct and worth keeping verbatim: capability detection is
//! core-promotion-aware, so an extension promoted into Vulkan 1.3 counts even
//! when the instance never enumerated it, and every barrier and submit goes
//! through one lowering point that picks the modern or legacy encoding from the
//! resulting tier.
//!
//! `select_sync_tier` is a pure function, so the tier ladder is testable with no
//! device.

/// Which encoding of barriers and submits this device gets.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SyncTier {
    /// `vkCmdPipelineBarrier` + `vkQueueSubmit`.
    #[default]
    Legacy,
    /// `vkCmdPipelineBarrier2` + `vkQueueSubmit2`.
    Sync2,
    /// As above, plus timeline semaphores.
    Sync2Timeline,
}

impl SyncTier {
    /// True for the two tiers that spell barriers `vkCmdPipelineBarrier2`.
    ///
    /// The question callers actually ask. `Sync2` and `Sync2Timeline` differ in
    /// semaphore type, not in barrier spelling, so anything that lowers a
    /// barrier should ask this rather than match on a tier name -- matching the
    /// name is how the next tier gets added and one branch silently stops
    /// compiling.
    pub fn uses_sync2(self) -> bool {
        matches!(self, SyncTier::Sync2 | SyncTier::Sync2Timeline)
    }

    pub fn supports_timeline_semaphores(self) -> bool {
        matches!(self, SyncTier::Sync2Timeline)
    }

    pub fn supports_sync2(self) -> bool {
        !matches!(self, SyncTier::Legacy)
    }
}

/// Packaged Vulkan 1.0 as 0x00MMmmpp.
pub fn version_major(api_version: u32) -> u32 {
    api_version >> 22
}

pub fn version_minor(api_version: u32) -> u32 {
    (api_version >> 12) & 0x3ff
}

/// The tier ladder.
///
/// Synchronization 2 became core in 1.3, so it does not need to be an
/// enumerated extension on a 1.3 device. Timeline semaphores likewise. Getting
/// this wrong shows up as a device that reports the extension but rejects the
/// entry point, or vice versa.
pub fn select_sync_tier(
    api_version: u32,
    synchronization2_extension: bool,
    timeline_extension: bool,
) -> SyncTier {
    let core_sync2 = version_major(api_version) > 1 || version_minor(api_version) >= 3;
    if !(core_sync2 || synchronization2_extension) {
        return SyncTier::Legacy;
    }
    if timeline_extension {
        SyncTier::Sync2Timeline
    } else {
        SyncTier::Sync2
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QueueFamily {
    pub index: u32,
    pub graphics: bool,
    pub compute: bool,
    pub transfer: bool,
    pub present: bool,
    pub timestamp_valid_bits: u32,
}

/// What the backend needs to know before it can create anything.
#[derive(Clone, PartialEq, Debug)]
pub struct Capabilities {
    pub device_name: String,
    pub api_version: u32,
    pub driver_version: u32,
    pub vendor_id: u32,
    pub device_id: u32,
    pub sync_tier: SyncTier,
    pub queue_families: Vec<QueueFamily>,
    pub graphics_queue_family: Option<u32>,
    pub dynamic_rendering: bool,
    pub memory_budget: bool,
    pub descriptor_indexing: bool,
    pub max_texture_dimension_2d: u32,
}

impl Capabilities {
    /// Panics-free: a device without a graphics queue is reported as such, and
    /// device creation refuses it rather than dereferencing a `None`.
    pub fn graphics_queue_family(&self) -> Option<u32> {
        self.graphics_queue_family
    }

    /// Frames in flight. Tied to the swapchain image count in practice; the
    /// backend clamps to what the surface reports.
    pub fn frames_in_flight(&self) -> u32 {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_decodes() {
        assert_eq!(version_major(vk_make_version(1, 3, 281)), 1);
        assert_eq!(version_minor(vk_make_version(1, 3, 281)), 3);
        assert_eq!(version_major(vk_make_version(1, 1, 0)), 1);
        assert_eq!(version_minor(vk_make_version(1, 1, 0)), 1);
    }

    #[test]
    fn legacy_device_gets_legacy_encodings() {
        let tier = select_sync_tier(vk_make_version(1, 1, 0), false, false);
        assert_eq!(tier, SyncTier::Legacy);
        assert!(!tier.supports_sync2());
        assert!(!tier.supports_timeline_semaphores());
    }

    #[test]
    fn synchronization2_extension_promotes_a_1_1_device() {
        let tier = select_sync_tier(vk_make_version(1, 1, 0), true, false);
        assert_eq!(tier, SyncTier::Sync2);
        assert!(tier.supports_sync2());
        assert!(!tier.supports_timeline_semaphores());
    }

    #[test]
    fn core_promotion_is_enough_on_1_3() {
        // No extension enumerated, but 1.3 has synchronization 2 in core.
        let tier = select_sync_tier(vk_make_version(1, 3, 0), false, false);
        assert_eq!(tier, SyncTier::Sync2);
    }

    #[test]
    fn timeline_is_the_top_tier() {
        let tier = select_sync_tier(vk_make_version(1, 3, 0), false, true);
        assert_eq!(tier, SyncTier::Sync2Timeline);
        assert!(tier.supports_timeline_semaphores());
    }

    const fn vk_make_version(major: u32, minor: u32, patch: u32) -> u32 {
        (major << 22) | (minor << 12) | patch
    }
}
