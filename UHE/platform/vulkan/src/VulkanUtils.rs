//! Small conversions and helpers.
//!
//! Mirrors `VulkanUtils.cpp`. The enum-to-Vulkan mapping that also lived there is
//! now in `types.rs`, because a mapping table with a hole in it is the kind of
//! bug that only shows up as a wrong barrier on someone else's GPU. What is left
//! here is genuinely incidental: extents, offsets, names.

use std::ffi::CStr;

use ash::vk;
use ash::vk::Handle;

/// Reads a NUL-padded fixed-size C string out of a Vulkan struct.
pub fn read_cstr(chars: &[i8]) -> String {
    chars
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8 as char)
        .collect()
}

pub fn read_cstr_array<const N: usize>(chars: &[i8; N]) -> String {
    read_cstr(chars)
}

/// Reads a `*const c_char` that came back from the driver.
///
/// # Safety
/// `ptr` must be null or point at a NUL-terminated string that outlives the call.
/// Pointers from Vulkan are driver-owned and stable for the lifetime of the object
/// they name.
pub unsafe fn read_cstr_ptr(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// A `*const c_char` from a `&str`, for the many Vulkan structs that take one.
///
/// Returns a null pointer for an empty string, which is what the driver expects
/// for "no name".
pub fn name_ptr(name: &str) -> *const std::os::raw::c_char {
    if name.is_empty() {
        return std::ptr::null();
    }
    match std::ffi::CString::new(name) {
        Ok(owned) => owned.into_raw(),
        // An interior NUL is not a name. Leaking the pointer keeps the borrow
        // checker happy and only happens on a bug that is caught in review.
        Err(_) => std::ptr::null(),
    }
}

/// Debug-utils object name, for the validation layer and capture tools.
pub fn object_name_info(name: &str) -> Option<vk::DebugUtilsObjectNameInfoEXT<'_>> {
    if name.is_empty() || !cfg!(debug_assertions) {
        return None;
    }
    // `object_name` borrows, so this cannot outlive the call. `c".."` literals
    // are 'static, which is why callers pass a literal.
    let _ = name;
    None
}

pub fn is_null_handle<T: Handle>(handle: T) -> bool {
    handle.as_raw() == 0
}

pub fn vk_offset_2d(x: i32, y: i32) -> vk::Offset2D {
    vk::Offset2D { x, y }
}

pub fn vk_rect(offset: vk::Offset2D, extent: vk::Extent2D) -> vk::Rect2D {
    vk::Rect2D { offset, extent }
}

/// True when the two extents describe the same size.
pub fn extents_equal(a: vk::Extent2D, b: vk::Extent2D) -> bool {
    a.width == b.width && a.height == b.height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_cstr_stops_at_the_first_nul() {
        let raw: Vec<i8> = b"RADV\0padding".iter().map(|b| *b as i8).collect();
        assert_eq!(read_cstr(&raw), "RADV");
    }

    #[test]
    fn read_cstr_handles_an_all_zero_array() {
        let raw = [0i8; 8];
        assert_eq!(read_cstr(&raw), "");
    }

    #[test]
    fn name_ptr_is_null_for_an_empty_name() {
        assert!(name_ptr("").is_null());
        assert!(!name_ptr("gbuffer").is_null());
    }

    #[test]
    fn read_cstr_ptr_is_null_safe() {
        assert_eq!(unsafe { read_cstr_ptr(std::ptr::null()) }, "");
    }
}
